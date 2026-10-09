# Plan 006 — Reflex policy gate (modelless household-rules floor)

ต่อจาก plan 005 — เอา `riir-reflex` (reflex.gist.rs) มาเป็นชั้นตัดสินที่ **ไม่ใช้
LLM** ใน pipeline ของ /agent

## ความจริงทางเทคนิคที่ต้องยอมรับ

- Reflex **แทน LLM ไม่ได้**: ไม่มี vision (อ่านใบเสร็จไม่ได้) และไม่มี
  generation (แต่งประโยคภาษาไทยไม่ได้) — สองงานนี้ยังต้องใช้ AI API
- สิ่งที่ Reflex ทำได้ดี: ตอบคำถาม typed (choice/noul) จาก corpus markdown
  ของเรา, µs-tier, bit-deterministic, **abstain เป็นคำตอบชั้นหนึ่ง**

## Design: "three-tier intelligence"

```
Tier 1  Reflex (local, modelless, ฟรี)     — draft นี้อยู่ในกฎประจำบ้านไหม?
        abstain / ต่ำกว่า threshold  ──────▶  Tier 2
Tier 2  LLM (server-side, /api/ai)         — แปลงใบเสร็จ/ภาษาพูดเป็น draft
Tier 3  มนุษย์เซ็น on-chain (มีอยู่แล้ว)   — ทรานแซกชันผ่าน wallet เท่านั้น
```

- `POST /api/policy` (server → `http://127.0.0.1:7331/decide`; Reflex ผูก
  loopback + ปิด CORS จึงต้องยิงจาก server เท่านั้น)
- คำถามต่อ draft: `noul` — "draft นี้ครอบคลุมโดยกฎประจำบ้านหรือไม่?"
  → `covered` เมื่อ yes และ confidence ≥ floor, นอกนั้น = `review`
- **Graceful degradation**: engine ไม่รัน / เข้าไม่ถึง → ไม่มี badge, flow เดิม
  ทุกอย่าง (default คือมนุษย์อนุมัติอยู่แล้ว จึงปลอดภัยตามสถาปัตยกรรมเดิม)
- corpus ตัวอย่าง: `corpus/household/*.md` — รันด้วย
  `RIIR_REFLEX_CORPUS=corpus/household reflex`
- Calibration (skill §6/§7): floor ตั้งได้ผ่าน `REFLEX_CONFIDENCE_FLOOR`
  (default 0.5) — และต้อง fit จาก data จริงก่อนอ้าง accuracy; feedback loop
  (`/feedback`) เป็น path หลัง hackathon

## ไฟล์

| ไฟล์ | คืออะไร |
|---|---|
| `app/src/lib/policy.ts` (+test) | state/question builders + verdict mapper (pure) |
| `app/src/app/api/policy/route.ts` | server proxy → Reflex /decide |
| `AgentClient.tsx` | badge "ตรงกฎบ้าน" ต่อ draft + เช็กอัตโนมัติเมื่อได้ draft ใหม่ |
| `corpus/household/` | กฎประจำบ้านตัวอย่าง (markdown) |
| `.env.example` | `REFLEX_URL`, `REFLEX_CONFIDENCE_FLOOR` |

## ไม่ทำ

- ไม่ให้ Reflex บล็อก/เซ็นแทน — มัน annotate เท่านั้น อำนาจอยู่ที่ลายเซ็น
- ไม่อ้าง accuracy จนกว่าจะ fit threshold บน labeled slice (≥16 case)
