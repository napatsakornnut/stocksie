# Deploying Stocksie to devnet (judge-clickable demo)

Everything here is verified working on localnet (surfpool). Devnet uses the
same commands with `--provider.cluster devnet`.

## 0. Prerequisites

- Anchor CLI (`anchor --version`), Solana CLI (`solana --version`)
- The deploy wallet: `~/.config/solana/id.json` (this is also the program's
  **upgrade authority** — keep the file safe)
- Built artifacts: `anchor build` (produces `target/deploy/stocksie.so`)

## 1. Fund the deploy wallet

A 387 KB program needs **≈2.70 SOL rent**, and the deploy temporarily holds a
second copy in a buffer account — budget **~5.5 SOL** to be safe.

```sh
solana airdrop 5 --url devnet   # rate-limited per IP; retry after a few minutes
solana balance --url devnet     # want >= 5.5 SOL
```

If the CLI faucet is rate-limited, the web faucet is
<https://faucet.solana.com> (same limiter — wait for the window to reset).

## 2. Deploy (program + IDL)

```sh
anchor deploy --provider.cluster devnet
```

This deploys the `.so` **and** the IDL (so judges can inspect the interface
via Anchor explorers). Verify:

```sh
solana program show At2vd5Mqd9xcHRFAZin1VHb6upEsF1GCXoFG19UHsQoj --url devnet
```

## 3. Point the app at devnet

`app/.env.local` (all `NEXT_PUBLIC_*`, so they reach the browser):

```bash
NEXT_PUBLIC_RPC_ENDPOINT=https://api.devnet.solana.com
NEXT_PUBLIC_RPC_WS_ENDPOINT=wss://api.devnet.solana.com
```

`npm run dev` (or `npm run build && npm start`). Toggle back to the local
surfpool stack with `http://127.0.0.1:8899` / `ws://127.0.0.1:8900`.

## 4. Demo data on devnet

- The connected wallet must hold devnet SOL (faucet above) — household
  creation, deposits, and requests all need rent + fees.
- The **Local Keypair (dev)** built-in wallet is per-machine; on devnet it
  needs its own airdrop once: `solana airdrop 5 <its address> --url devnet`.
- AI features additionally need `AI_API_KEY` in the server env (see
  `app/.env.example`) — free-tier Gemini works.

## 5. Hosting for the submission link

Any Node host works (the app is a standard Next.js 15 build). Required env
vars: the four `NEXT_PUBLIC_*` above + `AI_API_KEY` (+ optional
`AI_BASE_URL` / `AI_VISION_MODEL` / `AI_TEXT_MODEL`) as **server-side
secrets** — the key must never be `NEXT_PUBLIC_*`.

Optional: run the Reflex policy gate alongside for the household-rules badges
(`RIIR_REFLEX_CORPUS=corpus/household reflex`) — optional by design; without
it the app behaves identically.

## Localnet quick reference (surfpool)

```sh
surfpool start                                # RPC 8899, WS 8900
anchor deploy --provider.cluster localnet
npm run dev                                   # app on :3000
RIIR_REFLEX_CORPUS=corpus/household reflex    # policy gate, loopback 7331
```

Known gotcha: `app/public/receipt-test.html` renders a sample Thai receipt —
screenshot it to exercise the /agent receipt pipeline without paper.
