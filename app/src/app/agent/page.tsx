"use client";

// /agent route — the AI household agent (plan 005).
//
// Two input paths into one review pipeline: a receipt photo (vision) and a
// chat box (intent JSON). Both produce *draft* purchase requests that land in
// a single review tray; the connected wallet signs the real
// `create_purchase_request` transactions, so the AI can only ever propose —
// the household's approval flow on-chain stays the single source of authority.
//
// Layout mirrors /budget: back-to-dashboard link, title, wallet control.

import { AgentClient } from "@/components/agent/AgentClient";

export default function AgentPage() {
  return <AgentClient />;
}
