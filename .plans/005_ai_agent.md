# Plan 005 — AI Household Agent (AI × Solana track, Colosseum World's Fair)

Deadline ส่ง: **12 ต.ค. 2026** (วันนี้ 6 ต.ค. → เหลือ 6 วัน)
Track: Colosseum global (Solana ecosystem) + Superteam Thailand "AI × Solana" ($10k USDG)

## เป้าหมาย

เติม AI layer ที่ "มีความหมาย" (judging criterion: *AI should create genuine
leverage within the product*) ให้กับ stocksie โดยไม่แตะโปรแกรม on-chain เลย —
ใช้ instruction set ที่มีอยู่แล้วทั้ง 14 ตัว

**Narrative: "AI proposes, the household approves on-chain."**
AI ไม่มีสิทธิ์ใช้เงิน — มันแค่แปลงโลกจริง (ใบเสร็จ, ภาษาพูด) เป็น purchase
requests ที่ผ่าน approval flow ที่ program บังคับอยู่แล้ว (permissioned vault,
blake3 privacy hashes, 0.0001–0.5 SOL circuit breaker)

## Scope

### A. Receipt → purchase requests (vision)
ถ่าย/อัปโหลดรูปใบเสร็จ → vision model แตกเป็นรายการสินค้า (ชื่อ, จำนวน, ราคา
THB) → client แปลง THB→SOL (live rate จาก CoinGecko + fallback env) →
draft cards แก้ได้ → submit เป็น `create_purchase_request` ทีละรายการ

### B. Chat agent (text → intent)
พิมพ์ภาษาไทย/อังกฤษ ("ซื้อนม 2 ขวด งบ 200 บาท", "งบเหลือเท่าไหร่") → LLM คืน
intent JSON 3 แบบเท่านั้น (validated ฝั่ง client ด้วย pure validators):
- `propose_purchases` → draft cards (pipeline เดียวกับ A)
- `budget_answer` → ตอบจาก context ที่ client แนบไป (vault balance, pending
  count, members) — ตัวเลข grounded ไม่ hallucinate
- `clarify` → ถามกลับ

**ไม่ทำ**: auto-approve, server-side chain access, agent wallet แยก — เล่าเรื่อง
ได้ว่า security model ตั้งใจให้ AI มีสิทธิ์ "เสนอ" เท่านั้น

## Architecture

```
browser ──POST /api/ai──▶ Next route handler (first server code)
   │                        │  fetch → OpenAI-compatible API (env-driven:
   │                        │  AI_BASE_URL / AI_API_KEY / AI_*_MODEL)
   │◀── intent JSON ────────┘  (normalized + validated)
   │
   ├─ drafts tray (editable, validated: MIN/MAX lamports)
   └─ Submit selected → program.methods.createPurchaseRequest loop
        (each tx re-reads requestCounter — same race-safe pattern as
         PurchasePanel.tsx:166; buyer = connected wallet)
```

- AI server-side เท่านั้น — key ไม่หลุดสู่ browser
- on-chain ไม่เก็บ cleartext — ใช้ `toHash32()` (blake3) เดิม
- ไม่เพิ่ม dependency เลย (fetch native, ไม่มี SDK)

## Files

| ไฟล์ | คืออะไร |
|---|---|
| `src/lib/ai.ts` | pure types + validators + prompt builders (testable) |
| `src/lib/ai.test.ts` | validator/normalizer tests |
| `src/lib/fx.ts` + test | THB↔lamports conversion + rate fallback |
| `src/app/api/ai/route.ts` | single endpoint: mode `receipt` \| `chat` |
| `src/app/agent/page.tsx` | route shell (copy /budget pattern) |
| `src/components/agent/AgentClient.tsx` | uploader + drafts tray + chat UI |
| `Dashboard.tsx` | nav link 🤖 Agent |
| `.env.example` | AI_* vars + `NEXT_PUBLIC_SOL_THB_FALLBACK` |

## หลัง implement

1. Deploy program ขึ้น devnet + เซ็ต `NEXT_PUBLIC_RPC_ENDPOINT` → demo คลิกได้จริง
2. Demo video 2–3 นาที + submission essay (Problem, User, Product, AI, Solana
   Layer, Distribution, Future, Team)
3. Submit ทั้ง Colosseum platform + Superteam Earn listing ก่อน 12 ต.ค.

## Out of scope (หลัง hackathon)

LINE Mini App distribution, agent wallet ที่ได้รับ delegated allowance
(program ต้องเพิ่ม instruction), OCR fallback local
