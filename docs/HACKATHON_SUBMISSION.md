# Stocksie — Colosseum World's Fair / Superteam Thailand AI × Solana submission

> Deadline: **Oct 12, 2026**. Submit on BOTH the Colosseum platform and
> https://superteam.fun/earn/listing/ai-solana-track/ (Thailand track requires
> Colosseum registration with Thailand selected as country).
>
> One-liner: **"AI proposes, the household approves on-chain."**
> Stocksie turns a shared household pantry and budget into a permissioned
> Solana vault, and its AI agent turns everyday life — a receipt photo, a
> sentence in Thai — into purchase requests the family approves together.

## Problem

Household shopping is a coordination problem, and every family already feels
it: someone notices the milk is out (usually not the person who's at the
store), someone else approves the spend, someone buys it, and someone pays the
buyer back — often via screenshots and bank-transfer IOUs that nobody audits.
The money side is worse than the pantry side: shared budgets have no shared
ledger, reimbursement is trust-based, and kids learn nothing about how a
budget actually works.

Existing tools split the problem in half. Chat groups coordinate but hold no
money. Fintech apps hold money but are single-user. Nothing gives a family a
shared treasury with *permissioned* spending — and nothing makes the
paperwork disappear.

## User

Primary: **Thai families and shared households** — a parent managing a home
budget with kids' allowances, siblings sharing a rental, or a household staff
arrangement. They already coordinate purchases in LINE groups today; Stocksie
meets them there with the same mental model (someone requests, someone
approves) but with real, auditable money.

Secondary: any small trust group with a shared budget — dorm floors, student
clubs, community kitchens. The role system (Owner / Parent / Child / Guest)
maps onto all of them.

## Product

