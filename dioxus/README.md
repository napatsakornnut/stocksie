# Stocksie RIIR — Dioxus/WASM household app (โหมดบ้าน)

The Stocksie household app rewritten in Rust ("RIIR") with
[Dioxus](https://dioxuslabs.com) compiled to WebAssembly — **zero JavaScript
client code** (the only JS shipped is the dioxus/wasm-bindgen runtime glue).
Live at **https://stocksie-dioxus.pages.dev** (Cloudflare Pages + a Pages
Function AI proxy).

> The Next.js frontend (`app/`, https://stocksie-th.vercel.app) remains the
> original production surface — this crate is the RIIR line that has since
> grown past it for everyday users.

## What's inside

- **โหมดบ้าน (home mode)** — Thai-first 3-tab UI for everyday users:
  - 🏠 **หน้าแรก** — baht-first hero (live SOL→THB), member chips,
    role-gated approval cards with a 4-step lifecycle bar
    (`ขอ → อนุมัติ → ซื้อ → จ่ายคืน`), baht-first deposit, invite +
    **one-tap member confirmation link** (`/?owner=…&add=<pubkey>`)
  - 📋 **รายการ** — filter chips with counts + totals row
  - 🤖 **ผู้ช่วย** — chat that greets itself from chain data, quick-reply
    chips, Thai receipt scan → item chips → real `create_purchase_request`
  - ⚙️ **โหมดขั้นสูง** keeps the full prototype (all 13/13 mutating
    instructions of the Anchor program callable from Rust)
- **No-wallet demo** — "ลองดูบ้านตัวอย่างก่อน" renders a real devnet
  household read-only; connecting a wallet and the join flow need nothing
  else
- **Wallet Standard interop** (`src/wallet.rs`) — Phantom via js-sys;
  transactions are hand-compiled in Rust (`src/tx.rs` — 3-way key sort,
  program-index remap, golden-tested byte-for-byte vs web3.js)
- **Full program surface** (`src/programs.rs`) — every instruction from the
  IDL: verbatim discriminators, borsh args, blake3 text hashes, PDA
  derivation; the E2E test executes 13 transactions on localnet
- **PWA** — installable (manifest + icons + service worker; assets cached
  immutable, navigation network-first, `/api/*` never cached). The deploy
  script injects manifest/SEO/font tags into the dx-generated index.html
- **AI proxy** (`functions/api/ai.js`) — same-origin Cloudflare Pages
  Function: receipt + chat modes, Gemini via an OpenAI-compatible endpoint,
  key server-side only, with per-IP rate limit, same-origin enforcement and
  payload caps
- **Theming** — system/light/dark with manual override, Thai font
  (Noto Sans Thai) and safe-area padding for installed iPhone use

## Run it locally

```sh
rustup target add wasm32-unknown-unknown   # once
cargo install dioxus-cli                   # once (~/.cargo/bin/dx)
cd dioxus
dx build --web --release                   # static output
# serve the output dir on http://localhost:8190 and open it —
# localhost AI calls fall back to the stocksie-th deployment
```

Notes: run `dx` from this crate dir (workspace root fails), clear
`RUSTC_WRAPPER=""` when sccache is installed, release build only for
production (debug bakes in a dev banner).

## Deploy (one command)

```sh
dioxus/scripts/deploy-pages.sh
```

builds release → stages `/tmp/dx-pages` (site + `functions/` + `_headers` +
PWA files + injected tags) → `wrangler pages deploy` → prints a verify
checklist. See `../docs/DEPLOY_DIOXUS.md` for the full runbook (secrets,
first-time setup, the Yarn-PnP-under-$HOME gotcha, "Uploaded 0 files"
dedupe trap, and how to check which AI URL the served wasm uses).

## Native E2E (localnet)

`cargo test` in this crate (with a localnet validator running) signs with
throwaway ed25519 keys and executes the whole lifecycle on-chain — init →
deposit → members → requests → approve/reject/confirm/reimburse → decline/
close/withdraw/award/remove/set_role — asserting decoded state after every
step (13 transactions, rent reclaim asserted).

## Architecture in one breath

`main.rs` (app + wire-level RPC + borsh decoders) · `programs.rs` (IDL →
instruction builders) · `tx.rs` (legacy wire-format transaction compiler) ·
`wallet.rs` (Wallet Standard interop) · `functions/api/ai.js` (Pages
Function) · `pwa/` (manifest + service worker + icons) ·
`scripts/deploy-pages.sh` (build → stage → deploy).
