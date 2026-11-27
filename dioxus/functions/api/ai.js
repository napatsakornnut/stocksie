// AI proxy for the Dioxus app (same-origin — no CORS needed).
// Port of app/src/app/api/ai/route.ts: receipt + chat modes against any
// OpenAI-compatible endpoint. The key lives in Cloudflare Pages secrets:
//   npx wrangler pages secret put AI_API_KEY --project-name stocksie-dioxus
//
// Abuse brakes (the key's quota is the crown jewel):
//   - 20 req / 5 min per IP (isolate-memory bucket, best-effort per colo)
//   - cross-origin POSTs (Origin host != request host) rejected
//   - body ≤ 11 MB (receipt cap is 8 MB pre-base64), chat text ≤ 2000 chars
const LIMIT = 20;
const WINDOW = 5 * 60 * 1000;

function rateLimited(ip) {
  const buckets = (globalThis.__aiBuckets ||= new Map());
  const now = Date.now();
  const b = buckets.get(ip);
  if (!b || b.resetAt < now) {
    if (buckets.size > 5000) buckets.clear();
    buckets.set(ip, { count: 1, resetAt: now + WINDOW });
    return false;
  }
  b.count += 1;
  return b.count > LIMIT;
}

export async function onRequestPost({ request, env }) {
  const cors = {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type",
  };
  if (!env.AI_API_KEY) {
    return Response.json({ error: "AI_API_KEY not set on this Pages project" }, { status: 501, headers: cors });
  }

  // same-origin only when the browser announces an origin (curl stays open)
  const origin = request.headers.get("Origin");
  if (origin) {
    try {
      const oHost = new URL(origin).host;
      const here = new URL(request.url).host;
      const local = /^(localhost|127\.0\.0\.1)(:\d+)?$/.test(oHost);
      if (oHost !== here && !local) {
        return Response.json({ error: "Cross-origin use is not allowed." }, { status: 403, headers: cors });
      }
    } catch {
      return Response.json({ error: "Bad Origin header." }, { status: 400, headers: cors });
    }
  }

  const ip = request.headers.get("CF-Connecting-IP") || "unknown";
  if (rateLimited(ip)) {
    return Response.json({ error: "ใช้งานถี่เกินไป — ลองอีกครั้งในภายหลัง" }, { status: 429, headers: cors });
  }

  const len = Number(request.headers.get("Content-Length") || 0);
  if (len > 11 * 1024 * 1024) {
    return Response.json({ error: "ไฟล์ใหญ่เกินไป (สูงสุด 8 MB)" }, { status: 413, headers: cors });
  }

  const base = (env.AI_BASE_URL || "https://api.openai.com/v1").replace(/\/+$/, "");
  const visionModel = env.AI_VISION_MODEL || "gpt-4o-mini";
  const textModel = env.AI_TEXT_MODEL || "gpt-4o-mini";

  let body;
  try {
    body = await request.json();
  } catch {
    return Response.json({ error: "Invalid JSON body." }, { status: 400, headers: cors });
  }
  const mode = body.mode;
  if (mode !== "receipt" && mode !== "chat") {
    return Response.json({ error: "mode must be receipt or chat" }, { status: 400, headers: cors });
  }
  if (mode === "chat" && typeof body.text === "string" && body.text.length > 2000) {
    return Response.json({ error: "ข้อความยาวเกินไป (สูงสุด 2000 ตัวอักษร)" }, { status: 413, headers: cors });
  }

  const system =
    mode === "receipt"
      ? "You extract household purchases from receipt or shopping-list photos. Return STRICT JSON only, no markdown, in this shape: {\"currency\":\"THB\",\"items\":[{\"name\":\"UHT milk 1L\",\"quantity\":2,\"note\":\"\",\"totalMinor\":9400}]}. totalMinor is the LINE TOTAL (unit price × quantity) in the currency's minor unit (satang for THB, cents for USD). Exclude non-purchasable lines: totals, VAT, change, store headers. Merge duplicate lines. Keep item names in the language printed on the receipt. At most 20 items. If the image is not a receipt or shopping list, return {\"currency\":\"THB\",\"items\":[]}."
      : "You are the purchasing assistant inside Stocksie, a Solana app where a household shares a permissioned on-chain vault and approves purchase requests together. You PROPOSE; the human approves on-chain. You can never execute anything and must not claim otherwise. The user's message includes a READ-ONLY CONTEXT block (vault balance, pending request count, members, recent requests). Treat it as ground truth — never invent balances beyond it. Return STRICT JSON only, exactly one of: {\"intent\":\"propose_purchases\",\"reply\":\"<one short sentence>\",\"items\":[{\"name\":\"...\",\"quantity\":2,\"note\":\"\",\"totalMinor\":9400}]} or {\"intent\":\"budget_answer\",\"reply\":\"<answer using ONLY the context numbers>\"} or {\"intent\":\"clarify\",\"reply\":\"<a short question back>\"}. totalMinor is the estimated line total in THB satang unless another currency is named. Reply under 80 words in the user's language.";

  const userParts =
    mode === "receipt"
      ? [
          { type: "image_url", image_url: { url: `data:${body.image.mimeType};base64,${body.image.base64}` } },
          { type: "text", text: "Extract every purchasable line as JSON." },
        ]
      : [
          {
            type: "text",
            text:
              (body.context
                ? `READ-ONLY CONTEXT (ground truth): vault ${body.context.vaultBalanceSol} SOL, ${body.context.pendingRequestCount} pending, ${body.context.memberCount} members.\n\n`
                : "") + `USER: ${body.text}`,
          },
        ];

  const model = mode === "receipt" ? visionModel : textModel;
  try {
    const res = await fetch(`${base}/chat/completions`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${env.AI_API_KEY}` },
      body: JSON.stringify({
        model,
        messages: [
          { role: "system", content: mode === "receipt" ? system : chatSystem() },
          { role: "user", content: userParts },
        ],
        temperature: 0.2,
        max_tokens: 1500,
      }),
    });
    if (!res.ok) {
      const detail = await res.text().catch(() => "");
      return Response.json({ error: `AI provider error ${res.status}` }, { status: 502, headers: cors });
    }
    const json = await res.json();
    const content = json.choices?.[0]?.message?.content;
    if (!content) {
      return Response.json({ error: "AI provider returned an empty completion." }, { status: 502, headers: cors });
    }
    const fenced = content.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
    const candidate = (fenced ? fenced[1] : content).trim();
    const start = candidate.search(/[{[]/);
    if (start === -1) {
      return Response.json({ error: "AI reply contained no JSON." }, { status: 502, headers: cors });
    }
    const parsed = JSON.parse(candidate.slice(start));
    if (mode === "receipt") {
      if (!parsed.items || parsed.items.length === 0) {
        return Response.json({ error: "Could not find any purchasable items in that image." }, { status: 422, headers: cors });
      }
      return Response.json({ ok: true, mode: "receipt", currency: parsed.currency || "THB", items: parsed.items }, { headers: cors });
    }
    if (!parsed.intent || typeof parsed.reply !== "string") {
      return Response.json({ error: "The assistant reply was not actionable — try rephrasing." }, { status: 422, headers: cors });
    }
    return Response.json({ ok: true, mode: "chat", ...parsed }, { headers: cors });
  } catch (e) {
    return Response.json({ error: `AI provider failure: ${e.message}` }, { status: 502, headers: cors });
  }
}

function chatSystem() {
  return [
    "You are the purchasing assistant inside Stocksie, a Solana app where a household shares a permissioned on-chain vault and approves purchase requests together.",
    "You PROPOSE; the human approves on-chain. You can never execute anything and must not claim otherwise.",
    "The user's message includes a READ-ONLY CONTEXT block (vault balance, pending request count, members, recent requests). Treat it as ground truth — never invent balances beyond it.",
    "Return STRICT JSON only, exactly one of:",
    '{"intent":"propose_purchases","reply":"<one short sentence>","items":[{"name":"...","quantity":2,"note":"","totalMinor":9400}]}',
    '{"intent":"budget_answer","reply":"<answer using ONLY the context numbers>"}',
    '{"intent":"clarify","reply":"<a short question back>"}',
    "totalMinor is the estimated line total in THB satang unless another currency is named.",
    "Reply under 80 words in the user's language.",
  ].join("\n");
}

export async function onRequestOptions() {
  return new Response(null, {
    status: 204,
    headers: {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Methods": "POST, OPTIONS",
      "Access-Control-Allow-Headers": "Content-Type",
    },
  });
}