Stocksie (https://github.com/napatsakornnut/stocksie) is a working app today:

- **Shared vault** — the household's SOL lives in a program-owned PDA, not in
  anyone's wallet. Deposit once, everyone spends against the same balance.
- **Purchase lifecycle** — request → approve/reject → restock → reimburse,
  enforced on-chain. Amounts are circuit-broken at 0.5 SOL per request.
- **Privacy by design** — item names, prices, and reject reasons are blake3
  hashed. The chain proves the flow happened; it never learns what the family
  buys.
- **Budget dashboard** — one page for balance, approvals, reimbursements, and
  the full purchase ledger, live-polled from the chain.
- **Rewards** — kids earn points for reporting low stock, completing restocks,
  and beating best-price benchmarks; gamification that teaches budgeting.

## AI

The AI household agent (shipped in this repo) creates leverage exactly where
the friction was: **data entry and initiative.**

1. **Receipt → reimbursement, zero typing.** Photograph the receipt after a
   shopping run; a vision model extracts every purchasable line and drafts
   the purchase requests pre-filled with amounts converted from THB at the
   live SOL rate. The buyer reviews, taps submit, and the reimbursement
   pipeline starts itself.
2. **Chat that proposes.** "ซื้อนม 2 ขวด งบ 200 บาท" becomes a draft request
   in the review tray. Budget questions ("งบเหลือเท่าไหร่") are answered
   strictly from a read-only snapshot of the live chain state attached to the
   request — the model never invents balances.
3. **The security model is the pitch.** The AI has *no* chain access and *no*
   keypair. It can only produce drafts into a review tray; every field is
   editable, every amount is re-validated against the program's guards, and a
   human signature is what moves money. This is agentic UX with household-grade
   safety — the AI is a proposing member of the family, not a signer.

Implementation: a single Next.js route (`/api/ai`) holds the key and speaks to
any OpenAI-compatible endpoint (env-swappable); prompts pin the model to a
strict JSON contract that pure, unit-tested validators normalize before the
UI ever renders it.

4. **A modelless policy floor under the AI.** Before any draft is submitted it
   is screened by a local, zero-weight decision engine
   ([riir-reflex](https://reflex.gist.rs)) against the household's own rules
   corpus (`corpus/household/*.md` — "staples under ฿300 are standing rules",
   "anything over ฿500 goes to review"). It answers in microseconds,
   bit-deterministically, offline, for free — and when the corpus can't ground
   an answer it **abstains to human review**, which is stocksie's default
   anyway. The stack is a three-tier safety cascade: **modelless rules floor →
   LLM proposals → human on-chain signature.** No tier can move money alone.

## Solana Layer

Solana is load-bearing, not decorative:

- **Program**: an Anchor program (14 instructions) implements the household,
  membership roles, the purchase state machine, reimbursement, and rewards.
  All authority checks are on-chain — the UI is a thin client.
- **PDA vault**: the household PDA *is* the treasury; Solana's account model
  gives a shared account no single human controls.
- **PDAs for identity**: households, members, and purchase requests are all
  deterministic accounts seeded by pubkeys — no server-side database exists
  anywhere in the product.
- **Censorship-free coordination**: a Parent approves from their wallet; the
  buyer is paid back from the vault to theirs. No processor can freeze the
  family's budget.
- Runs on Surfpool/localnet today, devnet for the demo. Stablecoin settlement
  (USDC/USDG) is the immediate post-hackathon step — the program's vault
  abstraction makes it a token-program swap, not a rewrite.

## Distribution

- **Where the users are**: Thai households coordinate in LINE today. A LINE
  Mini App wrapper of the /agent page (receipt photo → request → MOM gets an
  approval prompt) is the growth wedge — the AI feature is precisely what
  makes a wallet app feel like messaging.
- **Superteam Thailand** build-station / office hours during the hackathon for
  first feedback and demo day.
- **University pilot**: student clubs and dorm shared budgets — a self-serve
  "create your household" flow, seeded via the built-in dev wallet for
  frictionless onboarding on devnet.
- The reward system is the retention loop: children accumulate points for
  participation, which parents can treat as allowance leverage.

## Future / Traction

- Working end-to-end product (not a mock): program + tests + live UI, built
  across 8 planned phases (see `docs/`), with the AI layer shipped for this
  hackathon.
- Next 30 days: devnet → testnet deploy, USDC settlement, LINE Mini App
  wrapper, Thai-language receipt prompts tuned for local store receipts.
- Next 6 months: agent allowances (an on-chain instruction granting the AI a
  capped auto-approval budget, e.g. ≤฿200 without human sign-off — the vault
  model extends naturally), multi-household "grandparent view", and a
  merchant QR path so the approved request pays the store directly.

## Team

- **ozoneRatchapon** — protocol & frontend engineering; designed the on-chain
  household model, the privacy hashing scheme, and the AI agent architecture.
- **napatsakornnut** — repository owner; product & growth (Thailand
  distribution, Superteam network).

(Built in the open: docs, plans, and the full security-test mapping live in
the repo — `docs/SECURITY.md`, `plan/`, `.plans/`.)

---

## Pre-submission checklist

- [ ] Register on https://colosseum.com/worldsfair — **country: Thailand**
- [ ] Submit the project on the Colosseum platform
- [ ] Submit on https://superteam.fun/earn/listing/ai-solana-track/
- [ ] Set `AI_API_KEY` (+ optional `AI_BASE_URL`/`AI_*_MODEL`) in `app/.env.local`
- [ ] Optional policy gate: `brew install riir-reflex` then
      `RIIR_REFLEX_CORPUS=corpus/household reflex` — drafts get the
      "matches a household rule" badge; without it the app behaves identically
- [x] Deploy the program to devnet; point `NEXT_PUBLIC_RPC_ENDPOINT` at devnet —
      **done 2026-10-10**: `At2vd…Qoj` live (runbook:
      `docs/DEPLOY_DEVNET.md`; IDL account still to init)
- [ ] Record 2–3 min demo video: receipt photo → drafts → submit → owner
      approves → reimburse; plus a Thai chat proposal
- [x] Public GitHub repo visible to judges (transferred org:
      `napatsakornnut/stocksie`) — pushed through `b0e3572` on 2026-10-10
- [ ] Hosted UI link for judges — Vercel import + env vars per
      `docs/DEPLOY_VERCEL.md`, then paste the URL here: ___________
- [ ] IDL account on devnet (explorer polish, optional):
      `anchor idl init …` in `docs/DEPLOY_DEVNET.md` §2
