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
- [x] v0.2 request ledger — getProgramAccounts + discriminator/base58 memcmp +
      borsh decode, cross-verify กับ raw RPC ตรงทุก field
- [x] v0.3 Wallet Standard signing — memo tx wire builder (golden test vs
      web3.js serializeMessage) + js-sys interop (connect / signAndSend);
      mock-provider verify: captured bytes == web3.js reconstruction
      (BYTE-IDENTICAL, 209 bytes) — real-Phantom test คือหมุดถัดไปของผู้ใช้
- [x] v0.4 agent UI — chat box ยิง /api/ai เดิม; route เพิ่ม origin-gated
      CORS (AI_ALLOWED_ORIGINS, default ปิด cross-origin เพื่อไม่ให้ยืม key)
      — verify สอง intent จริง: budget_answer grounded ตรง chain + Thai
      propose พร้อม item chips
- [x] v0.5 action instructions — compiler ทั่วไป (keys sort signers →
      writable → readonly + program idx remap) + builders จาก IDL จริง
      (disc/writable/signer); **E2E บน localnet ผ่านทั้ง lifecycle**:
      init → deposit → add_member → create → approve, assert state on-chain
      ทุกขั้น (บั๊กที่ E2E จับได้: program idx ไม่ remap, key sort 2 กลุ่ม
      ทำ owner_member หลุดโซน writable, buyer ต้อง add_member ก่อน +
      self-approval ถูกแบน)
- [x] v0.6 action parity ครบ — confirm_restock (buyer เซ็นเอง) +
      reimburse_buyer (owner จ่าย, buyer resolve จาก ledger) + add_member UI
      (role select) — E2E 6 ขั้นผ่านจน status = Reimbursed และ
      reimbursed_amount ตรง; mock verify add_member bytes (disc + role tag +
      ordering) — RIIR parity = 100% ของ household lifecycle (เหลือแค่
      receipt scan)
- [x] v0.7 receipt scan — file input (web-sys) → /api/ai → item chips →
      "Open request" ต่อบรรทัด (THB→SOL live rate, counter อ่านใหม่จาก
      chain ต่อ tx) — verify ในเบราว์เซอร์ด้วย fixture (app/public/
      receipt-test.html): 4 บรรทัด, bytes ที่จับได้ decode ถูกทุกจุด
      (38,515,908 lamports ≈ ฿94)
- [x] v0.9 ครบทุก instruction — decline/close/withdraw/award/remove/set_role
      (13/13 mutating instructions callable จาก Rust) — E2E 13 tx บน localnet
      ครบ (reject→close ทดสอบ rent reclaim เป็น account-gone ด้วย)
- [ ] (ถ้าต้องการ) styling parity — inline styles ทำงานแล้ว เหลือจัดฟอร์มสวยเท่านั้น

## บทเรียนทางเทคนิค (จากการ build ครั้งนี้)

- `solana-pubkey` 2.x: `find_program_address` ต้อง feature `curve25519`
  (บน non-SBF target); 3.x ย้าย API ไปแล้ว — pin "2"
- `getAccountInfo` ซ้อน account ใต้ `result.value` — double-unwrap ผิดชั้น
  ทำให้เสียเวลา debug หนึ่งรอบ
- gloo-net 0.8: `.json()`/`.body()` คืน Result ต้อง map_err
- dx (dioxus-cli 0.7.10): ต้องเคลียร์ `RUSTC_WRAPPER` เมื่อเครื่องมี sccache;
  hot-reload อาจค้าง — build static (`dx build --web`) + static server เชื่อถือ
  ได้กว่าเวลาทดสอบ
