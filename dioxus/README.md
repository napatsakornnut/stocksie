# Stocksie RIIR prototype — Dioxus/WASM household reader

A first step toward rewriting the Stocksie frontend in Rust ("RIIR"), using
[Dioxus](https://dioxuslabs.com) compiled to WebAssembly. The production
frontend remains `app/` (Next.js) — **this crate is additive**, a working
proof that the on-chain client logic runs in pure Rust with zero JS client
code.

## What works today (v0.2)

- Derive the household PDA client-side in Rust
  (`Pubkey::find_program_address`, `solana-pubkey`)
- Fetch the account over JSON-RPC (`gloo-net`, WASM)
- Hand-decode the Anchor borsh layout (offsets documented in `src/main.rs`)
- Read view: vault balance, owner, member count, request counter, rewards
  total, creation slot, and the blake3 name fingerprint (privacy-by-design,
  shown as hex)
- **Purchase-request ledger (v0.2)**: `getProgramAccounts` with an Anchor
  discriminator memcmp filter (`sha256("account:PurchaseRequest")[..8]`,
  base58-encoded via `bs58`) + household filter @8, borsh-decoded to a
  sortable table — cross-verified against a raw RPC probe (id, amount,
  status tag, slot all match)
- **Wallet Standard signing (v0.3)**: detects `window.phantom.solana` /
  `window.solana` via js-sys, builds a SPL Memo v1 wire transaction in pure
  Rust (hand-rolled shortvec + message layout, zeroed signature slot —
  golden-tested byte-for-byte against `@solana/web3.js serializeMessage`,
  see `src/tx.rs` tests), and hands it to
  `signAndSendTransaction({transaction})`; the wallet signs and submits.
  Verified with an injected mock provider (captured wire bytes =
  byte-identical to a web3.js reconstruction, 209 bytes incl. live devnet
  blockhash) — try it with real Phantom in your Chrome.
- **AI agent UI (v0.4)**: chat box calling the existing `/api/ai` route —
  the route gained origin-gated CORS (`AI_ALLOWED_ORIGINS`, default empty =
  same-origin only, so the hosted key stays private). Both intents verified
  live through the Dioxus UI: grounded budget answers (numbers from the
  household/ledger loaded above) and Thai purchase proposals rendered as
  item chips.
- **Household actions (v0.5/v0.6)**: the full lifecycle — initialize /
  deposit / add_member / open request / approve / **confirm_restock (buyer
  signs) / reimburse_buyer (owner pays)** — each built as an Anchor
  instruction (discriminators from the IDL, blake3 text hashes, PDA
  derivation) and compiled through the shared wire builder. **Proven against
  the real program**: a native E2E test (`tests` in `src/main.rs`) signs
  with throwaway ed25519 keys (owner AND buyer) and executes the whole
  lifecycle on localnet — asserting decoded on-chain state after every step
  (member count, vault mirror, request counter, status tags, reimbursed
  amount). Three wire bugs the E2E caught and killed: program index must be
  remapped after the key sort; keys must sort signers → writable → readonly
  (the runtime reads writability by position); a buyer must be added as a
  member first (add_member) and cannot approve their own request.
  Household-PDA derivation always goes through the ADMIN address, never the
  signing member's — a bug class the E2E surfaced twice.

## Run it

```sh
rustup target add wasm32-unknown-unknown   # once
cargo install dioxus-cli                   # once
cd dioxus
dx serve --web                             # dev server
dx build --web                             # static output in dist/
```

Two demo owners are pre-wired (the devnet demo household + the deploy wallet,
which has no household — the error path).

## The RIIR frontier (deliberately not yet)

| Piece | Why it's the hard part |
|---|---|
| **AI agent UI** | The `/api/ai` route is server-side; a Dioxus client can call it as-is. |
| **Styling parity** | Tailwind classes can't carry over 1:1; the prototype uses inline styles. |

Signing is now proven — the remaining RIIR work is breadth, not a blocked
mechanism: rebuild the household *actions* (init/deposit/request/approve)
on the same wire-builder pattern, then the agent page.

If the RIIR continues, the suggested slice order is: request ledger →
wallet-signing interop → agent page. Keep `app/` as the production surface
until signing works end-to-end here.
