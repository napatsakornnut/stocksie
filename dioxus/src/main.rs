// Stocksie RIIR prototype (plan 007) — the household read view rewritten in
// Rust on the web via Dioxus/WASM.
//
// Scope of this first step:
//   - derive the household PDA client-side in pure Rust
//     (`Pubkey::find_program_address`, solana-pubkey)
//   - fetch the account over JSON-RPC (gloo-net, WASM)
//   - hand-decode the Anchor borsh layout — no Anchor TS client, no JS
//
// Layout (programs/stocksie/src/state/household.rs, all little-endian):
//   [0..8]    Anchor discriminator
//   [8..40]   owner: Pubkey
//   [40..72]  name_hash: [u8; 32]  (blake3 of the display name — raw name
//             never lands on-chain)
//   [72]      bump: u8
//   [73..77]  member_count: u32
//   [77..85]  request_counter: u64
//   [85..93]  total_rewards_distributed: u64
//   [93..101] vault_balance: u64  (mirror; the account's lamports are truth)
//   [101..109] created_slot: u64
//
// Not yet in Rust (the real RIIR frontier, see README): the AI agent UI. The
// Next.js app remains the production surface — this crate is additive.

mod programs;
mod tx;
mod wallet;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use dioxus::prelude::*;
use gloo_net::http::Request;
use serde_json::json;
use sha2::{Digest, Sha256};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Stocksie program (deployed on devnet — see docs/DEPLOY_DEVNET.md).
const PROGRAM_ID: &str = "At2vd5Mqd9xcHRFAZin1VHb6upEsF1GCXoFG19UHsQoj";

/// The Next.js AI route (plan 005). The route admits cross-origin POSTs only
/// from origins listed in AI_ALLOWED_ORIGINS on the server — see
/// app/src/app/api/ai/route.ts. Point this at the hosted URL to reuse that
/// deployment instead of a local dev server.
const AI_API_URL: &str = "http://localhost:3000/api/ai";

/// Devnet JSON-RPC endpoint (the prototype targets devnet like the hosted app).
const RPC_URL: &str = "https://api.devnet.solana.com";

/// Households to try out of the box (owner address, label).
const DEMO_OWNERS: [(&str, &str); 2] = [
    (
        "98KkCG8KnhvkFbbEPx5JkpHpwzDbJKxuzBGK38Fcpnod",
        "devnet demo (บ้านdevnet)",
    ),
    (
        "9Bz7p4RWdX7eaR4hFUeCc7aSZjDHsie8q1u8imwavkBN",
        "deploy wallet (no household)",
    ),
];

fn main() {
    dioxus::launch(app);
}

#[derive(Clone, Debug, PartialEq)]
struct HouseholdView {
    pda: String,
    /// Raw key so downstream fetches (the request ledger) can filter on it.
    pda_key: Pubkey,
    lamports_sol: String,
    owner: String,
    name_hash_hex: String,
    member_count: u32,
    request_counter: u64,
    total_rewards: u64,
    vault_balance_mirror: u64,
    created_slot: u64,
}

#[derive(Clone, Debug)]
enum FetchState {
    Idle,
    Loading,
    Loaded(Box<HouseholdView>),
    Failed(String),
}

/// One decoded `PurchaseRequest` account.
#[derive(Clone, Debug, PartialEq)]
struct RequestView {
    pubkey: String,
    request_id: u64,
    amount_sol: String,
    status: &'static str,
    buyer: String,
    reward_earned: u64,
    created_slot: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum LedgerState {
    Idle,
    Loading,
    Loaded(Vec<RequestView>),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
enum SendState {
    Idle,
    Building,
    Sent(String),
    Failed(String),
}

/// One AI-agent conversation turn (plan 005 chat contract, client side).
#[derive(Clone, Debug, PartialEq)]
struct ChatMsg {
    role: &'static str, // "user" | "assistant"
    text: String,
    items: Vec<ChatItem>,
}

#[derive(Clone, Debug, PartialEq)]
struct ChatItem {
    name: String,
    quantity: u32,
    total_minor: i64,
}

impl ChatItem {
    fn label(&self) -> String {
        format!(
            "{} ×{} · ฿{:.2}",
            self.name,
            self.quantity,
            self.total_minor as f64 / 100.0
        )
    }
}

/// Parse a user-typed SOL amount to lamports (string math, ≤9 decimals).
fn parse_sol_to_lamports(input: &str) -> Option<u64> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    let (whole, frac) = match t.split_once('.') {
        Some((w, f)) => (w, f),
        None => (t, ""),
    };
    if whole.is_empty() && frac.is_empty() {
        return None;
    }
    if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if frac.len() > 9 {
        return None;
    }
    let lamports: u128 = whole.parse::<u128>().ok()? * 1_000_000_000
        + format!("{:0<9}", frac).parse::<u128>().ok()?;
    u64::try_from(lamports).ok()
}

/// Fetch the current blockhash from devnet (shared by every send path).
async fn fetch_blockhash() -> Result<[u8; 32], String> {
    let r = rpc_call(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getLatestBlockhash",
        "params": [{"commitment": "confirmed"}]
    }))
    .await?;
    let bh = r
        .get("value")
        .and_then(|v| v.get("blockhash"))
        .and_then(|b| b.as_str())
        .ok_or("missing blockhash in RPC response")?
        .to_string();
    bs58::decode(&bh)
        .into_vec()
        .map_err(|e| format!("bad blockhash: {e}"))
        .and_then(|v| <[u8; 32]>::try_from(v).map_err(|_| "blockhash length".to_string()))
}

/// Build + hand to the injected wallet for signing/submission.
async fn execute(payer: &Pubkey, ixs: Vec<tx::ProgramIx>) -> Result<String, String> {
    let provider = wallet::detect_provider()?;
    let blockhash = fetch_blockhash().await?;
    let wire = tx::build_wire_tx(payer, &blockhash, &ixs);
    wallet::sign_and_send(&provider, &wire).await
}

