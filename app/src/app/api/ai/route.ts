// /api/ai — the app's first (and only) server-side route (plan 005).
//
// Holds the LLM API key server-side and speaks to any OpenAI-compatible
// chat-completions endpoint (OpenAI, Z.ai GLM, Together, local Ollama with an
// /v1 path, …) — configured purely via env so the demo can swap providers
// without a code change:
//
//   AI_API_KEY       required; without it the route answers 501 with setup help
//   AI_BASE_URL      default https://api.openai.com/v1
//   AI_VISION_MODEL  default gpt-4o-mini (receipt mode)
//   AI_TEXT_MODEL    default gpt-4o-mini (chat mode)
//
// The route has NO chain access. It converts images / text into validated
// intent JSON (lib/ai.ts normalizers) and the browser is what signs and sends
// the Solana transactions — the security model is "AI proposes, the household
// approves on-chain", and the server never touches a keypair.
//
// `response_format: json_object` is requested opportunistically: several
// OpenAI-compatible providers reject the parameter outright, so a 400 gets
// one plain-prompt retry rather than failing the demo.

import { NextResponse } from "next/server";
import {
  parseAgentRequest,
  parseReceiptReply,
  parseChatReply,
  receiptSystemPrompt,
  chatSystemPrompt,
  type ReceiptParse,
  type AgentIntent,
  type AgentRequestBody,
} from "@/lib/ai";

interface AiConfig {
  baseUrl: string;
  apiKey: string;
  visionModel: string;
  textModel: string;
}

function aiConfig(): AiConfig | null {
  const apiKey = process.env.AI_API_KEY?.trim();
  if (!apiKey) return null;
  return {
    baseUrl: (
      process.env.AI_BASE_URL?.trim() || "https://api.openai.com/v1"
    ).replace(/\/+$/, ""),
    apiKey,
    visionModel: process.env.AI_VISION_MODEL?.trim() || "gpt-4o-mini",
    textModel: process.env.AI_TEXT_MODEL?.trim() || "gpt-4o-mini",
  };
}

type ChatPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } };

interface ChatMessage {
  role: "system" | "user";
  content: string | ChatPart[];
}

function buildMessages(body: AgentRequestBody): ChatMessage[] {
  if (body.mode === "receipt") {
    const parts: ChatPart[] = [
      {
        type: "image_url",
        image_url: {
          url: `data:${body.image!.mimeType};base64,${body.image!.base64}`,
        },
      },
      { type: "text", text: "Extract every purchasable line as JSON." },
    ];
    return [
      { role: "system", content: receiptSystemPrompt() },
      { role: "user", content: parts },
    ];
  }
  const ctx = body.context!;
  const contextBlock = [
    "READ-ONLY CONTEXT (ground truth):",
    `- Vault balance: ${ctx.vaultBalanceSol} SOL`,
    `- Pending purchase requests: ${ctx.pendingRequestCount}`,
    `- Household members: ${ctx.memberCount}`,
    `- Recent requests: ${
      ctx.recentRequests.length > 0 ? ctx.recentRequests.join("; ") : "none"
    }`,
  ].join("\n");
  return [
    { role: "system", content: chatSystemPrompt() },
    { role: "user", content: `${contextBlock}\n\nUSER: ${body.text}` },
  ];
}

async function callChat(
  cfg: AiConfig,
  model: string,
  messages: ChatMessage[]
): Promise<string> {
  const body = {
    model,
    messages,
    temperature: 0.2,
    max_tokens: 1500,
    response_format: { type: "json_object" as const },
  };
  let res = await fetch(`${cfg.baseUrl}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${cfg.apiKey}`,
    },
    body: JSON.stringify(body),
  });
  // Some OpenAI-compatible providers reject response_format outright — retry
  // once without it before giving up (the prompts already demand strict JSON).
  if (res.status === 400) {
    const { response_format: _ignored, ...rest } = body;
    res = await fetch(`${cfg.baseUrl}/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${cfg.apiKey}`,
      },
      body: JSON.stringify(rest),
    });
  }
  if (!res.ok) {
    const detail = await res.text().catch(() => "");
    throw new Error(
      `AI provider error ${res.status}${detail ? `: ${detail.slice(0, 200)}` : ""}`
    );
  }
  const json = (await res.json()) as {
    choices?: { message?: { content?: string } }[];
  };
  const content = json.choices?.[0]?.message?.content;
  if (typeof content !== "string" || content.length === 0) {
    throw new Error("AI provider returned an empty completion.");
  }
  return content;
}

/** Models like to wrap JSON in ``` fences despite the prompt — strip them. */
function extractJson(content: string): unknown {
  const fenced = content.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
  const candidate = (fenced ? fenced[1] : content).trim();
  const start = candidate.search(/[{[]/);
  if (start === -1) throw new Error("AI reply contained no JSON.");
  return JSON.parse(candidate.slice(start));
}

export async function POST(request: Request) {
  const cfg = aiConfig();
  if (!cfg) {
    return NextResponse.json(
      {
        error:
          "AI is not configured on this server. Set AI_API_KEY (and optionally AI_BASE_URL / AI_VISION_MODEL / AI_TEXT_MODEL) — any OpenAI-compatible endpoint works.",
      },
      { status: 501 }
    );
  }

  let parsedBody;
  try {
    parsedBody = parseAgentRequest(await request.json());
  } catch {
    return NextResponse.json({ error: "Invalid JSON body." }, { status: 400 });
  }
  if (parsedBody === null) {
    return NextResponse.json({ error: "Invalid request body." }, { status: 400 });
  }
  if ("error" in parsedBody) {
    return NextResponse.json({ error: parsedBody.error }, { status: 400 });
  }

  const messages = buildMessages(parsedBody);
  try {
    const content = await callChat(
      cfg,
      parsedBody.mode === "receipt" ? cfg.visionModel : cfg.textModel,
      messages
    );
    const raw = extractJson(content);

    if (parsedBody.mode === "receipt") {
      const parse: ReceiptParse | null = parseReceiptReply(raw);
      if (!parse) {
        return NextResponse.json(
          { error: "Could not find any purchasable items in that image." },
          { status: 422 }
        );
      }
      return NextResponse.json({ ok: true, mode: "receipt", ...parse });
    }

    const intent: AgentIntent | null = parseChatReply(raw);
    if (!intent) {
      return NextResponse.json(
        { error: "The assistant reply was not actionable — try rephrasing." },
        { status: 422 }
      );
    }
    return NextResponse.json({ ok: true, mode: "chat", ...intent });
  } catch (err) {
    const message =
      err instanceof Error ? err.message : "Unknown AI provider failure.";
    return NextResponse.json({ error: message }, { status: 502 });
  }
}
