# Deploying the app to Vercel (the judges' clickable link)

The program is already live on devnet (`At2vd5Mqd9xcHRFAZin1VHb6upEsF1GCXoFG19UHsQoj`).
This puts the UI on a public URL pointed at it. ~10 minutes, free tier is enough.

**Two paths to get there:**

- **Dashboard + GitHub** (§1–3): needs the GitHub account that owns the repo
  to install the Vercel GitHub App — after the ownership transfer that's
  `napatsakornnut`.
- **CLI, no GitHub** (used for the live site): the repo owner after the
  transfer can't be required for hosting, so the site was deployed straight
  from a local checkout:

  ```sh
  npx vercel login              # once — any Vercel account
  cd app && npx vercel link --yes --project=stocksie-th
  # add the env vars from the table below:
  #   printf '%s' "<value>" | npx vercel env add <KEY> production
  npx vercel --prod             # → https://stocksie-th.vercel.app
  ```

  CLI deploys are standalone (no git integration); re-run `npx vercel --prod`
  to ship updates.

## 1. Import the repo

1. <https://vercel.com/new> → import **napatsakornnut/stocksie** (GitHub).
2. **Root Directory: `app`** (the Next.js app lives there — Vercel auto-detects Next.js).
3. Framework preset: Next.js (default). Node 20+ default is fine.

## 2. Environment variables (set BEFORE the first build)

`NEXT_PUBLIC_*` values are inlined into the client bundle at build time —
adding them later requires a redeploy.

| Key | Value |
|---|---|
| `NEXT_PUBLIC_RPC_ENDPOINT` | `https://api.devnet.solana.com` |
| `NEXT_PUBLIC_RPC_WS_ENDPOINT` | `wss://api.devnet.solana.com` |
| `NEXT_PUBLIC_PROGRAM_ID` | `At2vd5Mqd9xcHRFAZin1VHb6upEsF1GCXoFG19UHsQoj` |
| `NEXT_PUBLIC_RPC_COMMITMENT` | `confirmed` |
| `AI_API_KEY` | your Gemini key — **Secret**, never `NEXT_PUBLIC_*` |
| `AI_BASE_URL` | `https://generativelanguage.googleapis.com/v1beta/openai` |
| `AI_VISION_MODEL` | `gemini-3.5-flash-lite` |
| `AI_TEXT_MODEL` | `gemini-3.5-flash-lite` |

Deploy → get `https://<project>.vercel.app`.

## 3. Smoke test the deployment

- `/agent` loads; connect Phantom/Solflare with a devnet-funded wallet.
- Chat “ซื้อนม 2 ขวด งบ 50 บาท” → drafts appear → submit → receipt.
- `curl -X POST https://<project>.vercel.app/api/ai -H 'Content-Type: application/json' -d '{"mode":"chat","text":"hi"}'`
  returns AI JSON (proves the server-side key works).

## Notes

- **Reflex policy gate on Vercel**: serverless functions can't run the local
  loopback binary, so household-rule badges won't appear on the hosted URL —
  by design the app behaves identically without them. The full three-tier
  demo (badges included) runs locally per `docs/DEPLOY_DEVNET.md` §5.
- **Rate limits**: free Gemini tier + free devnet RPC are fine for judging
  traffic; the demo wallet used on camera should stay local (surfpool) so a
  devnet hiccup can't derail the recording.
- Demo-data toggle on your machine: `app/.env.local` switches between
  `http://127.0.0.1:8899` (surfpool, has the demo household) and devnet.
