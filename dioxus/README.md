# Stocksie RIIR prototype — Dioxus/WASM household reader

A first step toward rewriting the Stocksie frontend in Rust ("RIIR"), using
[Dioxus](https://dioxuslabs.com) compiled to WebAssembly. The production
frontend remains `app/` (Next.js) — **this crate is additive**, a working
proof that the on-chain client logic runs in pure Rust with zero JS client
code.

## What works today (v0.1)

- Derive the household PDA client-side in Rust
  (`Pubkey::find_program_address`, `solana-pubkey`)
- Fetch the account over JSON-RPC (`gloo-net`, WASM)
- Hand-decode the Anchor borsh layout (offsets documented in `src/main.rs`)
- Read view: vault balance, owner, member count, request counter, rewards
  total, creation slot, and the blake3 name fingerprint (privacy-by-design,
  shown as hex)

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

## The RIIR frontier (deliberately not in v0.1)

| Piece | Why it's the hard part |
|---|---|
| **Wallet Standard signing** | Phantom/Solflare inject JS providers on `window`; signing from Rust means web-sys bindings to the injected provider (`signTransaction` in → serialized tx out). Read-only views don't need it; anything money-moving does. |
| **Purchase-request ledger** | `getProgramAccounts` + memcmp filters + borsh decode — mechanical, just volume. |
| **AI agent UI** | The `/api/ai` route is server-side; a Dioxus client can call it as-is. |
| **Styling parity** | Tailwind classes can't carry over 1:1; the prototype uses inline styles. |

If the RIIR continues, the suggested slice order is: request ledger →
wallet-signing interop → agent page. Keep `app/` as the production surface
until signing works end-to-end here.