/// Reload household + ledger into the page signals (used by the fetch button
/// and after every successful action).
async fn refresh_into(
    mut state: Signal<FetchState>,
    mut ledger: Signal<LedgerState>,
    owner_raw: String,
) {
    let owner_raw = owner_raw.trim().to_string();
    state.set(FetchState::Loading);
    ledger.set(LedgerState::Idle);
    match load_household(&owner_raw).await {
        Ok(v) => {
            let key = v.pda_key;
            state.set(FetchState::Loaded(Box::new(v)));
            // Household lives → pull its purchase-request ledger.
            ledger.set(LedgerState::Loading);
            ledger.set(match load_ledger(&key).await {
                Ok(rows) => LedgerState::Loaded(rows),
                Err(e) => LedgerState::Failed(e),
            });
        }
        Err(e) => {
            state.set(FetchState::Failed(e));
            ledger.set(LedgerState::Idle);
        }
    }
}

fn app() -> Element {
    let mut owner_input = use_signal(|| DEMO_OWNERS[0].0.to_string());
    let mut state = use_signal(|| FetchState::Idle);
    let mut ledger = use_signal(|| LedgerState::Idle);
    let mut wallet_pubkey = use_signal(|| None::<String>);
    let mut wallet_error = use_signal(|| None::<String>);
    let mut wallet_busy = use_signal(|| false);
    let mut send_state = use_signal(|| SendState::Idle);
    let mut action_msg = use_signal(String::new);
    let mut action_busy = use_signal(|| false);
    let mut name_input = use_signal(String::new);
    let mut deposit_input = use_signal(String::new);
    let mut item_input = use_signal(String::new);
    let mut req_amount_input = use_signal(String::new);
    let mut approve_id_input = use_signal(String::new);
    let mut chat = use_signal(Vec::new);
    let mut chat_input = use_signal(String::new);
    let mut chat_busy = use_signal(|| false);

    let connect_wallet = move |_| {
        spawn(async move {
            wallet_busy.set(true);
            wallet_error.set(None);
            match wallet::detect_provider() {
                Err(e) => wallet_error.set(Some(e)),
                Ok(provider) => match wallet::connect(&provider).await {
                    Ok(pk) => wallet_pubkey.set(Some(pk)),
                    Err(e) => wallet_error.set(Some(e)),
                },
            }
            wallet_busy.set(false);
        });
    };

    let send_memo = move |_| {
        spawn(async move {
            wallet_error.set(None);
            let Some(pk) = wallet_pubkey().clone() else {
                wallet_error.set(Some("Connect a wallet first.".into()));
                return;
            };
            send_state.set(SendState::Building);
            let payer = match Pubkey::from_str(&pk) {
                Ok(p) => p,
                Err(_) => {
                    send_state.set(SendState::Failed("connected key is invalid".into()));
                    return;
                }
            };
            // recent blockhash from devnet, then hand the wire bytes to the
            // injected wallet — IT fills the signature slot and submits.
            let blockhash = match rpc_call(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getLatestBlockhash",
                "params": [{"commitment": "confirmed"}]
            })).await.and_then(|r| {
                let bh = r.get("value")
                    .and_then(|v| v.get("blockhash"))
                    .and_then(|b| b.as_str())
                    .ok_or("missing blockhash in RPC response")?
                    .to_string();
                bs58::decode(&bh).into_vec()
                    .map_err(|e| format!("bad blockhash: {e}"))
                    .and_then(|v| <[u8; 32]>::try_from(v).map_err(|_| "blockhash length".to_string()))
            }) {
                Ok(bh) => bh,
                Err(e) => {
                    send_state.set(SendState::Failed(e));
                    return;
                }
            };
            let wire = tx::build_memo_tx(&payer, &blockhash, "GM from Stocksie RIIR 🦀 (Dioxus/WASM)");
            match wallet::detect_provider() {
                Err(e) => send_state.set(SendState::Failed(e)),
                Ok(provider) => match wallet::sign_and_send(&provider, &wire).await {
                    Ok(sig) => send_state.set(SendState::Sent(sig)),
                    Err(e) => send_state.set(SendState::Failed(e)),
                },
            }
        });
    };

    // --- action handlers (plan 007 v0.5) — every op = wire bytes + wallet sig ---
    let init_household = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let name = name_input().trim().to_string();
            if name.is_empty() {
                action_msg.set("⚠ Enter a household name.".into());
                action_busy.set(false);
                return;
            }
            let owner = Pubkey::from_str(&pk).expect("connected key");
            match execute(
                &owner,
                vec![programs::initialize_household(
                    &owner,
                    programs::text_hash(&name),
                    &programs::program_id(),
                )],
            )
            .await
            {
                Ok(sig) => {
                    action_msg.set(format!("✓ household created — {}", &sig[..16.min(sig.len())]));
                    refresh_into(state, ledger, pk).await;
                }
                Err(e) => action_msg.set(format!("⚠ {e}")),
            }
            action_busy.set(false);
        });
    };

    let deposit = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Some(lamports) = parse_sol_to_lamports(&deposit_input()) else {
                action_msg.set("⚠ Enter a valid SOL amount.".into());
                action_busy.set(false);
                return;
            };
            let owner = Pubkey::from_str(&pk).expect("connected key");
            match execute(&owner, vec![programs::deposit_funds(&owner, lamports, &programs::program_id())]).await {
                Ok(sig) => {
                    action_msg.set(format!("✓ deposited — {}", &sig[..16.min(sig.len())]));
                    refresh_into(state, ledger, pk).await;
                }
                Err(e) => action_msg.set(format!("⚠ {e}")),
            }
            action_busy.set(false);
        });
    };

    let open_request = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Some(lamports) = parse_sol_to_lamports(&req_amount_input()) else {
                action_msg.set("⚠ Enter a valid SOL limit.".into());
                action_busy.set(false);
                return;
            };
            let item = item_input().trim().to_string();
            if item.is_empty() {
                action_msg.set("⚠ Enter the item to buy.".into());
                action_busy.set(false);
                return;
            }
            // the request PDA needs the household's CURRENT counter + 1
            let counter = match &*state.read() {
                FetchState::Loaded(v) => v.request_counter + 1,
                _ => {
                    action_msg.set("⚠ Load a household first.".into());
                    action_busy.set(false);
                    return;
                }
            };
            let owner = Pubkey::from_str(&pk).expect("connected key");
            match execute(
                &owner,
                vec![programs::create_purchase_request(
                    &owner,
                    &owner, // buyer = self; approval needs another member
                    counter,
                    lamports,
                    programs::text_hash(&item),
                    programs::text_hash("best-value snapshot"),
                    &programs::program_id(),
                )],
            )
            .await
            {
                Ok(sig) => {
                    action_msg.set(format!("✓ request opened — {}", &sig[..16.min(sig.len())]));
                    refresh_into(state, ledger, pk).await;
                }
                Err(e) => action_msg.set(format!("⚠ {e}")),
            }
            action_busy.set(false);
        });
    };

    let approve = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Ok(request_id) = approve_id_input().trim().parse::<u64>() else {
                action_msg.set("⚠ Enter the request number.".into());
                action_busy.set(false);
                return;
            };
            let owner = Pubkey::from_str(&pk).expect("connected key");
            match execute(&owner, vec![programs::approve_purchase_request(&owner, request_id, &programs::program_id())]).await {
                Ok(sig) => {
                    action_msg.set(format!("✓ approved — {}", &sig[..16.min(sig.len())]));
                    refresh_into(state, ledger, pk).await;
                }
                Err(e) => action_msg.set(format!("⚠ {e}")),
            }
            action_busy.set(false);
        });
    };

    let send_chat = move |_| {
        spawn(async move {
            let text = chat_input().trim().to_string();
            if text.is_empty() || chat_busy() {
                return;
            }
            chat_input.set(String::new());
            chat.push(ChatMsg { role: "user", text: text.clone(), items: vec![] });
            chat_busy.set(true);

            // Grounding context: read household/ledger state BEFORE any await
            // (signals can't be held across awaits in Dioxus).
            let (vault, pending, members, recent) = {
                let st = state.read();
                let (vault, members) = match &*st {
                    FetchState::Loaded(v) => (v.lamports_sol.clone(), v.member_count),
                    _ => ("unknown".to_string(), 0),
                };
                let lg = ledger.read();
                let rows: Vec<RequestView> = match &*lg {
                    LedgerState::Loaded(rows) => rows.clone(),
                    _ => Vec::new(),
                };
                let pending = rows
                    .iter()
                    .filter(|r| r.status == "waiting")
                    .count();
                let recent: Vec<String> = rows
                    .iter()
                    .take(5)
                    .map(|r| format!("#{} {} for {} SOL", r.request_id, r.status, r.amount_sol))
                    .collect();
                (vault, pending, members, recent)
            };

            let body = json!({
                "mode": "chat",
                "text": text,
                "context": {
                    "vaultBalanceSol": vault,
                    "pendingRequestCount": pending,
                    "memberCount": members,
                    "recentRequests": recent,
                }
            });
            match post_json(AI_API_URL, body).await {
                Ok(v) => {
                    let ok = v.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                    let mode = v.get("mode").and_then(|m| m.as_str()).unwrap_or("");
                    if ok && mode == "chat" {
                        let reply = v
                            .get("reply")
                            .and_then(|r| r.as_str())
                            .unwrap_or("")
                            .to_string();
                        let items = v
                            .get("items")
                            .and_then(|i| i.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|it| {
                                        Some(ChatItem {
                                            name: it.get("name")?.as_str()?.to_string(),
                                            quantity: it
                                                .get("quantity")
                                                .and_then(|q| q.as_u64())
                                                .unwrap_or(1) as u32,
                                            total_minor: it
                                                .get("totalMinor")
                                                .and_then(|t| t.as_i64())
                                                .unwrap_or(0),
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        chat.push(ChatMsg { role: "assistant", text: reply, items });
                    } else {
                        let err = v
                            .get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or("unexpected AI response")
                            .to_string();
                        chat.push(ChatMsg {
                            role: "assistant",
                            text: format!("⚠ {err}"),
                            items: vec![],
                        });
                    }
                }
                Err(e) => chat.push(ChatMsg {
                    role: "assistant",
                    text: format!("⚠ {e}"),
                    items: vec![],
                }),
            }
            chat_busy.set(false);
        });
    };

    let fetch = move |owner_raw: String| {
        spawn(async move {
            refresh_into(state, ledger, owner_raw).await;
        });
    };

    rsx! {
        main { style: "max-width:640px;margin:48px auto;padding:0 20px;font-family:ui-sans-serif,system-ui;color:#1c1917",
            header {
                h1 { "🤖🏠 Stocksie — RIIR prototype" }
                p { style: "color:#78716c;font-size:14px",
                    "Household reader rewritten in Rust (Dioxus → WASM). "
                    "PDA derivation + borsh decode, zero JavaScript client code."
                }
            }

            div { style: "display:flex;gap:8px;margin:16px 0",
                input {
                    style: "flex:1;padding:10px;border:1px solid #d6d3d1;border-radius:8px;font-family:ui-monospace,monospace;font-size:13px",
                    value: "{owner_input}",
                    oninput: move |e| owner_input.set(e.value()),
                    placeholder: "Household owner wallet address",
                }
                button {
                    style: "padding:10px 16px;border:none;border-radius:8px;background:#059669;color:white;font-weight:600;cursor:pointer",
                    onclick: move |_| fetch(owner_input()),
                    "Load household"
                }
            }

            div { style: "display:flex;gap:8px;flex-wrap:wrap;margin-bottom:16px",
                for (addr, label) in DEMO_OWNERS {
                    button {
                        style: "padding:6px 10px;border:1px solid #d6d3d1;border-radius:6px;background:#fafaf9;font-size:12px;cursor:pointer",
                        onclick: move |_| {
                            owner_input.set(addr.to_string());
                            fetch(addr.to_string());
                        },
                        "{label}"
                    }
                }
            }

            match &*state.read() {
                FetchState::Idle => rsx! { p { style: "color:#a8a29e", "Enter an owner address (or pick a demo) and load." } },
                FetchState::Loading => rsx! { p { style: "color:#a8a29e", "Fetching from devnet…" } },
                FetchState::Failed(msg) => rsx! {
                    div { style: "padding:12px;border:1px solid #fecaca;border-radius:8px;background:#fef2f2;color:#b91c1c;font-size:13px",
                        "⚠ {msg}"
                    }
                },
                FetchState::Loaded(v) => rsx! { HouseholdCard { view: *v.clone() } },
            }

            h2 { style: "margin:28px 0 8px;font-size:16px", "🛒 Purchase ledger" }
            match &*ledger.read() {
                LedgerState::Idle => rsx! { p { style: "color:#a8a29e;font-size:13px", "Load a household to see its requests." } },
                LedgerState::Loading => rsx! { p { style: "color:#a8a29e;font-size:13px", "Reading the ledger from devnet…" } },
                LedgerState::Failed(msg) => rsx! {
                    div { style: "padding:12px;border:1px solid #fecaca;border-radius:8px;background:#fef2f2;color:#b91c1c;font-size:13px",
                        "⚠ {msg}"
                    }
                },
                LedgerState::Loaded(rows) if rows.is_empty() => rsx! {
                    p { style: "color:#a8a29e;font-size:13px", "No purchase requests for this household yet." }
                },
                LedgerState::Loaded(rows) => rsx! { LedgerTable { rows: rows.clone() } },
            }

            h2 { style: "margin:28px 0 8px;font-size:16px", "👛 Wallet — first signed tx from Rust" }
            div { style: "border:1px solid #d6d3d1;border-radius:12px;padding:16px;display:grid;gap:10px",
                p { style: "font-size:12px;color:#78716c;margin:0",
                    "Detects window.phantom (Wallet Standard), builds the memo tx wire bytes in Rust, and hands them to the wallet — the wallet fills the signature slot and submits."
                }
                match &*wallet_pubkey.read() {
                    Some(pk) => rsx! {
                        div { style: "display:flex;gap:8px;align-items:center;flex-wrap:wrap",
                            span { style: "padding:2px 10px;border-radius:9999px;background:#dcfce7;color:#166534;font-size:12px;font-weight:600", "connected" }
                            span { style: "font-family:ui-monospace,monospace;font-size:12px", "{pk}" }
                        }
                        button {
                            style: "padding:8px 14px;border:none;border-radius:8px;background:#059669;color:white;font-weight:600;cursor:pointer",
                            onclick: send_memo,
                            disabled: matches!(&*send_state.read(), SendState::Building),
                            if matches!(&*send_state.read(), SendState::Building) {
                                "Building & sending…"
                            } else {
                                "Send a GM memo tx (your wallet signs)"
                            }
                        }
                    },
                    None => rsx! {
                        button {
                            style: "padding:8px 14px;border:none;border-radius:8px;background:#8250df;color:white;font-weight:600;cursor:pointer",
                            onclick: connect_wallet,
                            disabled: *wallet_busy.read(),
                            if *wallet_busy.read() { "Connecting…" } else { "Connect Phantom" }
                        }
                    },
                }
                if let Some(err) = &*wallet_error.read() {
                    p { style: "font-size:12px;color:#b91c1c;margin:0", "⚠ {err}" }
                }
                match &*send_state.read() {
                    SendState::Idle => rsx! {
                        p { style: "font-size:12px;color:#a8a29e;margin:0",
                            "Nothing sent yet — the tx is a SPL Memo v1 with your key as fee payer."
                        }
                    },
                    SendState::Building => rsx! {
                        p { style: "font-size:12px;color:#a8a29e;margin:0", "Fetching blockhash & building wire bytes…" }
                    },
                    SendState::Failed(e) => rsx! {
                        p { style: "font-size:12px;color:#b91c1c;margin:0", "⚠ {e}" }
                    },
                    SendState::Sent(sig) => rsx! {
                        div { style: "padding:10px;border:1px solid #bbf7d0;border-radius:8px;background:#f0fdf4;display:grid;gap:6px",
                            span { style: "font-size:12px;color:#166534;font-weight:600", "Signed & submitted ✓" }
                            a {
                                href: "https://explorer.solana.com/tx/{sig}?cluster=devnet",
                                target: "_blank",
                                style: "font-size:12px;color:#1d4ed8",
                                "view on Solana Explorer ↗"
                            }
                            code { style: "font-size:10px;word-break:break-all;color:#78716c", "{sig}" }
                        }
                    },
                }
            }

            h2 { style: "margin:28px 0 8px;font-size:16px", "🤖 AI agent — same /api/ai as the Next.js app" }
            div { style: "border:1px solid #d6d3d1;border-radius:12px;padding:16px;display:grid;gap:10px",
                div {
                    style: "max-height:260px;overflow-y:auto;display:flex;flex-direction:column;gap:8px;padding:10px;border:1px solid #e7e5e4;border-radius:8px;background:#fafaf9",
                    if chat.len() == 0 {
                        p { style: "font-size:12px;color:#a8a29e;margin:0",
                            "Try “ซื้อนม 2 ขวด งบ 50 บาท” or “งบเหลือเท่าไหร่” — proposals list items; answers use the live household numbers loaded above."
                        }
                    }
                    for msg in chat() {
                        div {
                            class: if msg.role == "user" { "align-self:flex-end" } else { "align-self:flex-start" },
                            style: if msg.role == "user" {
                                "max-width:85%;padding:8px 12px;border-radius:12px 12px 2px 12px;background:#059669;color:white;font-size:13px"
                            } else {
                                "max-width:90%;padding:8px 12px;border-radius:12px 12px 12px 2px;background:#e7e5e4;color:#1c1917;font-size:13px"
                            },
                            "{msg.text}"
                            if !msg.items.is_empty() {
                                div { style: "display:flex;flex-wrap:wrap;gap:6px;margin-top:8px",
                                    for item in msg.items {
                                        span {
                                            style: "padding:2px 8px;border-radius:6px;background:#f5f5f4;color:#44403c;font-size:11px;border:1px solid #d6d3d1",
                                            "{item.label()}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if chat_busy() {
                        div { style: "align-self:flex-start;font-size:12px;color:#a8a29e", "Thinking…" }
                    }
                }
                div { style: "display:flex;gap:8px",
                    input {
                        style: "flex:1;padding:10px;border:1px solid #d6d3d1;border-radius:8px;font-size:13px",
                        value: "{chat_input}",
                        oninput: move |e| chat_input.set(e.value()),
                        placeholder: "Ask the agent… (Thai/English)",
                        disabled: chat_busy(),
                    }
                    button {
                        style: "padding:10px 16px;border:none;border-radius:8px;background:#8250df;color:white;font-weight:600;cursor:pointer",
                        onclick: send_chat,
                        disabled: chat_busy() || chat_input().trim().is_empty(),
                        "Send"
                    }
                }
            }

            h2 { style: "margin:28px 0 8px;font-size:16px", "🛠 Actions — household ops as Rust wire bytes" }
            div { style: "border:1px solid #d6d3d1;border-radius:12px;padding:16px;display:grid;gap:12px",
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "flex:1;min-width:200px",
                        label { style: "font-size:11px;color:#78716c", "Household name" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid #d6d3d1;border-radius:8px;font-size:13px",
                            value: "{name_input}",
                            oninput: move |e| name_input.set(e.value()),
                            placeholder: "บ้านของเรา",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:#0f766e;color:white;font-weight:600;cursor:pointer",
                        onclick: init_household,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Initialize household"
                    }
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "flex:1;min-width:140px",
                        label { style: "font-size:11px;color:#78716c", "Deposit (SOL)" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid #d6d3d1;border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{deposit_input}",
                            oninput: move |e| deposit_input.set(e.value()),
                            placeholder: "0.1",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:#0f766e;color:white;font-weight:600;cursor:pointer",
                        onclick: deposit,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Deposit"
                    }
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "flex:2;min-width:180px",
                        label { style: "font-size:11px;color:#78716c", "Item to buy" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid #d6d3d1;border-radius:8px;font-size:13px",
                            value: "{item_input}",
                            oninput: move |e| item_input.set(e.value()),
                            placeholder: "นมสด 1 ลิตร ×2",
                        }
                    }
                    div { style: "width:120px",
                        label { style: "font-size:11px;color:#78716c", "Limit (SOL)" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid #d6d3d1;border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{req_amount_input}",
                            oninput: move |e| req_amount_input.set(e.value()),
                            placeholder: "0.02",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:#0f766e;color:white;font-weight:600;cursor:pointer",
                        onclick: open_request,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Open request"
                    }
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "width:120px",
                        label { style: "font-size:11px;color:#78716c", "Request #" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid #d6d3d1;border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{approve_id_input}",
                            oninput: move |e| approve_id_input.set(e.value()),
                            placeholder: "1",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:#0f766e;color:white;font-weight:600;cursor:pointer",
                        onclick: approve,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Approve"
                    }
                }
                if !action_msg().is_empty() {
                    p { style: "font-size:12px;margin:0;color:if action_msg().starts_with('✓') { \"#166534\" } else { \"#b91c1c\" }", "{action_msg}" }
                }
                p { style: "font-size:11px;color:#a8a29e;margin:0",
                    "Note: create with buyer = you, then approve — the program forbids self-approval, so use a second wallet for the buyer in a real household."
                }
            }

            footer { style: "margin-top:40px;color:#a8a29e;font-size:11px;border-top:1px solid #e7e5e4;padding-top:12px",
                "Privacy note: the household name never lands on-chain — the contract stores a blake3 fingerprint only, decoded here as hex. "
                "Program: " {PROGRAM_ID}
            }
        }
    }
}

#[component]
fn HouseholdCard(view: HouseholdView) -> Element {
    rsx! {
        div { style: "border:1px solid #d6d3d1;border-radius:12px;padding:20px;display:grid;gap:10px",
            h2 { style: "margin:0;font-size:18px", "Household " span { style: "color:#059669", "✓ live on devnet" } }
            Row { label: "Household PDA".to_string(), value: view.pda.clone() }
            Row { label: "Vault balance".to_string(), value: format!("{} SOL (account lamports)", view.lamports_sol) }
            Row { label: "Mirror field vault_balance".to_string(), value: format!("{} lamports", view.vault_balance_mirror) }
            Row { label: "Owner".to_string(), value: view.owner.clone() }
            Row { label: "Members".to_string(), value: view.member_count.to_string() }
            Row { label: "Purchase requests opened".to_string(), value: view.request_counter.to_string() }
            Row { label: "Rewards distributed".to_string(), value: format!("{} pts", view.total_rewards) }
            Row { label: "Created at slot".to_string(), value: view.created_slot.to_string() }
            Row { label: "Name fingerprint (blake3, hex)".to_string(), value: view.name_hash_hex.clone() }
        }
    }
}

#[component]
fn Row(label: String, value: String) -> Element {
    rsx! {
        div { style: "display:flex;justify-content:space-between;gap:12px;font-size:13px",
            span { style: "color:#78716c", "{label}" }
            span { style: "font-family:ui-monospace,monospace;word-break:break-all;text-align:right", "{value}" }
        }
    }
}

fn status_style(status: &str) -> &'static str {
    match status {
        "waiting" => "background:#fef3c7;color:#92400e",
        "approved" => "background:#dbeafe;color:#1e40af",
        "restocked" => "background:#ede9fe;color:#5b21b6",
        "reimbursed" => "background:#dcfce7;color:#166534",
        "rejected" => "background:#fee2e2;color:#991b1b",
        _ => "background:#f5f5f4;color:#57534e",
    }
}

#[component]
fn LedgerTable(rows: Vec<RequestView>) -> Element {
    rsx! {
        div { style: "border:1px solid #d6d3d1;border-radius:12px;overflow:hidden",
            table { style: "width:100%;border-collapse:collapse;font-size:12px",
                thead {
                    tr { style: "background:#fafaf9;text-align:left",
                        th { style: "padding:8px 10px", "#" }
                        th { style: "padding:8px 10px", "Status" }
                        th { style: "padding:8px 10px", "Spending limit" }
                        th { style: "padding:8px 10px", "Buyer" }
                        th { style: "padding:8px 10px", "Rewards" }
                        th { style: "padding:8px 10px", "Created" }
                    }
                }
                tbody {
                    for row in rows {
                        tr { style: "border-top:1px solid #e7e5e4",
                            td { style: "padding:8px 10px;font-family:ui-monospace,monospace", "#{row.request_id}" }
                            td { style: "padding:8px 10px",
                                span {
                                    style: "padding:2px 8px;border-radius:9999px;font-size:11px;font-weight:600 {status_style(row.status)}",
                                    "{row.status}"
                                }
                            }
                            td { style: "padding:8px 10px;font-family:ui-monospace,monospace", "{row.amount_sol} SOL" }
                            td { style: "padding:8px 10px;font-family:ui-monospace,monospace", title: "{row.buyer}", "{row.buyer}" }
                            td { style: "padding:8px 10px", "{row.reward_earned} pts" }
                            td { style: "padding:8px 10px;color:#78716c", "slot {row.created_slot}" }
                        }
                    }
                }
            }
        }
    }
}

/// Derive the PDA, fetch the account, decode the borsh layout.
async fn load_household(owner_raw: &str) -> Result<HouseholdView, String> {
    let owner = Pubkey::from_str(owner_raw)
        .map_err(|_| format!("“{owner_raw}” is not a valid Solana address"))?;
    let program = Pubkey::from_str(PROGRAM_ID).expect("constant parses");

    let (pda, bump) = Pubkey::find_program_address(&[b"household", owner.as_ref()], &program);
    let account = rpc_get_account(&pda).await?;

    // rpc_get_account already unwrapped the JSON-RPC envelope: this IS the
    // getAccountInfo result = { context, value: account | null }
    let account = account
        .get("value")
        .filter(|v| !v.is_null())
        .ok_or("household not initialized for this owner (no account)")?;
    let Some(data) = account.get("data") else {
        return Err("household not initialized for this owner (no account)".into());
    };
    // getAccountInfo with base64 encoding returns [data, encoding]
    let encoded = data
        .as_array()
        .and_then(|d| d.first())
        .and_then(|s| s.as_str())
        .ok_or("malformed RPC response")?;
    let bytes = B64
        .decode(encoded)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    if bytes.len() < 109 {
        return Err(format!(
            "account too short for a Household ({len} bytes) — wrong owner?",
            len = bytes.len()
        ));
    }
    let u32_at = |off: usize| u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
    let u64_at = |off: usize| u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
    let lamports = account
        .get("lamports")
        .and_then(|v| v.as_u64())
        .ok_or("missing lamports")?;

    Ok(HouseholdView {
        pda: format!("{pda} (bump {bump})"),
        pda_key: pda,
        lamports_sol: format!("{:.6}", lamports as f64 / 1e9),
        owner: Pubkey::try_from(&bytes[8..40])
            .map(|p| p.to_string())
            .unwrap_or_else(|_| "decode error".into()),
        name_hash_hex: hex(&bytes[40..72]),
        member_count: u32_at(73),
        request_counter: u64_at(77),
        total_rewards: u64_at(85),
        vault_balance_mirror: u64_at(93),
        created_slot: u64_at(101),
    })
}

/// Anchor's sha256("account:PurchaseRequest")[..8] discriminator — lets a
/// `getProgramAccounts` memcmp filter skip Member/household accounts.
fn purchase_discriminator() -> [u8; 8] {
    let hash = Sha256::digest(b"account:PurchaseRequest");
    hash[..8].try_into().expect("8-byte slice")
}

fn status_name(tag: u8) -> &'static str {
    // programs/stocksie/src/types.rs — plain u8-tag enum, no payloads
    match tag {
        0 => "waiting",
        1 => "approved",
        2 => "restocked",
        3 => "reimbursed",
        4 => "rejected",
        _ => "unknown",
    }
}

/// Fetch every `PurchaseRequest` of one household:
/// getProgramAccounts filtered by discriminator @0 and household @8.
async fn load_ledger(household: &Pubkey) -> Result<Vec<RequestView>, String> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [
            PROGRAM_ID,
            {
                "encoding": "base64",
                "commitment": "confirmed",
                "filters": [
                    {"memcmp": {"offset": 0, "bytes": bs58::encode(purchase_discriminator()).into_string()}},
                    {"memcmp": {"offset": 8, "bytes": household.to_string()}}
                ]
            }
        ]
    });
    let result = rpc_call(body).await?;
    let accounts = result
        .as_array()
        .ok_or("unexpected getProgramAccounts response shape")?;

    let mut rows = Vec::with_capacity(accounts.len());
    for entry in accounts {
        let account = entry
            .get("account")
            .ok_or("missing account in getProgramAccounts entry")?;
        let encoded = account
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|d| d.first())
            .and_then(|s| s.as_str())
            .ok_or("malformed account data")?;
        let bytes = B64.decode(encoded).map_err(|e| format!("base64 decode failed: {e}"))?;
        // Layout (programs/stocksie/src/state/purchase_request.rs): 8-byte
        // discriminator, then household@8, buyer@40, id@72, amount@80,
        // item_hash@88, unit_cost_hash@120, status@152 (u8), approved_by@153,
        // approved_slot@185, restocked_slot@193, reimbursed@201, reward@209,
        // bump@217, created_slot@218. Total 226.
        if bytes.len() < 226 {
            continue; // short/legacy account — skip rather than misread
        }
        let u64_at = |off: usize| u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
        rows.push(RequestView {
            pubkey: entry
                .get("pubkey")
                .and_then(|p| p.as_str())
                .unwrap_or("?")
                .to_string(),
            request_id: u64_at(72),
            amount_sol: format!("{:.6}", u64_at(80) as f64 / 1e9),
            status: status_name(bytes[152]),
            buyer: Pubkey::try_from(&bytes[40..72])
                .map(|p| {
                    let s = p.to_string();
                    format!("{}…{}", &s[..6], &s[s.len() - 4..])
                })
                .unwrap_or_else(|_| "decode error".into()),
            reward_earned: u64_at(209),
            created_slot: u64_at(218),
        });
    }
    rows.sort_by_key(|r| r.request_id);
    Ok(rows)
}

/// Generic JSON-RPC POST against the devnet endpoint, unwrapping the
/// `result` envelope (or surfacing the error object verbatim).
async fn rpc_call(body: serde_json::Value) -> Result<serde_json::Value, String> {
    let result = post_json(RPC_URL, body).await?;
    if let Some(err) = result.get("error") {
        return Err(format!("devnet RPC error: {err}"));
    }
    result.get("result").cloned().ok_or_else(|| {
        "unexpected RPC response (missing result envelope)".to_string()
    })
}

/// POST a JSON body anywhere and parse the JSON reply.
async fn post_json(url: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let resp = Request::post(url)
        .json(&body)
        .map_err(|e| format!("body encode failed: {e}"))?
        .send()
        .await
        .map_err(|e| format!("request to {url} failed: {e}"))?;
    if resp.status() != 200 {
        return Err(format!("{} returned HTTP {}", url, resp.status()));
    }
    let text = resp.text().await.map_err(|e| format!("read failed: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("bad JSON from {url}: {e}"))
}

/// getAccountInfo with base64 encoding — returns the full result envelope
/// (`{ context, value: account | null }`), NOT the bare account.
async fn rpc_get_account(pubkey: &Pubkey) -> Result<serde_json::Value, String> {
    rpc_call(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [pubkey.to_string(), {"encoding": "base64", "commitment": "confirmed"}]
    }))
    .await
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Native E2E: execute the real instructions against a running localnet.
//
// This is the correctness proof for the hand-built wire format — the test
// signs with a throwaway ed25519 key and sends initialize_household →
// deposit_funds → create_purchase_request → approve_purchase_request to the
// deployed program, then asserts the decoded on-chain state after each step.
// Skips (with a note) when no localnet is reachable, so `cargo test` works
// offline.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod e2e {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const LOCALNET: &str = "http://127.0.0.1:8899";
    /// The localnet program keypair (regenerated when target/ was cleaned —
    /// devnet keeps At2vd… deployed; this file is local-only).
    const LOCAL_KEYPAIR: &str = "../target/deploy/stocksie-keypair.json";

    fn localnet_alive() -> bool {
        std::thread::scope(|s| {
            let h = std::thread::spawn(|| {
                ureq::AgentBuilder::new()
                    .timeout(std::time::Duration::from_millis(1500))
                    .build()
                    .post(LOCALNET)
                    .send_json(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"getHealth"}))
            });
            matches!(h.join(), Ok(Ok(_)))
        })
    }

    fn rpc(params_url: &str, value: serde_json::Value) -> serde_json::Value {
        let res = ureq::post(params_url)
            .send_json(value)
            .expect("localnet RPC reachable");
        res.into_json().expect("RPC JSON")
    }

    fn send_and_confirm(wallet: &SigningKey, wire: &[u8]) -> String {
        let sig: ed25519_dalek::Signature = wallet.sign(&wire[65..]);
        let mut signed = wire.to_vec();
        signed[1..65].copy_from_slice(&sig.to_bytes());
        let res = rpc(
            LOCALNET,
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "sendTransaction",
                "params": [B64.encode(&signed), {"encoding": "base64", "preflightCommitment": "confirmed"}]
            }),
        );
        if let Some(err) = res.get("error") {
            panic!("sendTransaction rejected: {err}");
        }
        let sig_str = res["result"].as_str().expect("signature").to_string();
        for _ in 0..80 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let st = rpc(
                LOCALNET,
                json!({"jsonrpc":"2.0","id":1,"method":"getSignatureStatuses","params":[[sig_str.clone()],{"searchTransactionHistory":false}]}),
            );
            if let Some(s) = st.get("result").and_then(|r| r.get("value")).and_then(|v| v.get(0)) {
                if !s.is_null() {
                    if s.get("err").map(|e| !e.is_null()).unwrap_or(false) {
                        panic!("tx failed on-chain: {}", s["err"]);
                    }
                    let confirmed = s.get("confirmationStatus")
                        .and_then(|c| c.as_str())
                        .map(|c| c == "confirmed" || c == "finalized")
                        .unwrap_or(false);
                    if confirmed {
                        return sig_str;
                    }
                }
            }
        }
        panic!("confirmation timeout");
    }

    fn account(program: &Pubkey, owner: &Pubkey) -> Option<serde_json::Value> {
        let (hh, _) = programs::household_pda(owner, program);
        let res = rpc(
            LOCALNET,
            json!({"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":[hh.to_string(), {"encoding":"base64","commitment":"confirmed"}]}),
        );
        let v = res.get("result")?.get("value")?.clone();
        if v.is_null() { None } else { Some(v) }
    }

    fn decode(bytes_base64: &str) -> Vec<u8> {
        B64.decode(bytes_base64).expect("account base64")
    }

    #[test]
    fn household_lifecycle_executes_on_localnet() {
        if !localnet_alive() {
            eprintln!("skipping e2e — no localnet on {LOCALNET}");
            return;
        }
        // program id from the local keypair file (regenerated per anchor sync)
        let kp_raw: Vec<u8> = serde_json::from_str(
            &std::fs::read_to_string(LOCAL_KEYPAIR).expect("local program keypair"),
        )
        .expect("keypair JSON");
        let program = Pubkey::try_from(&kp_raw[32..64]).expect("program pubkey");

        // fresh wallet per run → fresh household PDA
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let wallet = SigningKey::from_bytes(&programs::text_hash(&nanos.to_string()));
        let owner = Pubkey::from(wallet.verifying_key().to_bytes());

        // fund the wallet
        rpc(
            LOCALNET,
            json!({"jsonrpc":"2.0","id":1,"method":"requestAirdrop","params":[owner.to_string(), 2_000_000_000u64]}),
        );
        let mut funded = false;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let bal = rpc(
                LOCALNET,
                json!({"jsonrpc":"2.0","id":1,"method":"getBalance","params":[owner.to_string()]}),
            );
            if bal["result"]["value"].as_u64().unwrap_or(0) >= 1_000_000_000 {
                funded = true;
                break;
            }
        }
        assert!(funded, "airdrop did not land");

        // real recent blockhash — the chain rejects zero/unknown hashes
        let latest = rpc(
            LOCALNET,
            json!({"jsonrpc":"2.0","id":1,"method":"getLatestBlockhash","params":[{"commitment":"confirmed"}]}),
        );
        let blockhash_str = latest["result"]["value"]["blockhash"]
            .as_str()
            .expect("blockhash")
            .to_string();
        let blockhash: [u8; 32] = bs58::decode(&blockhash_str)
            .into_vec()
            .expect("blockhash b58")
            .try_into()
            .expect("32 bytes");

        // 1. initialize_household
        let name_hash = programs::text_hash("บ้าน RIIR E2E");
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::initialize_household(&owner, name_hash, &program)],
        );
        send_and_confirm(&wallet, &wire);
        let acc = account(&program, &owner).expect("household exists after init");
        let data = decode(acc["data"][0].as_str().expect("b64"));
        assert_eq!(u32::from_le_bytes(data[73..77].try_into().unwrap()), 1);

        // 2. deposit_funds 0.1 SOL
        let lamports_before = acc["lamports"].as_u64().unwrap();
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::deposit_funds(&owner, 100_000_000, &program)],
        );
        send_and_confirm(&wallet, &wire);
        let acc = account(&program, &owner).expect("household after deposit");
        let data = decode(acc["data"][0].as_str().expect("b64"));
        assert_eq!(u64::from_le_bytes(data[93..101].try_into().unwrap()), 100_000_000);
        assert!(
            acc["lamports"].as_u64().unwrap() >= lamports_before + 100_000_000,
            "vault lamports must grow by the deposit"
        );

        // 3a. add the buyer as a Child member (buyers must already be members)
        let buyer_wallet = SigningKey::from_bytes(&programs::text_hash(&format!("{nanos}-buyer")));
        let buyer = Pubkey::from(buyer_wallet.verifying_key().to_bytes());
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::add_member(&owner, &buyer, 2, &program)], // role: Child
        );
        send_and_confirm(&wallet, &wire);

        // 3b. create_purchase_request (buyer ≠ approver — self-approval is
        // forbidden by the program)
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::create_purchase_request(
                &owner,
                &buyer,
                1, // first request id
                10_000_000,
                programs::text_hash("นม 2 ขวด"),
                programs::text_hash("฿50 at the corner store"),
                &program,
            )],
        );
        send_and_confirm(&wallet, &wire);
        let acc = account(&program, &owner).expect("household after request");
        let data = decode(acc["data"][0].as_str().expect("b64"));
        assert_eq!(u64::from_le_bytes(data[77..85].try_into().unwrap()), 1);

        // 4. approve_purchase_request
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::approve_purchase_request(&owner, 1, &program)],
        );
        send_and_confirm(&wallet, &wire);
        let (hh, _) = programs::household_pda(&owner, &program);
        let req = programs::purchase_pda(&hh, 1, &program);
        let res = rpc(
            LOCALNET,
            json!({"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":[req.to_string(), {"encoding":"base64","commitment":"confirmed"}]}),
        );
        let req_data = decode(res["result"]["value"]["data"][0].as_str().expect("b64"));
        assert_eq!(req_data[152], 1, "status must be Approved (1)");
        println!("E2E OK — full household lifecycle executed from Rust wire bytes");
    }
}
