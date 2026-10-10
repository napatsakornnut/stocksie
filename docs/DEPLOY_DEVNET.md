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

Congested devnet (hackathon weeks) can reject `solana program deploy`'s write
burst with `Max retries exceeded`, stranding a half-written buffer each try.
The paced, resumable writer in `scripts/deploy-devnet-buffer.js` walks the
same on-chain wire under the public RPC rate ceiling:

```sh
# create + write the whole ELF at ~4 sends in flight, with 429 backoff
node scripts/deploy-devnet-buffer.js target/deploy/stocksie.so

# …or resume a known buffer, writing only chunks that differ
node scripts/deploy-devnet-buffer.js target/deploy/stocksie.so <BUFFER_PUBKEY>

# finalize (single tx — the CLI sees a complete buffer and just deploys it)
solana program deploy --url devnet --buffer <BUFFER_PUBKEY> \
  --program-id target/deploy/stocksie-keypair.json \
  --with-compute-unit-price 100000 target/deploy/stocksie.so
```

Recovered that way on 2026-10-10: buffer `95Te…tXMC` → program
`At2vd…Qoj` live on devnet (slot 509421543).

Wire notes for anyone touching the script (all verified the hard way):
`UpgradeableLoaderInstruction` is bincode-encoded — **Write is variant 1**
with a u32 offset + u64 length prefix + bytes; **variant 4 is SetAuthority**,
so a wrong index can silently revoke the buffer's authority and brick it.
Buffer account layout: 4-byte tag + 1-byte COption flag + 32-byte authority,
then program bytes at offset 37.

Then the IDL (explorer interface metadata — optional, retryable):

```sh
anchor idl init --filepath target/idl/stocksie.json --provider.cluster devnet \
  --priority-fee 100000 At2vd5Mqd9xcHRFAZin1VHb6upEsF1GCXoFG19UHsQoj
```

Verify:

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
