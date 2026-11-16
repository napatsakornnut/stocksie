# Plan 007 — RIIR เป็น Dioxus (prototype)

หลังปิด hackathon (deadline 12 ต.ค. ผ่านไปแล้ว) ผู้ใช้อยากลอง RIIR frontend
เป็น Rust ด้วย Dioxus ประกอบกับธีมของระบบนิเวศ reflex (gist-rs) ที่เป็น Rust

## หลักการตัดสินใจ

- **Additive ไม่ใช่ rewrite ทันที** — `app/` (Next.js) ยังเป็น production
  surface (กรรมการ/ผู้ใช้ใช้อยู่: https://stocksie-th.vercel.app) crate ใหม่
  `dioxus/` อยู่ข้าง ๆ แบบ standalone (มี `[workspace]` ว่าง ๆ ตัดจาก Anchor
  workspace ที่ root)
- **อ่านก่อนเขียน** — v0.1 เลือก slice ที่พิสูจน์เส้นทางได้โดยไม่ต้องแตะ
  wallet: derive PDA + fetch JSON-RPC + decode borby ด้วยมือ (offsets
  ใน src/main.rs) ผ่านการ verify กับ devnet จริงทุก field
- ขอบเขตต่อไปเรียงตามความยากใน README: request ledger → Wallet Standard
  signing interop (จุดยากสุด — provider เป็น JS injection) → agent page

## สถานะ

- [x] v0.1 household reader — verify กับ devnet ครบ (commit 752a07d)
- [ ] request ledger (getProgramAccounts + memcmp + decode)
- [ ] Wallet Standard signing ผ่าน web-sys
- [ ] agent UI (ยิง /api/ai เดิมได้เลย)

## บทเรียนทางเทคนิค (จากการ build ครั้งนี้)

- `solana-pubkey` 2.x: `find_program_address` ต้อง feature `curve25519`
  (บน non-SBF target); 3.x ย้าย API ไปแล้ว — pin "2"
- `getAccountInfo` ซ้อน account ใต้ `result.value` — double-unwrap ผิดชั้น
  ทำให้เสียเวลา debug หนึ่งรอบ
- gloo-net 0.8: `.json()`/`.body()` คืน Result ต้อง map_err
- dx (dioxus-cli 0.7.10): ต้องเคลียร์ `RUSTC_WRAPPER` เมื่อเครื่องมี sccache;
  hot-reload อาจค้าง — build static (`dx build --web`) + static server เชื่อถือ
  ได้กว่าเวลาทดสอบ
