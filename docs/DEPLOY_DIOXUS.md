# Deploy — Dioxus app (Cloudflare Pages)

ผู้ใช้หลักอยู่ที่ **https://stocksie-dioxus.pages.dev** (Cloudflare Pages, project
`stocksie-dioxus`) — static build ของ `dioxus/` crate + Cloudflare Pages Function
ที่ `dioxus/functions/api/ai.js` เป็น AI proxy same-origin (key อยู่ฝั่ง server)

> Next.js app (`app/`) ยัง deploy อยู่ที่ https://stocksie-th.vercel.app — ดู
> `DEPLOY_VERCEL.md`

## ตั้งค่าครั้งแรก (ครั้งเดียว)

```sh
# 1. toolchain
rustup target add wasm32-unknown-unknown
cargo install dioxus-cli          # dx
npx wrangler login                # auth Cloudflare (browser)

# 2. secrets ของ AI proxy (ต่อโปรเจกต์ — ถามค่าจาก .env.local ของ app/)
cd dioxus
npx wrangler pages secret put AI_API_KEY --project-name stocksie-dioxus      # Gemini API key
npx wrangler pages secret put AI_BASE_URL --project-name stocksie-dioxus     # https://generativelanguage.googleapis.com/v1beta/openai
npx wrangler pages secret put AI_VISION_MODEL --project-name stocksie-dioxus # เช่น gemini-3.5-flash-lite
npx wrangler pages secret put AI_TEXT_MODEL --project-name stocksie-dioxus   # เช่น gemini-3.5-flash-lite
```

รายละเอียด route: `app/src/app/api/ai/route.ts` (mode receipt / chat,
context จาก chain)

## Deploy ปกติ (คำสั่งเดียว)

```sh
dioxus/scripts/deploy-pages.sh
```

script ทำ: build release → stage `/tmp/dx-pages` (ไฟล์เว็บ + `functions/` +
`_headers`) → `wrangler pages deploy` → พิมพ์ checklist ยืนยัน

### ทำไมต้อง stage ไป /tmp

`/Users/ozone/.pnp.cjs` (Yarn Plug'n'Play ค้างจาก 2025-03) อยู่เหนือทุก path
ใน `$HOME` — esbuild ของ wrangler จะเจอและปัด manifest นั้นใส่การ resolve
`@cloudflare/unenv-preset` / `path-to-regexp` ของตัว wrangler เอง จน bundling
Functions พังทุกครั้งเมื่อรันจากใต้ `$HOME` (เช่นจาก `dioxus/`) — จาก `/tmp`
ไม่พัง

## Verify หลัง deploy (สำคัญ — deploy "สำเร็จ" ไม่ได้แปลว่าไฟล์ใหม่ขึ้นจริง)

1. **asset hash เปลี่ยนจริง** — wrangler dedupe ด้วย content hash และบอก
   `Uploaded 0 files (N already uploaded)` เมื่อ stage ได้ build เก่า:

   ```sh
   curl -s https://stocksie-dioxus.pages.dev/ | grep -o 'assets/[^"]*\.js'
   # ต้องตรงกับ assets/... ใน index.html ของ build ใหม่
   ```

2. **AI proxy ยังตอบ**

   ```sh
   curl -X POST https://stocksie-dioxus.pages.dev/api/ai \
     -H 'Content-Type: application/json' \
     -d '{"mode":"chat","text":"ทดสอบ","context":{"vaultBalanceSol":"0.001","pendingRequestCount":0,"memberCount":1,"recentRequests":[]}}'
   ```

3. **เปิดเว็บจริง** — โหลดหน้า (index.html ต้อง `cache-control: no-cache`),
   แชทตอบภาษาไทย, ไม่มี "TypeError: Failed to fetch"

## Debug: wasm ที่ serve ใช้ AI URL ไหน

```sh
curl -s https://stocksie-dioxus.pages.dev/assets/<hash>.wasm -o /tmp/served.wasm
python3 - <<'EOF'
import re
data = open('/tmp/served.wasm','rb').read()
for m in re.finditer(rb'/api/ai', data):
    print(repr(data[max(0,m.start()-45):m.end()+8]))
EOF
```

wasm ที่ถูกต้องมีทั้ง absolute fallback (`stocksie-th.vercel.app/api/ai`) **และ**
`/api/ai` แบบ relative คู่กับ `localhost`/`127.0.0.1` (layout ของ
`ai_api_url()` ใน `dioxus/src/main.rs`) — ถ้ามีแต่ absolute แปลว่า wasm เก่า

## Gotchas สะสม

- **รัน dx จาก `dioxus/` เสมอ** — จาก workspace root ได้ "Failed to find binary
  package"; และต้องเคลียร์ `RUSTC_WRAPPER=""` เมื่อเครื่องมี sccache
- **release build เท่านั้น** — debug มี banner "Dioxus dev" ติดมาในหน้าเว็บ
- index.html ไม่ cache (`max-age=0, must-revalidate`) — ผู้ใช้ได้ wasm ใหม่ทันที
  ที่ reload; `/assets/*` cache ถาวร (`_headers`, ชื่อไฟล์ hash กันชน)
- RPC มี retry 429/network ×3 + error ภาษาไทยใน app แล้ว (`b91de93`)
- โปรเจกต์ Vercel `stocksie-dioxus` (เวอร์ชันเก่า) ไม่มี AI proxy — ถ้าจะใช้
  ต้องเพิ่ม origin ใน `AI_ALLOWED_ORIGINS` ของ stocksie-th ทาง dashboard
