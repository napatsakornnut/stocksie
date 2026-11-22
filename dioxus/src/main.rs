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
const AI_API_URL: &str = "https://stocksie-th.vercel.app/api/ai";

/// Devnet JSON-RPC endpoint (the prototype targets devnet like the hosted app).
const RPC_URL: &str = "https://api.devnet.solana.com";

/// Program guards mirrored from lib/constants.rs (receipt → request conversion).
const MIN_REQUEST_LAMPORTS: u64 = 100_000;
const MAX_REQUEST_LAMPORTS: u64 = 500_000_000;
/// Fallback THB/SOL when CoinGecko is unreachable (same default as the Next app).
const DEFAULT_SOL_THB: f64 = 5600.0;

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
    amount_lamports: u64,
    status: &'static str,
    buyer: String,
    buyer_key: Pubkey,
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
enum ViewMode {
    Home,
    Lists,
    Assistant,
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

    fn price_label(&self) -> String {
        format!("฿{:.2}", self.total_minor as f64 / 100.0)
    }
}

/// Send one chat turn: user bubble → /api/ai → assistant bubble (with item
/// chips on propose_purchases) → scroll the conversation into view.
async fn send_chat_msg(
    mut chat: Signal<Vec<ChatMsg>>,
    mut chat_input: Signal<String>,
    mut chat_busy: Signal<bool>,
    state: Signal<FetchState>,
    ledger: Signal<LedgerState>,
) {
    let text = chat_input().trim().to_string();
    if text.is_empty() || chat_busy() {
        return;
    }
    chat_input.set(String::new());
    chat.push(ChatMsg { role: "user", text: text.clone(), items: vec![] });
    chat_busy.set(true);
    scroll_chat_box();

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
    scroll_chat_box();
}

/// Scroll the conversation box to the latest message.
fn scroll_chat_box() {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("chat-box"))
    {
        use wasm_bindgen::JsCast;
        if let Ok(html) = el.dyn_into::<web_sys::HtmlElement>() {
            html.set_scroll_top(html.scroll_height());
        }
    }
}
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

/// Read the picked file from a hidden file input (web-sys; the Dioxus file
/// API moved between versions — this is version-proof).
async fn read_receipt_file(input_id: &str) -> Result<(Vec<u8>, String), String> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("no window")?;
    let doc = window.document().ok_or("no document")?;
    let input = doc
        .get_element_by_id(input_id)
        .ok_or("file input missing")?
        .dyn_into::<web_sys::HtmlInputElement>()
        .map_err(|_| "not an input element")?;
    let file = input
        .files()
        .and_then(|fl| fl.get(0))
        .ok_or("pick an image first")?;
    let mime = file.type_();
    let promise = file.array_buffer();
    let ab = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("read failed: {e:?}"))?;
    Ok((js_sys::Uint8Array::new(&ab).to_vec(), mime))
}

/// Household PDA + the CURRENT on-chain request counter — each receipt item
/// creates a real request, so the counter must be re-read per send (never
/// trust a cached one for back-to-back creates).
async fn fetch_household_state(owner: &Pubkey) -> Result<(Pubkey, u64), String> {
    let (household, _) = programs::household_pda(owner, &programs::program_id());
    let res = rpc_call(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [household.to_string(), {"encoding": "base64", "commitment": "confirmed"}]
    }))
    .await?;
    let acc = res
        .get("value")
        .filter(|v| !v.is_null())
        .ok_or("household not initialized for this wallet")?;
    let data_b64 = acc
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|d| d.first())
        .and_then(|s| s.as_str())
        .ok_or("malformed account")?;
    let bytes = B64.decode(data_b64).map_err(|e| format!("base64: {e}"))?;
    if bytes.len() < 85 {
        return Err("account too short".into());
    }
    Ok((household, u64::from_le_bytes(bytes[77..85].try_into().unwrap())))
}

/// Theme tokens + dark/light via prefers-color-scheme. Rendered as a <style>
/// element (the CSS braces can't live inline in rsx text — it's format-parsed).
const THEME_CSS: &str = r#"
    :root {
        --bg: #fafaf9; --surface: #ffffff; --surface-2: #f5f5f4; --surface-3: #fafaf9;
        --border: #e7e5e4; --border-strong: #d6d3d1;
        --text: #1c1917; --muted: #78716c; --faint: #a8a29e; --ink-soft: #44403c; --chip-ink-muted: #57534e;
        --accent: #059669; --teal: #0f766e; --violet: #8250df;
        --danger: #b91c1c; --danger-bg: #fef2f2; --danger-border: #fecaca; --danger-ink: #991b1b;
        --success: #166534; --success-bg: #f0fdf4; --success-border: #bbf7d0;
        --link: #1d4ed8;
        --warn-bg: #fef3c7; --warn-ink: #92400e;
        --info-bg: #dbeafe; --info-ink: #1e40af;
        --violet-bg: #ede9fe; --violet-ink: #5b21b6;
    }
    @media (prefers-color-scheme: dark) {
        :root {
            --bg: #0c0a09; --surface: #1c1917; --surface-2: #292524; --surface-3: #1c1917;
            --border: #44403c; --border-strong: #57534e;
            --text: #f5f5f4; --muted: #a8a29e; --faint: #78716c; --ink-soft: #d6d3d1; --chip-ink-muted: #d6d3d1;
            --accent: #10b981; --teal: #0d9488; --violet: #a78bfa;
            --danger: #f87171; --danger-bg: #2c0a0a; --danger-border: #7f1d1d; --danger-ink: #fca5a5;
            --success: #34d399; --success-bg: #052e16; --success-border: #14532d;
            --link: #93c5fd;
            --warn-bg: #451a03; --warn-ink: #fbbf24;
            --info-bg: #172554; --info-ink: #93c5fd;
            --violet-bg: #2e1065; --violet-ink: #c4b5fd;
        }
    }
    html { background: var(--bg); }
    body {
        margin: 0;
        background: var(--bg);
        color: var(--text);
        font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
        -webkit-font-smoothing: antialiased;
    }
    button { font-family: inherit; }
    button[disabled] { opacity: 0.45; cursor: not-allowed; }
    input, select, textarea { color: var(--text); background: var(--surface); border-color: var(--border-strong); }
    input::placeholder { color: var(--faint); }
    a { color: var(--link); }

    /* manual theme override — html[data-theme] beats the system media query */
    [data-theme="dark"] {
        --bg: #0c0a09; --surface: #1c1917; --surface-2: #292524; --surface-3: #1c1917;
        --border: #44403c; --border-strong: #57534e;
        --text: #f5f5f4; --muted: #a8a29e; --faint: #78716c; --ink-soft: #d6d3d1; --chip-ink-muted: #d6d3d1;
        --accent: #10b981; --teal: #0d9488; --violet: #a78bfa;
        --danger: #f87171; --danger-bg: #2c0a0a; --danger-border: #7f1d1d; --danger-ink: #fca5a5;
        --success: #34d399; --success-bg: #052e16; --success-border: #14532d;
        --link: #93c5fd;
        --warn-bg: #451a03; --warn-ink: #fbbf24;
        --info-bg: #172554; --info-ink: #93c5fd;
        --violet-bg: #2e1065; --violet-ink: #c4b5fd;
    }
    [data-theme="light"] {
        --bg: #fafaf9; --surface: #ffffff; --surface-2: #f5f5f4; --surface-3: #fafaf9;
        --border: #e7e5e4; --border-strong: #d6d3d1;
        --text: #1c1917; --muted: #78716c; --faint: #a8a29e; --ink-soft: #44403c; --chip-ink-muted: #57534e;
        --accent: #059669; --teal: #0f766e; --violet: #8250df;
        --danger: #b91c1c; --danger-bg: #fef2f2; --danger-border: #fecaca; --danger-ink: #991b1b;
        --success: #166534; --success-bg: #f0fdf4; --success-border: #bbf7d0;
        --link: #1d4ed8;
        --warn-bg: #fef3c7; --warn-ink: #92400e;
        --info-bg: #dbeafe; --info-ink: #1e40af;
        --violet-bg: #ede9fe; --violet-ink: #5b21b6;
    }

    /* sticky section nav */
    .topnav {
        position: sticky; top: 0; z-index: 20;
        display: flex; gap: 4px; flex-wrap: wrap;
        padding: 8px 0; margin-bottom: 4px;
        background: var(--bg);
        border-bottom: 1px solid var(--border);
    }
    .topnav a {
        font-size: 12px; color: var(--muted); text-decoration: none;
        padding: 4px 10px; border-radius: 6px;
    }
    .topnav a:hover { color: var(--text); background: var(--surface-2); }
    [id] { scroll-margin-top: 72px; }

    /* result banners */
    .banner { padding: 10px 12px; border-radius: 8px; font-size: 12px; margin: 0; }
    .banner-ok {
        border: 1px solid var(--success-border); background: var(--success-bg); color: var(--success);
    }
    .banner-err {
        border: 1px solid var(--danger-border); background: var(--danger-bg); color: var(--danger);
    }

    /* copy buttons */
    .copybtn {
        font-size: 10px; padding: 2px 8px;
        border: 1px solid var(--border-strong); border-radius: 6px;
        background: var(--surface-2); color: var(--muted); cursor: pointer;
    }
    .copybtn:hover { color: var(--text); }

    .tabbar {
        position: fixed; bottom: 0; left: 0; right: 0; z-index: 30;
        display: flex; background: var(--surface); border-top: 1px solid var(--border);
    }
    .tabbar button {
        flex: 1; padding: 10px 0 8px; border: none; background: none;
        color: var(--muted); font-size: 11px; cursor: pointer; white-space: pre-line;
    }
    .tabbar button.active { color: var(--accent); font-weight: 700; }
    body { padding-bottom: 64px; }
    #chat-box { max-height: 260px; overflow-y: auto; display: flex; flex-direction: column; gap: 8px;
        padding: 10px; border: 1px solid var(--border); border-radius: 8px;
        background: var(--surface-3); }
"#;

/// Execute + report into the actions message line (shared by all handlers).
async fn execute_and_report(
    mut action_msg: Signal<String>,
    payer: &Pubkey,
    label: &str,
    ixs: Vec<tx::ProgramIx>,
) -> bool {
    match execute(payer, ixs).await {
        Ok(sig) => {
            action_msg.set(format!("✓ {label} — {}", &sig[..16.min(sig.len())]));
            true
        }
        Err(e) => {
            action_msg.set(format!("⚠ {label}: {e}"));
            false
        }
    }
}

// ---------------------------------------------------------------------------
// โหมดบ้าน (Slice 1) — member list, on-device contact book, friendly labels
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct MemberView {
    wallet: Pubkey,
    role_tag: u8, // 0 Owner, 1 Parent, 2 Child, 3 Guest
    reward_points: u64,
}

fn member_discriminator() -> [u8; 8] {
    Sha256::digest(b"account:Member")[..8].try_into().expect("8 bytes")
}

/// All ACTIVE members of one household (getProgramAccounts filtered by the
/// Member discriminator + household @8).
async fn fetch_members(household: &Pubkey) -> Result<Vec<MemberView>, String> {
    let res = rpc_call(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [
            PROGRAM_ID,
            {
                "encoding": "base64",
                "commitment": "confirmed",
                "filters": [
                    {"memcmp": {"offset": 0, "bytes": bs58::encode(member_discriminator()).into_string()}},
                    {"memcmp": {"offset": 8, "bytes": household.to_string()}}
                ]
            }
        ]
    }))
    .await?;
    let accounts = res
        .as_array()
        .ok_or("unexpected getProgramAccounts response shape")?;
    let mut members = Vec::new();
    for entry in accounts {
        let Some(account) = entry.get("account") else { continue };
        let Some(encoded) = account
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|d| d.first())
            .and_then(|s| s.as_str())
        else {
            continue;
        };
        let bytes = B64.decode(encoded).map_err(|e| format!("base64: {e}"))?;
        if bytes.len() < 91 || bytes[81] == 0 {
            continue; // short account or soft-removed member
        }
        members.push(MemberView {
            wallet: Pubkey::try_from(&bytes[40..72]).unwrap_or_default(),
            role_tag: bytes[72],
            reward_points: u64::from_le_bytes(bytes[73..81].try_into().unwrap()),
        });
    }
    Ok(members)
}

fn role_name(tag: u8) -> &'static str {
    match tag {
        0 => "เจ้าบ้าน",
        1 => "ผู้ปกครอง",
        2 => "ลูก",
        _ => "แขก",
    }
}

fn role_emoji(tag: u8) -> &'static str {
    match tag {
        0 => "👑",
        1 => "🧑",
        2 => "🧒",
        _ => "👤",
    }
}

/// หนึ่งรายการในสมุดรายชื่อครอบครัว — จำชื่อเล่นของแต่ละ wallet ไว้ในเครื่อง
/// (ไม่ลง chain — ความเป็นส่วนตัวเหมือนเดิม)
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Contact {
    pk: String,
    name: String,
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

fn load_contacts(hh: &str) -> Vec<Contact> {
    storage()
        .and_then(|s| s.get_item(&format!("contacts:{hh}")).ok().flatten())
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

fn save_contacts(hh: &str, contacts: &[Contact]) {
    if let Some(s) = storage() {
        let _ = s.set_item(&format!("contacts:{hh}"), &serde_json::to_string(contacts).unwrap_or_default());
    }
}

/// ข้อความของแต่ละคำขอ (จำในเครื่อง — บน chain เก็บ blake3 เท่านั้น)
fn load_item_texts(hh: &str) -> Vec<(u64, String)> {
    storage()
        .and_then(|s| s.get_item(&format!("items:{hh}")).ok().flatten())
        .and_then(|v| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&v).ok())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.parse::<u64>().ok()?, v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn save_item_text(hh: &str, id: u64, text: &str) {
    if let Some(s) = storage() {
        let mut items: serde_json::Map<String, serde_json::Value> = s
            .get_item(&format!("items:{hh}"))
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        items.insert(id.to_string(), serde_json::json!(text));
        let _ = s.set_item(&format!("items:{hh}"), &serde_json::to_string(&items).unwrap_or_default());
    }
}

/// Bundle of page signals shared by every action (all Copy — cheap to pass).
#[derive(Clone, Copy)]
struct Ctx {
    action_msg: Signal<String>,
    action_busy: Signal<bool>,
    state: Signal<FetchState>,
    ledger: Signal<LedgerState>,
    contacts: Signal<Vec<Contact>>,
    members: Signal<Vec<MemberView>>,
    item_texts: Signal<Vec<(u64, String)>>,
}

impl Ctx {
    /// Reload household + members + ledger + on-device caches for one owner.
    async fn refresh(&self, owner_raw: String) {
        let mut state = self.state;
        let mut ledger = self.ledger;
        let mut contacts = self.contacts;
        let mut members = self.members;
        let mut item_texts = self.item_texts;
        let owner_raw = owner_raw.trim().to_string();
        state.set(FetchState::Loading);
        ledger.set(LedgerState::Idle);
        match load_household(&owner_raw).await {
            Ok(v) => {
                let key = v.pda_key;
                state.set(FetchState::Loaded(Box::new(v)));
                // members + on-device caches (contacts, item texts)
                members.set(fetch_members(&key).await.unwrap_or_default());
                contacts.set(load_contacts(&key.to_string()));
                item_texts.set(load_item_texts(&key.to_string()));
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

    /// อนุมัติ (Owner/Parent) — ปุ่มบนบัตรคำขอในโหมดบ้าน
    async fn do_approve(&self, admin: &Pubkey, caller: &Pubkey, request_id: u64) -> bool {
        let action_msg = self.action_msg;
        let ixs = vec![programs::approve_purchase_request(
            admin, caller, request_id, &programs::program_id(),
        )];
        let ok = execute_and_report(action_msg, caller, "อนุมัติแล้ว", ixs).await;
        if ok {
            self.refresh(admin.to_string()).await;
        }
        ok
    }

    /// ไม่อนุมัติ — reason เก็บเป็น blake3 เหมือนเดิม
    async fn do_reject(&self, admin: &Pubkey, caller: &Pubkey, request_id: u64, reason: &str) -> bool {
        let action_msg = self.action_msg;
        let ixs = vec![programs::reject_purchase_request(
            admin,
            caller,
            request_id,
            programs::text_hash(reason),
            &programs::program_id(),
        )];
        let ok = execute_and_report(action_msg, caller, "ไม่อนุมัติแล้ว", ixs).await;
        if ok {
            self.refresh(admin.to_string()).await;
        }
        ok
    }

    /// จ่ายเงินคืนผู้ซื้อ (Owner/Parent)
    async fn do_reimburse(&self, admin: &Pubkey, caller: &Pubkey, buyer: &Pubkey, request_id: u64, lamports: u64) -> bool {
        let action_msg = self.action_msg;
        let ixs = vec![programs::reimburse_buyer(
            admin,
            caller,
            buyer,
            request_id,
            lamports,
            &programs::program_id(),
        )];
        let ok = execute_and_report(action_msg, caller, "จ่ายเงินคืนแล้ว", ixs).await;
        if ok {
            self.refresh(admin.to_string()).await;
        }
        ok
    }

    /// ผู้ซื้อยืนยันว่าซื้อแล้ว (unit cost เก็บ hash เหมือนเดิม)
    async fn do_confirm(&self, owner: &Pubkey, buyer: &Pubkey, request_id: u64) -> bool {
        let ixs = vec![programs::confirm_restock(
            owner,
            buyer,
            request_id,
            programs::text_hash("ซื้อแล้ว"),
            &programs::program_id(),
        )];
        let ok = execute_and_report(self.action_msg, buyer, "บันทึกว่าซื้อแล้ว", ixs).await;
        if ok {
            self.refresh(owner.to_string()).await;
        }
        ok
    }

    /// เปิดคำขอใหม่ + จำข้อความ item ไว้ในเครื่อง (บน chain เก็บ blake3 เท่านั้น)
    async fn do_create(&self, caller: &Pubkey, buyer: &Pubkey, counter: u64, lamports: u64, item_text: &str, unit_cost: &str) -> bool {
        let action_msg = self.action_msg;
        let ixs = vec![programs::create_purchase_request(
            caller,
            buyer,
            counter,
            lamports,
            programs::text_hash(item_text),
            programs::text_hash(unit_cost),
            &programs::program_id(),
        )];
        let ok = execute_and_report(action_msg, caller, &format!("เปิดคำขอ #{counter}"), ixs).await;
        if ok {
            let hh = programs::household_pda(caller, &programs::program_id()).0;
            save_item_text(&hh.to_string(), counter, item_text);
            self.refresh(caller.to_string()).await;
        }
        ok
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
    let mut member_input = use_signal(String::new);
    let mut member_role = use_signal(|| "2".to_string()); // Child by default
    let mut restock_id_input = use_signal(String::new);
    let mut restock_cost_input = use_signal(String::new);
    let mut reimb_id_input = use_signal(String::new);
    let mut reimb_amount_input = use_signal(String::new);
    let mut reject_id_input = use_signal(String::new);
    let mut reject_reason_input = use_signal(String::new);
    let mut close_id_input = use_signal(String::new);
    let mut withdraw_input = use_signal(String::new);
    let mut award_member_input = use_signal(String::new);
    let mut award_points_input = use_signal(String::new);
    let mut award_reason_input = use_signal(String::new);
    let mut remove_member_input = use_signal(String::new);
    let mut role_member_input = use_signal(String::new);
    let mut role_select = use_signal(|| "1".to_string());
    let mut chat = use_signal(Vec::new);
    let mut chat_input = use_signal(String::new);
    let mut chat_busy = use_signal(|| false);
    // receipt → requests (v0.7)
    let mut receipt_items = use_signal(Vec::new);
    let mut receipt_state = use_signal(|| "idle".to_string());
    let mut receipt_currency = use_signal(|| "THB".to_string());
    let mut sol_thb = use_signal(|| DEFAULT_SOL_THB);
    // โหมดบ้าน (v1.0): tab + shared context for every action
    let mut view_mode = use_signal(|| ViewMode::Home);
    let mut advanced_mode = use_signal(|| false);
    // เชิญสมาชิกผ่านลิงก์ (?join=<owner_address>)
    let mut copied_invite = use_signal(|| false);
    let mut joined_from = use_signal(|| None::<String>);
    let mut contacts = use_signal(Vec::new);
    let mut members = use_signal(Vec::new);
    let mut item_texts = use_signal(Vec::new);
    let ctx = Ctx {
        action_msg,
        action_busy,
        state,
        ledger,
        contacts,
        members,
        item_texts,
    };
    // theme: "system" | "light" | "dark" (persisted in localStorage)
    let mut theme_mode = use_signal(|| {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| s.get_item("stocksie-theme").ok().flatten())
            .unwrap_or_else(|| "system".to_string())
    });
    // apply + persist the theme whenever it changes
    use_effect(move || {
        let mode = theme_mode();
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(html) = doc.document_element() {
                let _ = match mode.as_str() {
                    "dark" => html.set_attribute("data-theme", "dark"),
                    "light" => html.set_attribute("data-theme", "light"),
                    _ => html.remove_attribute("data-theme"),
                };
            }
        }
        if let Some(store) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = store.set_item("stocksie-theme", &mode);
        }
    });

    let toggle_theme = move |_| {
        let next = match theme_mode().as_str() {
            "system" => "light",
            "light" => "dark",
            _ => "system",
        };
        theme_mode.set(next.to_string());
    };

    let connect_wallet = move |_| {
        spawn(async move {
            wallet_busy.set(true);
            wallet_error.set(None);
            match wallet::detect_provider() {
                Err(e) => wallet_error.set(Some(e)),
                Ok(provider) => match wallet::connect(&provider).await {
                    Ok(pk) => {
                        wallet_pubkey.set(Some(pk.clone()));
                        ctx.refresh(pk).await;
                    }
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
                    ctx.refresh(pk).await;
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
                    ctx.refresh(pk).await;
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
                    ctx.refresh(pk).await;
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
            match execute(&owner, vec![programs::approve_purchase_request(&owner, &owner, request_id, &programs::program_id())]).await {
                Ok(sig) => {
                    action_msg.set(format!("✓ approved — {}", &sig[..16.min(sig.len())]));
                    ctx.refresh(pk).await;
                }
                Err(e) => action_msg.set(format!("⚠ {e}")),
            }
            action_busy.set(false);
        });
    };

    let add_member_action = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Ok(new_member) = Pubkey::from_str(member_input().trim()) else {
                action_msg.set("⚠ Invalid member address.".into());
                action_busy.set(false);
                return;
            };
            let Ok(role_tag) = member_role().parse::<u8>() else {
                action_msg.set("⚠ Pick a role.".into());
                action_busy.set(false);
                return;
            };
            let owner = Pubkey::from_str(&pk).expect("connected key");
            match execute(&owner, vec![programs::add_member(&owner, &new_member, role_tag, &programs::program_id())]).await {
                Ok(sig) => {
                    action_msg.set(format!("✓ member added — {}", &sig[..16.min(sig.len())]));
                    ctx.refresh(pk).await;
                }
                Err(e) => action_msg.set(format!("⚠ {e}")),
            }
            action_busy.set(false);
        });
    };

    // buyer signs; the household comes from the admin address field above
    let confirm_restock_action = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Ok(request_id) = restock_id_input().trim().parse::<u64>() else {
                action_msg.set("⚠ Enter the request number.".into());
                action_busy.set(false);
                return;
            };
            let cost = restock_cost_input().trim().to_string();
            if cost.is_empty() {
                action_msg.set("⚠ Enter what you actually paid.".into());
                action_busy.set(false);
                return;
            }
            let buyer = Pubkey::from_str(&pk).expect("connected key");
            let owner = Pubkey::from_str(&owner_input()).expect("admin address");
            match execute(
                &buyer,
                vec![programs::confirm_restock(&owner, &buyer, request_id, programs::text_hash(&cost), &programs::program_id())],
            )
            .await
            {
                Ok(sig) => {
                    action_msg.set(format!("✓ restocked — {}", &sig[..16.min(sig.len())]));
                    ctx.refresh(owner_input()).await;
                }
                Err(e) => action_msg.set(format!("⚠ {e}")),
            }
            action_busy.set(false);
        });
    };

    // owner/parent signs; the buyer receives the payout
    let reimburse_action = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Ok(request_id) = reimb_id_input().trim().parse::<u64>() else {
                action_msg.set("⚠ Enter the request number.".into());
                action_busy.set(false);
                return;
            };
            let Some(lamports) = parse_sol_to_lamports(&reimb_amount_input()) else {
                action_msg.set("⚠ Enter a valid SOL amount.".into());
                action_busy.set(false);
                return;
            };
            let caller = Pubkey::from_str(&pk).expect("connected key");
            let owner = Pubkey::from_str(&owner_input()).expect("admin address");
            // buyer pubkey resolved from the loaded ledger (decoded on-chain)
            let buyer = match &*ledger.read() {
                LedgerState::Loaded(rows) => rows
                    .iter()
                    .find(|r| r.request_id == request_id)
                    .map(|r| r.buyer_key),
                _ => None,
            };
            let Some(buyer) = buyer else {
                action_msg.set("⚠ Request not found in the ledger — load the household first.".into());
                action_busy.set(false);
                return;
            };
            match execute(
                &caller,
                vec![programs::reimburse_buyer(&owner, &caller, &buyer, request_id, lamports, &programs::program_id())],
            )
            .await
            {
                Ok(sig) => {
                    action_msg.set(format!("✓ reimbursed — {}", &sig[..16.min(sig.len())]));
                    ctx.refresh(owner_input()).await;
                }
                Err(e) => action_msg.set(format!("⚠ {e}")),
            }
            action_busy.set(false);
        });
    };

    // --- admin/owner actions (v0.9) — household derives from the admin field ---
    let reject_action = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Ok(request_id) = reject_id_input().trim().parse::<u64>() else {
                action_msg.set("⚠ Enter the request number.".into());
                action_busy.set(false);
                return;
            };
            let reason = reject_reason_input().trim().to_string();
            if reason.is_empty() {
                action_msg.set("⚠ Enter a rejection reason (kept private).".into());
                action_busy.set(false);
                return;
            }
            let caller = Pubkey::from_str(&pk).expect("connected key");
            let admin = Pubkey::from_str(&owner_input()).expect("admin address");
            let ok = execute_and_report(
                action_msg,
                &caller,
                "declined",
                vec![programs::reject_purchase_request(
                    &admin,
                    &caller,
                    request_id,
                    programs::text_hash(&reason),
                    &programs::program_id(),
                )],
            )
            .await;
            if ok {
                ctx.refresh(owner_input()).await;
            }
            action_busy.set(false);
        });
    };

    let close_action = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Ok(request_id) = close_id_input().trim().parse::<u64>() else {
                action_msg.set("⚠ Enter the request number.".into());
                action_busy.set(false);
                return;
            };
            let caller = Pubkey::from_str(&pk).expect("connected key");
            let admin = Pubkey::from_str(&owner_input()).expect("admin address");
            let ok = execute_and_report(
                action_msg,
                &caller,
                "closed (rent reclaimed)",
                vec![programs::close_purchase_request(&admin, &caller, request_id, &programs::program_id())],
            )
            .await;
            if ok {
                ctx.refresh(owner_input()).await;
            }
            action_busy.set(false);
        });
    };

    let withdraw_action = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Some(lamports) = parse_sol_to_lamports(&withdraw_input()) else {
                action_msg.set("⚠ Enter a valid SOL amount.".into());
                action_busy.set(false);
                return;
            };
            let owner = Pubkey::from_str(&pk).expect("connected key");
            let ok = execute_and_report(
                action_msg,
                &owner,
                "withdrawn to your account",
                vec![programs::withdraw_funds(&owner, lamports, &programs::program_id())],
            )
            .await;
            if ok {
                ctx.refresh(pk).await;
            }
            action_busy.set(false);
        });
    };

    let award_action = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Ok(target) = Pubkey::from_str(award_member_input().trim()) else {
                action_msg.set("⚠ Invalid member address.".into());
                action_busy.set(false);
                return;
            };
            let Ok(points) = award_points_input().trim().parse::<u64>() else {
                action_msg.set("⚠ Points must be a whole number.".into());
                action_busy.set(false);
                return;
            };
            let reason = award_reason_input().trim().to_string();
            if reason.is_empty() {
                action_msg.set("⚠ Enter a reason (kept private).".into());
                action_busy.set(false);
                return;
            }
            let caller = Pubkey::from_str(&pk).expect("connected key");
            let admin = Pubkey::from_str(&owner_input()).expect("admin address");
            let ok = execute_and_report(
                action_msg,
                &caller,
                "reward granted",
                vec![programs::award_reward(
                    &admin,
                    &caller,
                    &target,
                    points,
                    programs::text_hash(&reason),
                    &programs::program_id(),
                )],
            )
            .await;
            if ok {
                ctx.refresh(owner_input()).await;
            }
            action_busy.set(false);
        });
    };

    let remove_member_action = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Ok(target) = Pubkey::from_str(remove_member_input().trim()) else {
                action_msg.set("⚠ Invalid member address.".into());
                action_busy.set(false);
                return;
            };
            let caller = Pubkey::from_str(&pk).expect("connected key");
            let admin = Pubkey::from_str(&owner_input()).expect("admin address");
            let ok = execute_and_report(
                action_msg,
                &caller,
                "member removed",
                vec![programs::remove_member(&admin, &caller, &target, &programs::program_id())],
            )
            .await;
            if ok {
                ctx.refresh(owner_input()).await;
            }
            action_busy.set(false);
        });
    };

    let set_role_action = move |_| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let Ok(target) = Pubkey::from_str(role_member_input().trim()) else {
                action_msg.set("⚠ Invalid member address.".into());
                action_busy.set(false);
                return;
            };
            let Ok(role_tag) = role_select().parse::<u8>() else {
                action_msg.set("⚠ Pick a role.".into());
                action_busy.set(false);
                return;
            };
            let caller = Pubkey::from_str(&pk).expect("connected key");
            let admin = Pubkey::from_str(&owner_input()).expect("admin address");
            let ok = execute_and_report(
                action_msg,
                &caller,
                "role updated",
                vec![programs::set_role(&admin, &caller, &target, role_tag, &programs::program_id())],
            )
            .await;
            if ok {
                ctx.refresh(owner_input()).await;
            }
            action_busy.set(false);
        });
    };

    let send_chat = move |_| {
        spawn(send_chat_msg(chat, chat_input, chat_busy, state, ledger));
    };

    let fetch = move |owner_raw: String| {
        spawn(async move {
            ctx.refresh(owner_raw).await;
        });
    };

    // live THB/SOL rate for receipt conversion (fallback constant offline)
    use_effect(move || {
        spawn(async move {
            if let Ok(res) = gloo_net::http::Request::get(
                "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=thb",
            )
            .send()
            .await
            {
                if let Ok(v) = res.json::<serde_json::Value>().await {
                    if let Some(thb) = v
                        .get("solana")
                        .and_then(|s| s.get("thb"))
                        .and_then(|t| t.as_f64())
                    {
                        if thb > 0.0 {
                            sol_thb.set(thb);
                        }
                    }
                }
            }
        });
    });

    // เชิญสมาชิก: อ่าน ?join=<owner_address> จากลิงก์ → โหลดบ้านของเจ้าบ้านทันที
    use_effect(move || {
        let join = web_sys::window()
            .and_then(|w| w.location().search().ok())
            .and_then(|search| {
                search
                    .split('&')
                    .find_map(|p| {
                        p.strip_prefix("?join=")
                            .or_else(|| p.strip_prefix("&join="))
                            .map(|s| s.trim().to_string())
                    })
                    .filter(|s| !s.is_empty())
            });
        if let Some(addr) = join {
            owner_input.set(addr.clone());
            joined_from.set(Some(addr.clone()));
            spawn(async move {
                ctx.refresh(addr).await;
            });
        }
    });

    // 🧾 receipt: read the picked file (web-sys), POST to /api/ai, list items
    let parse_receipt = move |_| {
        spawn(async move {
            receipt_state.set("loading".to_string());
            let (bytes, mime) = match read_receipt_file("receipt-file").await {
                Ok(r) => r,
                Err(e) => {
                    receipt_state.set(format!("error:{e}"));
                    return;
                }
            };
            if bytes.len() > 8 * 1024 * 1024 {
                receipt_state.set("error:image over 8 MB".to_string());
                return;
            }
            let body = json!({
                "mode": "receipt",
                "image": {
                    "base64": B64.encode(&bytes),
                    "mimeType": if mime.is_empty() { "image/png".to_string() } else { mime },
                }
            });
            match post_json(AI_API_URL, body).await {
                Ok(v) => {
                    let ok = v.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                    let mode = v.get("mode").and_then(|m| m.as_str()).unwrap_or("");
                    if ok && mode == "receipt" {
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
                        let currency = v
                            .get("currency")
                            .and_then(|c| c.as_str())
                            .unwrap_or("THB")
                            .to_string();
                        receipt_currency.set(currency);
                        receipt_items.set(items);
                        receipt_state.set("ok".to_string());
                    } else {
                        let err = v
                            .get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or("unexpected AI response")
                            .to_string();
                        receipt_state.set(format!("error:{err}"));
                    }
                }
                Err(e) => receipt_state.set(format!("error:{e}")),
            }
        });
    };

    // one receipt line → a real create_purchase_request (THB → SOL via live rate)
    let open_from_receipt = move |item: ChatItem| {
        spawn(async move {
            action_busy.set(true);
            action_msg.set(String::new());
            let Some(pk) = wallet_pubkey().clone() else {
                action_msg.set("⚠ Connect a wallet first.".into());
                action_busy.set(false);
                return;
            };
            let owner = Pubkey::from_str(&pk).expect("connected key");
            let (_, counter) = match fetch_household_state(&owner).await {
                Ok(r) => r,
                Err(e) => {
                    action_msg.set(format!("⚠ {e}"));
                    action_busy.set(false);
                    return;
                }
            };
            let lamports =
                ((item.total_minor as f64 / 100.0) / sol_thb() * 1e9).round() as u64;
            if lamports < MIN_REQUEST_LAMPORTS {
                action_msg.set(format!(
                    "⚠ {} converts to {} lamports — below the 0.0001 SOL minimum. Raise the price note.",
                    item.name, lamports
                ));
                action_busy.set(false);
                return;
            }
            if lamports > MAX_REQUEST_LAMPORTS {
                action_msg.set(format!(
                    "⚠ {} exceeds the 0.5 SOL per-request ceiling. Split the line.",
                    item.name
                ));
                action_busy.set(false);
                return;
            }
            let item_text = if item.quantity > 1 {
                format!("{} ×{}", item.name, item.quantity)
            } else {
                item.name.clone()
            };
            let unit_cost = format!("฿{:.2} from receipt", item.total_minor as f64 / 100.0);
            match execute(
                &owner,
                vec![programs::create_purchase_request(
                    &owner,
                    &owner, // buyer = self (another member approves on-chain)
                    counter + 1,
                    lamports,
                    programs::text_hash(&item_text),
                    programs::text_hash(&unit_cost),
                    &programs::program_id(),
                )],
            )
            .await
            {
                Ok(sig) => {
                    action_msg.set(format!(
                        "✓ {} opened as request #{} — {}",
                        item.name,
                        counter + 1,
                        &sig[..16.min(sig.len())]
                    ));
                    ctx.refresh(pk).await;
                }
                Err(e) => action_msg.set(format!("⚠ {e}")),
            }
            action_busy.set(false);
        });
    };


    rsx! {
        style { {THEME_CSS} }
        main { style: "max-width:640px;margin:48px auto;padding:0 20px;font-family:ui-sans-serif,system-ui;color:var(--text)",
            nav { class: "topnav",
                button {
                    class: "copybtn",
                    style: "padding:4px 10px;font-size:12px",
                    onclick: move |_| advanced_mode.set(!advanced_mode()),
                    {if *advanced_mode.read() { "🏠 โหมดบ้าน" } else { "⚙️ โหมดขั้นสูง" }}
                }
                if *advanced_mode.read() {
                    a { href: "#ledger", "📒 Ledger" }
                    a { href: "#wallet", "👛 Wallet" }
                    a { href: "#agent", "🤖 Agent" }
                    a { href: "#actions", "🛠 Actions" }
                    a { href: "#receipt", "🧾 Receipt" }
                }
                div { style: "flex:1" }
                button {
                    class: "copybtn",
                    style: "padding:4px 10px;font-size:12px",
                    onclick: toggle_theme,
                    title: "Toggle theme (system / light / dark)",
                    match theme_mode().as_str() {
                        "light" => "☀️ light",
                        "dark" => "🌙 dark",
                        _ => "🌗 system",
                    }
                }
            }
            if *advanced_mode.read() {
            header {
                h1 { "🤖🏠 Stocksie — RIIR prototype" }
                p { style: "color:var(--muted);font-size:14px",
                    "Household reader rewritten in Rust (Dioxus → WASM). "
                    "PDA derivation + borsh decode, zero JavaScript client code."
                }
            }

            div { style: "display:flex;gap:8px;margin:16px 0",
                input {
                    style: "flex:1;padding:10px;border:1px solid var(--border-strong);border-radius:8px;font-family:ui-monospace,monospace;font-size:13px",
                    value: "{owner_input}",
                    oninput: move |e| owner_input.set(e.value()),
                    placeholder: "Household owner wallet address",
                }
                button {
                    style: "padding:10px 16px;border:none;border-radius:8px;background:var(--accent);color:white;font-weight:600;cursor:pointer",
                    onclick: move |_| fetch(owner_input()),
                    "Load household"
                }
            }

            div { style: "display:flex;gap:8px;flex-wrap:wrap;margin-bottom:16px",
                for (addr, label) in DEMO_OWNERS {
                    button {
                        style: "padding:6px 10px;border:1px solid var(--border-strong);border-radius:6px;background:var(--surface);color:var(--text);font-size:12px;cursor:pointer",
                        onclick: move |_| {
                            owner_input.set(addr.to_string());
                            fetch(addr.to_string());
                        },
                        "{label}"
                    }
                }
            }

            match &*state.read() {
                FetchState::Idle => rsx! { p { style: "color:var(--faint)", "Enter an owner address (or pick a demo) and load." } },
                FetchState::Loading => rsx! { p { style: "color:var(--faint)", "Fetching from devnet…" } },
                FetchState::Failed(msg) => rsx! {
                    div { style: "padding:12px;border:1px solid var(--danger-border);border-radius:8px;background:var(--danger-bg);color:var(--danger);font-size:13px",
                        "⚠ {msg}"
                    }
                },
                FetchState::Loaded(v) => rsx! { HouseholdCard { view: *v.clone() } },
            }

            h2 { id: "ledger", style: "margin:28px 0 8px;font-size:16px", "🛒 Purchase ledger" }
            match &*ledger.read() {
                LedgerState::Idle => rsx! { p { style: "color:var(--faint);font-size:13px", "Load a household to see its requests." } },
                LedgerState::Loading => rsx! { p { style: "color:var(--faint);font-size:13px", "Reading the ledger from devnet…" } },
                LedgerState::Failed(msg) => rsx! {
                    div { style: "padding:12px;border:1px solid var(--danger-border);border-radius:8px;background:var(--danger-bg);color:var(--danger);font-size:13px",
                        "⚠ {msg}"
                    }
                },
                LedgerState::Loaded(rows) if rows.is_empty() => rsx! {
                    p { style: "color:var(--faint);font-size:13px", "No purchase requests for this household yet." }
                },
                LedgerState::Loaded(rows) => rsx! { LedgerTable { rows: rows.clone() } },
            }

            h2 { id: "wallet", style: "margin:28px 0 8px;font-size:16px", "👛 Wallet — first signed tx from Rust" }
            div { style: "border:1px solid var(--border-strong);border-radius:12px;padding:16px;display:grid;gap:10px",
                p { style: "font-size:12px;color:var(--muted);margin:0",
                    "Detects window.phantom (Wallet Standard), builds the memo tx wire bytes in Rust, and hands them to the wallet — the wallet fills the signature slot and submits."
                }
                match &*wallet_pubkey.read() {
                    Some(pk) => rsx! {
                        div { style: "display:flex;gap:8px;align-items:center;flex-wrap:wrap",
                            span { style: "padding:2px 10px;border-radius:9999px;background:#dcfce7;color:var(--success);font-size:12px;font-weight:600", "connected" }
                            span { style: "font-family:ui-monospace,monospace;font-size:12px", "{pk}" }
                        }
                        button {
                            style: "padding:8px 14px;border:none;border-radius:8px;background:var(--accent);color:white;font-weight:600;cursor:pointer",
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
                            style: "padding:8px 14px;border:none;border-radius:8px;background:var(--violet);color:white;font-weight:600;cursor:pointer",
                            onclick: connect_wallet,
                            disabled: *wallet_busy.read(),
                            if *wallet_busy.read() { "Connecting…" } else { "Connect Phantom" }
                        }
                    },
                }
                if let Some(err) = &*wallet_error.read() {
                    p { style: "font-size:12px;color:var(--danger);margin:0", "⚠ {err}" }
                }
                match &*send_state.read() {
                    SendState::Idle => rsx! {
                        p { style: "font-size:12px;color:var(--faint);margin:0",
                            "Nothing sent yet — the tx is a SPL Memo v1 with your key as fee payer."
                        }
                    },
                    SendState::Building => rsx! {
                        p { style: "font-size:12px;color:var(--faint);margin:0", "Fetching blockhash & building wire bytes…" }
                    },
                    SendState::Failed(e) => rsx! {
                        p { style: "font-size:12px;color:var(--danger);margin:0", "⚠ {e}" }
                    },
                    SendState::Sent(sig) => rsx! {
                        div { style: "padding:10px;border:1px solid var(--success-border);border-radius:8px;background:var(--success-bg);display:grid;gap:6px",
                            span { style: "font-size:12px;color:var(--success);font-weight:600", "Signed & submitted ✓" }
                            a {
                                href: "https://explorer.solana.com/tx/{sig}?cluster=devnet",
                                target: "_blank",
                                style: "font-size:12px;color:var(--link)",
                                "view on Solana Explorer ↗"
                            }
                            code { style: "font-size:10px;word-break:break-all;color:var(--muted)", "{sig}" }
                        }
                    },
                }
            }

            h2 { id: "agent", style: "margin:28px 0 8px;font-size:16px", "🤖 AI agent — same /api/ai as the Next.js app" }
            div { style: "border:1px solid var(--border-strong);border-radius:12px;padding:16px;display:grid;gap:10px",
                div {
                    id: "chat-box",
                    if chat.len() == 0 {
                        p { style: "font-size:12px;color:var(--faint);margin:0",
                            "Try “ซื้อนม 2 ขวด งบ 50 บาท” or “งบเหลือเท่าไหร่” — proposals list items; answers use the live household numbers loaded above."
                        }
                    }
                    for msg in chat() {
                        div {
                            class: if msg.role == "user" { "align-self:flex-end" } else { "align-self:flex-start" },
                            style: if msg.role == "user" {
                                "max-width:85%;padding:8px 12px;border-radius:12px 12px 2px 12px;background:var(--accent);color:white;font-size:13px"
                            } else {
                                "max-width:90%;padding:8px 12px;border-radius:12px 12px 12px 2px;background:var(--border);color:var(--text);font-size:13px"
                            },
                            "{msg.text}"
                            if !msg.items.is_empty() {
                                div { style: "display:flex;flex-wrap:wrap;gap:6px;margin-top:8px",
                                    for item in msg.items {
                                        span {
                                            style: "padding:2px 8px;border-radius:6px;background:var(--surface-2);color:var(--ink-soft);font-size:11px;border:1px solid var(--border-strong)",
                                            "{item.label()}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if chat_busy() {
                        div { style: "align-self:flex-start;font-size:12px;color:var(--faint)", "Thinking…" }
                    }
                }
                div { style: "display:flex;gap:8px",
                    input {
                        style: "flex:1;padding:10px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                        value: "{chat_input}",
                        oninput: move |e| chat_input.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter && !e.modifiers().contains(Modifiers::SHIFT) {
                                spawn(send_chat_msg(chat, chat_input, chat_busy, state, ledger));
                            }
                        },
                        placeholder: "Ask the agent… (Thai/English) — Enter to send",
                        disabled: chat_busy(),
                    }
                    button {
                        style: "padding:10px 16px;border:none;border-radius:8px;background:var(--violet);color:white;font-weight:600;cursor:pointer",
                        onclick: send_chat,
                        disabled: chat_busy() || chat_input().trim().is_empty(),
                        "Send"
                    }
                }
            }

            h2 { id: "actions", style: "margin:28px 0 8px;font-size:16px", "🛠 Actions — household ops as Rust wire bytes" }
            div { style: "border:1px solid var(--border-strong);border-radius:12px;padding:16px;display:grid;gap:12px",
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "flex:1;min-width:200px",
                        label { style: "font-size:11px;color:var(--muted)", "Household name" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                            value: "{name_input}",
                            oninput: move |e| name_input.set(e.value()),
                            placeholder: "บ้านของเรา",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: init_household,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Initialize household"
                    }
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "flex:1;min-width:140px",
                        label { style: "font-size:11px;color:var(--muted)", "Deposit (SOL)" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{deposit_input}",
                            oninput: move |e| deposit_input.set(e.value()),
                            placeholder: "0.1",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: deposit,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Deposit"
                    }
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "flex:2;min-width:180px",
                        label { style: "font-size:11px;color:var(--muted)", "Item to buy" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                            value: "{item_input}",
                            oninput: move |e| item_input.set(e.value()),
                            placeholder: "นมสด 1 ลิตร ×2",
                        }
                    }
                    div { style: "width:120px",
                        label { style: "font-size:11px;color:var(--muted)", "Limit (SOL)" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{req_amount_input}",
                            oninput: move |e| req_amount_input.set(e.value()),
                            placeholder: "0.02",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: open_request,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Open request"
                    }
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "width:120px",
                        label { style: "font-size:11px;color:var(--muted)", "Request #" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{approve_id_input}",
                            oninput: move |e| approve_id_input.set(e.value()),
                            placeholder: "1",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: approve,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Approve"
                    }
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "flex:2;min-width:180px",
                        label { style: "font-size:11px;color:var(--muted)", "New member address" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{member_input}",
                            oninput: move |e| member_input.set(e.value()),
                            placeholder: "Their wallet address",
                        }
                    }
                    div { style: "width:120px",
                        label { style: "font-size:11px;color:var(--muted)", "Role" }
                        select {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                            value: "{member_role}",
                            onchange: move |e| member_role.set(e.value()),
                            option { value: "1", "Parent" }
                            option { value: "2", "Child" }
                            option { value: "3", "Guest" }
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: add_member_action,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Add member"
                    }
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "width:120px",
                        label { style: "font-size:11px;color:var(--muted)", "Request #" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{restock_id_input}",
                            oninput: move |e| restock_id_input.set(e.value()),
                            placeholder: "1",
                        }
                    }
                    div { style: "flex:1;min-width:180px",
                        label { style: "font-size:11px;color:var(--muted)", "What you actually paid" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                            value: "{restock_cost_input}",
                            oninput: move |e| restock_cost_input.set(e.value()),
                            placeholder: "฿48 at 7-Eleven (buyer signs)",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: confirm_restock_action,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Mark as bought"
                    }
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "width:120px",
                        label { style: "font-size:11px;color:var(--muted)", "Request #" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{reimb_id_input}",
                            oninput: move |e| reimb_id_input.set(e.value()),
                            placeholder: "1",
                        }
                    }
                    div { style: "width:140px",
                        label { style: "font-size:11px;color:var(--muted)", "Pay back (SOL)" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{reimb_amount_input}",
                            oninput: move |e| reimb_amount_input.set(e.value()),
                            placeholder: "0.005",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: reimburse_action,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Reimburse buyer"
                    }
                }
                div { style: "font-size:11px;color:var(--faint);margin-top:4px", "TREASURY (admin)" }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "width:140px",
                        label { style: "font-size:11px;color:var(--muted)", "Withdraw (SOL)" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{withdraw_input}",
                            oninput: move |e| withdraw_input.set(e.value()),
                            placeholder: "0.01",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: withdraw_action,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Withdraw"
                    }
                }
                div { style: "font-size:11px;color:var(--faint);margin-top:8px", "REWARDS (admin/parent)" }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "flex:2;min-width:180px",
                        label { style: "font-size:11px;color:var(--muted)", "Member address" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{award_member_input}",
                            oninput: move |e| award_member_input.set(e.value()),
                            placeholder: "Their wallet address",
                        }
                    }
                    div { style: "width:110px",
                        label { style: "font-size:11px;color:var(--muted)", "Points" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{award_points_input}",
                            oninput: move |e| award_points_input.set(e.value()),
                            placeholder: "25",
                        }
                    }
                    div { style: "flex:1;min-width:160px",
                        label { style: "font-size:11px;color:var(--muted)", "Reason" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                            value: "{award_reason_input}",
                            oninput: move |e| award_reason_input.set(e.value()),
                            placeholder: "Great job!",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: award_action,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Give points"
                    }
                }
                div { style: "font-size:11px;color:var(--faint);margin-top:8px", "MEMBERS & ROLES (admin)" }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "flex:2;min-width:180px",
                        label { style: "font-size:11px;color:var(--muted)", "Member address" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{role_member_input}",
                            oninput: move |e| role_member_input.set(e.value()),
                            placeholder: "Their wallet address",
                        }
                    }
                    div { style: "width:120px",
                        label { style: "font-size:11px;color:var(--muted)", "New role" }
                        select {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                            value: "{role_select}",
                            onchange: move |e| role_select.set(e.value()),
                            option { value: "1", "Parent" }
                            option { value: "2", "Child" }
                            option { value: "3", "Guest" }
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: set_role_action,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Update role"
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--danger);color:white;font-weight:600;cursor:pointer",
                        onclick: remove_member_action,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Remove member"
                    }
                }
                div { style: "font-size:11px;color:var(--faint);margin-top:8px", "DECLINE / CLOSE / WITHDRAW A REQUEST" }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:flex-end",
                    div { style: "width:110px",
                        label { style: "font-size:11px;color:var(--muted)", "Request #" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{reject_id_input}",
                            oninput: move |e| reject_id_input.set(e.value()),
                            placeholder: "2",
                        }
                    }
                    div { style: "flex:1;min-width:160px",
                        label { style: "font-size:11px;color:var(--muted)", "Decline reason" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                            value: "{reject_reason_input}",
                            oninput: move |e| reject_reason_input.set(e.value()),
                            placeholder: "Changed my mind (🔒 private)",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--danger);color:white;font-weight:600;cursor:pointer",
                        onclick: reject_action,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Decline"
                    }
                    div { style: "width:110px",
                        label { style: "font-size:11px;color:var(--muted)", "Close #" }
                        input {
                            style: "width:100%;padding:8px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px;font-family:ui-monospace,monospace",
                            value: "{close_id_input}",
                            oninput: move |e| close_id_input.set(e.value()),
                            placeholder: "2",
                        }
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: close_action,
                        disabled: action_busy() || wallet_pubkey().is_none(),
                        "Close request"
                    }
                }
                if !action_msg().is_empty() {
                    div {
                        class: if action_msg().starts_with("✓") { "banner banner-ok" } else { "banner banner-err" },
                        "{action_msg}"
                    }
                }
                p { style: "font-size:11px;color:var(--faint);margin:0",
                    "Note: create with buyer = you, then approve — the program forbids self-approval, so use a second wallet for the buyer in a real household."
                }
            }

            h2 { id: "receipt", style: "margin:28px 0 8px;font-size:16px", "🧾 Receipt → requests" }
            div { style: "border:1px solid var(--border-strong);border-radius:12px;padding:16px;display:grid;gap:10px",
                div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:center",
                    input {
                        r#type: "file",
                        id: "receipt-file",
                        accept: "image/png,image/jpeg,image/webp",
                        style: "font-size:12px",
                    }
                    button {
                        style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                        onclick: parse_receipt,
                        disabled: *receipt_state.read() == "loading",
                        if *receipt_state.read() == "loading" { "Reading…" } else { "Extract items" }
                    }
                    span { style: "font-size:11px;color:var(--muted)",
                        "1 SOL ≈ ฿{sol_thb():.0} — extracted items convert to SOL automatically"
                    }
                }
                match &*receipt_state.read() {
                    s if s == "idle" => rsx! { p { style: "font-size:12px;color:var(--faint);margin:0", "Snap your receipt, pick the photo, and the AI drafts each line as a request." } },
                    s if s == "loading" => rsx! { p { style: "font-size:12px;color:var(--faint);margin:0", "The AI is reading the receipt…" } },
                    s if s.starts_with("error:") => {
                        let msg = s.trim_start_matches("error:");
                        rsx! { p { style: "font-size:12px;color:var(--danger);margin:0", "⚠ {msg}" } }
                    },
                    _ => rsx! {
                        p { style: "font-size:12px;color:var(--success);margin:0", "✓ items extracted — review, then open each request." }
                    },
                }
                div { style: "display:grid;gap:8px",
                    for item in receipt_items() {
                        div { style: "display:flex;gap:8px;align-items:center;flex-wrap:wrap;border:1px solid var(--border);border-radius:8px;padding:8px 10px",
                            span { style: "flex:1;font-size:13px", "{item.name}" }
                            span { style: "font-size:12px;color:var(--muted)", "×{item.quantity}" }
                            span { style: "font-size:12px;font-family:ui-monospace,monospace", "{item.price_label()}" }
                            button {
                                style: "padding:6px 12px;border:none;border-radius:8px;background:var(--accent);color:white;font-size:12px;font-weight:600;cursor:pointer",
                                onclick: move |_| open_from_receipt(item.clone()),
                                disabled: action_busy() || wallet_pubkey().is_none(),
                                "Open request"
                            }
                        }
                    }
                }
            }

            } else {
                header { style: "display:flex;justify-content:space-between;align-items:center",
                    h1 { style: "font-size:20px;margin:0", "🏠 บ้านของเรา" }
                }
                {match (wallet_pubkey().clone(), state().clone()) {
                    (None, _) => rsx! {
                        div { style: "border:1px solid var(--border-strong);border-radius:14px;padding:20px;display:grid;gap:10px;background:var(--surface)",
                            p { style: "margin:0;font-size:14px", "ยินดีต้อนรับ 👋" }
                            p { style: "margin:0;font-size:12px;color:var(--muted)", "1️⃣ เชื่อมกระเป๋าเงินเพื่อเริ่มใช้งาน" }
                            if let Some(err) = &*wallet_error.read() {
                                p { style: "margin:0;font-size:12px;color:var(--danger)",
                                    "⚠ ยังไม่พบกระเป๋าเงินในเบราว์เซอร์นี้ — ติดตั้ง Phantom จาก phantom.app แล้วลองอีกครั้ง"
                                }
                            }
                            button {
                                style: "padding:10px 14px;border:none;border-radius:8px;background:var(--violet);color:white;font-weight:600;cursor:pointer",
                                onclick: connect_wallet,
                                disabled: *wallet_busy.read(),
                                if *wallet_busy.read() { "Connecting…" } else { "เข้าใช้งานด้วยกระเป๋าเงิน" }
                            }
                        }
                    },
                    (Some(_), FetchState::Loading) => rsx! { p { style: "color:var(--faint);font-size:14px", "กำลังโหลด…" } },
                    (Some(_), FetchState::Failed(msg)) => rsx! {
                        div { class: "banner banner-err", "⚠ {msg}" }
                        p { style: "color:var(--muted);font-size:13px", "ใส่ address เจ้าบ้านในโหมดขั้นสูง ⚙️ แล้วกด Load household" }
                    },
                    (Some(_), FetchState::Idle) => rsx! {
                        div { style: "border:1px solid var(--border-strong);border-radius:14px;padding:20px;display:grid;gap:10px;background:var(--surface)",
                            p { style: "margin:0;font-size:14px", "2️⃣ ตั้งชื่อบ้าน — จบในคลิกเดียว" }
                            div { style: "display:flex;gap:8px",
                                input {
                                    style: "flex:1;padding:10px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                                    value: "{name_input}", oninput: move |e| name_input.set(e.value()),
                                    placeholder: "เช่น บ้านสวนยา" }
                                button {
                                    style: "padding:10px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                                    onclick: init_household, disabled: action_busy() || name_input().trim().is_empty(),
                                    "สร้างบ้าน" }
                            }
                        }
                    },
                    (Some(_), FetchState::Loaded(v)) => {
                        let thb_balance = v.lamports_sol.parse::<f64>().unwrap_or(0.0) * sol_thb();
                        let owner_pk = Pubkey::from_str(&v.owner).ok();
                        let wallet_pk = wallet_pubkey().and_then(|p| Pubkey::from_str(&p).ok());
                        let my_role = wallet_pk.and_then(|w| members().iter().find(|m| m.wallet == w).map(|m| m.role_tag));
                        let can_approve = matches!(my_role, Some(0) | Some(1));
                        let texts = item_texts();
                        let pending_rows: Vec<_> = match &*ledger.read() {
                            LedgerState::Loaded(rows) => rows.iter().filter(|r| r.status == "waiting").cloned().collect(),
                            _ => Vec::new(),
                        };
                        let other_rows: Vec<_> = match &*ledger.read() {
                            LedgerState::Loaded(rows) => rows.iter().filter(|r| r.status != "waiting").cloned().collect(),
                            _ => Vec::new(),
                        };
                        match view_mode() {
                            ViewMode::Home => rsx! {
                                if joined_from().is_some() {
                                    div { class: "banner banner-ok", "คุณถูกเชิญเข้าบ้าน ✓ ส่ง address ของคุณให้เจ้าบ้านกด Add member หลังเชื่อมกระเป๋าเงิน" }
                                }
                                div { style: "border:1px solid var(--border-strong);border-radius:14px;padding:20px;background:var(--surface);display:grid;gap:4px",
                                    span { style: "font-size:12px;color:var(--muted)", "💰 งบครัวเรือน" }
                                    span { style: "font-size:34px;font-weight:700", "฿{thb_balance:.2}" }
                                    span { style: "font-size:12px;color:var(--faint)", "≈ {v.lamports_sol} SOL · สมาชิก {v.member_count} คน" }
                                    button {
                                        style: "padding:8px 12px;border:none;border-radius:8px;background:var(--violet);color:white;font-weight:600;cursor:pointer;width:fit-content",
                                        onclick: move |_| view_mode.set(ViewMode::Assistant),
                                        "🤖 ให้ผู้ช่วยช่วยซื้อของ →"
                                    }
                                }
                                div { style: "display:flex;gap:8px;flex-wrap:wrap;margin-top:12px",
                                    for m in members() {
                                        div { style: "display:flex;gap:6px;align-items:center;padding:6px 10px;border-radius:9999px;border:1px solid var(--border);background:var(--surface)",
                                            span { "{role_emoji(m.role_tag)}" }
                                            span { style: "font-size:12px", "{contact_name(&m.wallet, &contacts(), &members())}" }
                                            if m.reward_points > 0 {
                                                span { style: "font-size:11px;color:var(--faint)", "{m.reward_points} pts" }
                                            }
                                        }
                                    }
                                }
                                if let LedgerState::Failed(e) = &*ledger.read() {
                                    div { class: "banner banner-err", "⚠ โหลดรายการจาก devnet ไม่สำเร็จ (เครือข่ายติดขัด) — รอสักครู่แล้วสลับแท็บกลับมาดูใหม่" }
                                }
                                if can_approve {
                                div { style: "border:1px dashed var(--border-strong);border-radius:12px;padding:14px;display:grid;gap:8px",
                                    span { style: "font-size:13px;font-weight:600", "🔗 เชิญสมาชิกเข้าบ้าน" }
                                    button {
                                        class: "copybtn",
                                        style: "padding:6px 10px;width:fit-content",
                                        onclick: move |_| {
                                            let owner_addr = v.owner.clone();
                                            if let Some(w) = web_sys::window() {
                                                let origin = w.location().origin().unwrap_or_default();
                                                let invite = format!("{origin}/?join={owner_addr}");
                                                let _ = w.navigator().clipboard().write_text(&invite);
                                                copied_invite.set(true);
                                                spawn(async move {
                                                    gloo_timers::future::TimeoutFuture::new(1600).await;
                                                    copied_invite.set(false);
                                                });
                                            }
                                        },
                                        if *copied_invite.read() { "✓ คัดลอกลิงก์เชิญแล้ว" } else { "คัดลอกลิงก์เชิญสมาชิก" }
                                    }
                                    p { style: "font-size:11px;color:var(--muted);margin:0",
                                        "สมาชิกใหม่เปิดลิงก์ กดเข้าใช้งาน แล้วส่ง address ของเขากลับมา — คุณกด Add member ในโหมดขั้นสูง"
                                    }
                                }
                                }
                                h2 { style: "margin:24px 0 8px;font-size:16px", "📋 รออนุมัติ ({pending_rows.len()})" }
                                if pending_rows.is_empty() {
                                    p { style: "color:var(--faint);font-size:13px", "ไม่มีคำขอรออนุมัติ — สบายใจได้ :)" }
                                }
                                for row in pending_rows {
                                    div { style: "border:1px solid var(--border);border-radius:12px;padding:14px;background:var(--surface);display:grid;gap:8px",
                                        div { style: "display:flex;justify-content:space-between;gap:8px",
                                            span { style: "font-size:14px;font-weight:600", "{item_text_of(&texts, row.request_id)}" }
                                            span { style: "font-size:14px;font-weight:700", "฿{row_amount_thb(&row.amount_sol, sol_thb()):.2}" }
                                        }
                                        span { style: "font-size:12px;color:var(--muted)", "ขอโดย {contact_name(&row.buyer_key, &contacts(), &members())}" }
                                        if can_approve {
                                            div { style: "display:flex;gap:8px",
                                                button {
                                                    style: "flex:1;padding:10px;border:none;border-radius:8px;background:var(--accent);color:white;font-weight:700;cursor:pointer",
                                                    onclick: move |_| {
                                                        let ctx = ctx;
                                                        let admin = owner_pk;
                                                        let caller = wallet_pk;
                                                        spawn(async move {
                                                            if let (Some(a), Some(c)) = (admin, caller) { ctx.do_approve(&a, &c, row.request_id).await; }
                                                        });
                                                    },
                                                    "✓ อนุมัติ"
                                                }
                                                button {
                                                    style: "flex:1;padding:10px;border:none;border-radius:8px;background:var(--danger-bg);color:var(--danger);font-weight:700;cursor:pointer",
                                                    onclick: move |_| {
                                                        let admin = owner_pk;
                                                        let caller = wallet_pk;
                                                        spawn(async move {
                                                            if let (Some(a), Some(c)) = (admin, caller) { ctx.do_reject(&a, &c, row.request_id, "ไม่อนุมัติจากหน้าหลัก").await; }
                                                        });
                                                    },
                                                    "✗ ไม่อนุมัติ"
                                                }
                                            }
                                        } else {
                                            span { style: "font-size:12px;color:var(--faint)", "รอผู้ใหญ่อนุมัติ" }
                                        }
                                    }
                                }
                                div { style: "border:1px dashed var(--border-strong);border-radius:12px;padding:14px;display:grid;gap:10px",
                                    div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:center",
                                        input {
                                            r#type: "file",
                                            id: "receipt-file",
                                            accept: "image/png,image/jpeg,image/webp",
                                            style: "font-size:12px",
                                        }
                                        button {
                                            style: "padding:8px 14px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                                            onclick: parse_receipt,
                                            disabled: *receipt_state.read() == "loading",
                                            if *receipt_state.read() == "loading" { "Reading…" } else { "Extract items" }
                                        }
                                        span { style: "font-size:11px;color:var(--muted)",
                                            "1 SOL ≈ ฿{sol_thb():.0}"
                                        }
                                    }
                                    match &*receipt_state.read() {
                                        s if s == "idle" => rsx! { p { style: "font-size:12px;color:var(--faint);margin:0", "ถ่ายใบเสร็จแล้วให้ AI แตกเป็นรายการ — กดเปิดคำขอได้เลย" } },
                                        s if s == "loading" => rsx! { p { style: "font-size:12px;color:var(--faint);margin:0", "กำลังอ่านใบเสร็จ…" } },
                                        s if s.starts_with("error:") => {
                                            let msg = s.trim_start_matches("error:");
                                            rsx! { p { style: "font-size:12px;color:var(--danger);margin:0", "⚠ {msg}" } }
                                        },
                                        _success => rsx! {
                                            p { style: "font-size:12px;color:var(--success);margin:0", "✓ แตกได้ {receipt_items().len()} รายการ — กดเปิดคำขอทีละรายการ" }
                                        },
                                    }
                                    for item in receipt_items() {
                                        div { style: "display:flex;gap:8px;align-items:center;flex-wrap:wrap;border:1px solid var(--border);border-radius:8px;padding:8px 10px",
                                            span { style: "flex:1;font-size:13px", "{item.name}" }
                                            span { style: "font-size:12px;color:var(--muted)", "×{item.quantity}" }
                                            span { style: "font-size:12px;font-family:ui-monospace,monospace", "{item.price_label()}" }
                                            button {
                                                style: "padding:6px 12px;border:none;border-radius:8px;background:var(--accent);color:white;font-size:12px;font-weight:600;cursor:pointer",
                                                onclick: move |_| open_from_receipt(item.clone()),
                                                disabled: action_busy() || wallet_pubkey().is_none(),
                                                "Open request"
                                            }
                                        }
                                    }
                                }
                                h2 { style: "margin:24px 0 8px;font-size:16px", "✅ กำลังดำเนินการ / เสร็จแล้ว" }
                                for row in other_rows {
                                    div { style: "border:1px solid var(--border);border-radius:12px;padding:14px;background:var(--surface);display:grid;gap:8px",
                                        div { style: "display:flex;justify-content:space-between;gap:8px",
                                            span { style: "font-size:14px;font-weight:600", "{item_text_of(&texts, row.request_id)}" }
                                            span { style: "font-size:12px;padding:2px 8px;border-radius:9999px {status_style(row.status)}", "{status_thai(row.status)}" }
                                        }
                                        span { style: "font-size:12px;color:var(--muted)", "ผู้ซื้อ: {contact_name(&row.buyer_key, &contacts(), &members())} · ฿{row_amount_thb(&row.amount_sol, sol_thb()):.2}" }
                                        if row.status == "approved" && wallet_pk == Some(row.buyer_key) {
                                            button {
                                                style: "padding:8px 12px;border:none;border-radius:8px;background:var(--accent);color:white;font-weight:600;cursor:pointer",
                                                onclick: move |_| {
                                                    let ctx = ctx;
                                                    let owner = owner_pk;
                                                    let buyer = wallet_pk;
                                                    spawn(async move {
                                                        if let (Some(o), Some(b)) = (owner, buyer) { ctx.do_confirm(&o, &b, row.request_id).await; }
                                                    });
                                                },
                                                "ฉันซื้อแล้ว ✓"
                                            }
                                        }
                                        if row.status == "restocked" && my_role == Some(0) {
                                            button {
                                                style: "padding:8px 12px;border:none;border-radius:8px;background:var(--teal);color:white;font-weight:600;cursor:pointer",
                                                onclick: move |_| {
                                                    let ctx = ctx;
                                                    let owner = owner_pk;
                                                    let caller = wallet_pk;
                                                    let buyer = row.buyer_key;
                                                    let amt = row.amount_lamports;
                                                    spawn(async move {
                                                        if let (Some(o), Some(c)) = (owner, caller) { ctx.do_reimburse(&o, &c, &buyer, row.request_id, amt).await; }
                                                    });
                                                },
                                                "จ่ายเงินคืน"
                                            }
                                        }
                                    }
                                }
                            },
                            ViewMode::Lists => rsx! {
                                for row in other_rows {
                                    div { style: "border:1px solid var(--border);border-radius:12px;padding:14px;background:var(--surface);display:grid;gap:6px;margin-bottom:10px",
                                        div { style: "display:flex;justify-content:space-between;gap:8px",
                                            span { style: "font-size:14px;font-weight:600", "{item_text_of(&texts, row.request_id)}" }
                                            span { style: "font-size:12px;padding:2px 8px;border-radius:9999px {status_style(row.status)}", "{status_thai(row.status)}" }
                                        }
                                        span { style: "font-size:12px;color:var(--muted)", "฿{row_amount_thb(&row.amount_sol, sol_thb()):.2} · ผู้ซื้อ {contact_name(&row.buyer_key, &contacts(), &members())}" }
                                    }
                                }
                                for row in pending_rows {
                                    div { style: "border:1px solid var(--border);border-radius:12px;padding:14px;background:var(--surface);display:grid;gap:6px;margin-bottom:10px",
                                        div { style: "display:flex;justify-content:space-between;gap:8px",
                                            span { style: "font-size:14px;font-weight:600", "{item_text_of(&texts, row.request_id)}" }
                                            span { style: "font-size:12px;padding:2px 8px;border-radius:9999px {status_style(row.status)}", "{status_thai(row.status)}" }
                                        }
                                        span { style: "font-size:12px;color:var(--muted)", "฿{row_amount_thb(&row.amount_sol, sol_thb()):.2} · ผู้ซื้อ {contact_name(&row.buyer_key, &contacts(), &members())}" }
                                    }
                                }
                            },
                            ViewMode::Assistant => rsx! {
                                div { style: "border:1px solid var(--border-strong);border-radius:12px;padding:16px;display:grid;gap:10px",
                                    div {
                                        id: "chat-box",
                                        if chat.len() == 0 {
                                            p { style: "font-size:12px;color:var(--faint);margin:0",
                                                "พิมพ์อะไรก็ได้ เช่น “ซื้อนม 2 ขวด งบ 50 บาท” — ผมช่วยเปิดคำขอให้เลย" }
                                        }
                                        for msg in chat() {
                                            div {
                                                class: if msg.role == "user" { "align-self:flex-end" } else { "align-self:flex-start" },
                                                style: if msg.role == "user" {
                                                    "max-width:85%;padding:8px 12px;border-radius:12px 12px 2px 12px;background:var(--accent);color:white;font-size:13px"
                                                } else {
                                                    "max-width:90%;padding:8px 12px;border-radius:12px 12px 12px 2px;background:var(--surface-2);color:var(--text);font-size:13px"
                                                },
                                                "{msg.text}"
                                                if !msg.items.is_empty() {
                                                    div { style: "display:flex;flex-wrap:wrap;gap:6px;margin-top:8px",
                                                        for item in msg.items {
                                                            span {
                                                                style: "padding:2px 8px;border-radius:6px;background:var(--surface-2);color:var(--ink-soft);font-size:11px;border:1px solid var(--border-strong)",
                                                                "{item.label()}"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        if chat_busy() {
                                            div { style: "align-self:flex-start;font-size:12px;color:var(--faint)", "กำลังคิด…" }
                                        }
                                    }
                                    div { style: "display:flex;gap:8px",
                                        input {
                                            style: "flex:1;padding:10px;border:1px solid var(--border-strong);border-radius:8px;font-size:13px",
                                            value: "{chat_input}",
                                            oninput: move |e| chat_input.set(e.value()),
                                            onkeydown: move |e| {
                                                if e.key() == Key::Enter {
                                                    spawn(send_chat_msg(chat, chat_input, chat_busy, state, ledger));
                                                }
                                            },
                                            placeholder: "พิมพ์ตรงนี้…",
                                            disabled: chat_busy(),
                                        }
                                        button {
                                            style: "padding:10px 16px;border:none;border-radius:8px;background:var(--violet);color:white;font-weight:600;cursor:pointer",
                                            onclick: send_chat,
                                            disabled: chat_busy() || chat_input().trim().is_empty(),
                                            "ส่ง"
                                        }
                                    }
                                }
                            },
                        }
                    },
                }}
                div { class: "tabbar",
                    button { class: if *view_mode.read() == ViewMode::Home { "active" } else { "" }, onclick: move |_| view_mode.set(ViewMode::Home), "🏠\nหน้าแรก" }
                    button { class: if *view_mode.read() == ViewMode::Lists { "active" } else { "" }, onclick: move |_| view_mode.set(ViewMode::Lists), "📋\nรายการ" }
                    button { class: if *view_mode.read() == ViewMode::Assistant { "active" } else { "" }, onclick: move |_| view_mode.set(ViewMode::Assistant), "🤖\nผู้ช่วย" }
                    button { onclick: move |_| advanced_mode.set(true), "⚙️\nขั้นสูง" }
                }
            }
            footer { style: "margin-top:40px;color:var(--faint);font-size:11px;border-top:1px solid var(--border);padding-top:12px",
                if *advanced_mode.read() {
                    "Privacy note: the household name never lands on-chain — the contract stores a blake3 fingerprint only, decoded here as hex. "
                    "Program: " {PROGRAM_ID}
                } else {
                    "Stocksie — ครัวเรือนของคุณบน Solana 🔷 โหมดขั้นสูงมีรายละเอียดเทคนิค"
                }
            }
        }
    }
}

#[component]
fn HouseholdCard(view: HouseholdView) -> Element {
    rsx! {
        div { style: "border:1px solid var(--border-strong);border-radius:12px;padding:20px;display:grid;gap:10px",
            h2 { style: "margin:0;font-size:18px", "Household " span { style: "color:var(--accent)", "✓ live on devnet" } }
            Row { label: "Household PDA".to_string(), value: view.pda.clone(), copy: Some(view.pda_key.to_string()) }
            Row { label: "Vault balance".to_string(), value: format!("{} SOL (account lamports)", view.lamports_sol), copy: None }
            Row { label: "Mirror field vault_balance".to_string(), value: format!("{} lamports", view.vault_balance_mirror), copy: None }
            Row { label: "Owner".to_string(), value: view.owner.clone(), copy: Some(view.owner.clone()) }
            Row { label: "Members".to_string(), value: view.member_count.to_string(), copy: None }
            Row { label: "Purchase requests opened".to_string(), value: view.request_counter.to_string(), copy: None }
            Row { label: "Rewards distributed".to_string(), value: format!("{} pts", view.total_rewards), copy: None }
            Row { label: "Created at slot".to_string(), value: view.created_slot.to_string(), copy: None }
            Row { label: "Name fingerprint (blake3, hex)".to_string(), value: view.name_hash_hex.clone(), copy: None }
        }
    }
}

#[component]
fn Row(label: String, value: String, copy: Option<String>) -> Element {
    let mut copied = use_signal(|| false);
    rsx! {
        div { style: "display:flex;justify-content:space-between;gap:12px;font-size:13px",
            span { style: "color:var(--muted)", "{label}" }
            div { style: "display:flex;gap:6px;align-items:center;min-width:0",
                span { style: "font-family:ui-monospace,monospace;word-break:break-all;text-align:right", "{value}" }
                if let Some(text) = copy {
                    button {
                        class: "copybtn",
                        title: "Copy to clipboard",
                        onclick: move |_| {
                            if let Some(w) = web_sys::window() {
                                let _ = w.navigator().clipboard().write_text(&text);
                            }
                            copied.set(true);
                            spawn(async move {
                                gloo_timers::future::TimeoutFuture::new(1200).await;
                                copied.set(false);
                            });
                        },
                        if copied() { "✓" } else { "copy" }
                    }
                }
            }
        }
    }
}

fn status_thai(status: &str) -> &'static str {
    match status {
        "waiting" => "รออนุมัติ",
        "approved" => "อนุมัติแล้ว",
        "restocked" => "ซื้อแล้ว",
        "reimbursed" => "จ่ายคืนแล้ว",
        "rejected" => "ไม่อนุมัติ",
        _ => "?",
    }
}

fn contact_name(pk: &Pubkey, contacts: &[Contact], members: &[MemberView]) -> String {
    let addr = pk.to_string();
    if let Some(c) = contacts.iter().find(|c| c.pk == addr) {
        return c.name.clone();
    }
    if let Some(m) = members.iter().find(|m| m.wallet == *pk) {
        return format!("{} {}", role_emoji(m.role_tag), role_name(m.role_tag));
    }
    format!("{}…{}", &addr[..6], &addr[addr.len() - 4..])
}

fn item_text_of(texts: &[(u64, String)], id: u64) -> String {
    texts
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, t)| t.clone())
        .unwrap_or_else(|| format!("คำขอ #{id}"))
}

fn row_amount_thb(amount_sol: &str, rate: f64) -> f64 {
    amount_sol.parse::<f64>().unwrap_or(0.0) * rate
}

fn status_style(status: &str) -> &'static str {
    match status {
        "waiting" => "background:var(--warn-bg);color:var(--warn-ink)",
        "approved" => "background:var(--info-bg);color:var(--info-ink)",
        "restocked" => "background:var(--violet-bg);color:var(--violet-ink)",
        "reimbursed" => "background:#dcfce7;color:var(--success)",
        "rejected" => "background:var(--danger-bg);color:var(--danger-ink)",
        _ => "background:var(--surface-2);color:var(--chip-ink-muted)",
    }
}

#[component]
fn LedgerTable(rows: Vec<RequestView>) -> Element {
    rsx! {
        div { style: "border:1px solid var(--border-strong);border-radius:12px;overflow:hidden",
            table { style: "width:100%;border-collapse:collapse;font-size:12px",
                thead {
                    tr { style: "background:var(--surface-3);text-align:left",
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
                        tr { style: "border-top:1px solid var(--border)",
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
                            td { style: "padding:8px 10px;color:var(--muted)", "slot {row.created_slot}" }
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
            amount_lamports: u64_at(80),
            status: status_name(bytes[152]),
            buyer: Pubkey::try_from(&bytes[40..72])
                .map(|p| {
                    let s = p.to_string();
                    format!("{}…{}", &s[..6], &s[s.len() - 4..])
                })
                .unwrap_or_else(|_| "decode error".into()),
            buyer_key: Pubkey::try_from(&bytes[40..72]).unwrap_or_default(),
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

/// POST a JSON body anywhere and parse the JSON reply — with automatic retry
/// (2 retries, backoff) for transient devnet failures (429 / NetworkError).
/// Errors map to friendly Thai for the UI.
async fn post_json(url: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let mut last_err = String::new();
    let mut attempt = 0;
    let resp = loop {
        if attempt > 0 {
            gloo_timers::future::TimeoutFuture::new(800 * (attempt as u32)).await;
        }
        let built = match Request::post(url).json(&body) {
            Ok(r) => r,
            Err(e) => return Err(format!("body encode failed: {e}")),
        };
        let sent = built.send().await;
        match sent {
            Err(e) => {
                last_err = format!("เครือข่ายติดขัด — ลองอีกครั้งในอีกสักครู่ ({e})");
            }
            Ok(resp) if resp.status() == 429 => {
                last_err = "devnet ติดขัด (rate limit) — ลองอีกครั้งในอีกสักครู่".into();
            }
            Ok(resp) if resp.status() != 200 => {
                return Err(format!("{} returned HTTP {}", url, resp.status()));
            }
            Ok(resp) => {
                let text = resp
                    .text()
                    .await
                    .map_err(|e| format!("read failed: {e}"))?;
                let parsed: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| format!("bad JSON from {url}: {e}"))?;
                break parsed;
            }
        }
        attempt += 1;
        if attempt >= 3 {
            return Err(last_err);
        }
    };
    let result = match &resp {
        v if v.get("error").is_some() => {
            return Err(format!("devnet RPC error: {}", v["error"]));
        }
        v => v.clone(),
    };
    Ok(result)
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
            &[programs::approve_purchase_request(&owner, &owner, 1, &program)],
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

        // 5. confirm_restock — signed BY the buyer (fee money: airdrop)
        rpc(
            LOCALNET,
            json!({"jsonrpc":"2.0","id":1,"method":"requestAirdrop","params":[buyer.to_string(), 200_000_000u64]}),
        );
        let mut buyer_funded = false;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let bal = rpc(
                LOCALNET,
                json!({"jsonrpc":"2.0","id":1,"method":"getBalance","params":[buyer.to_string()]}),
            );
            if bal["result"]["value"].as_u64().unwrap_or(0) >= 10_000 {
                buyer_funded = true;
                break;
            }
        }
        assert!(buyer_funded, "buyer airdrop did not land");
        let wire = tx::build_wire_tx(
            &buyer,
            &blockhash,
            &[programs::confirm_restock(&owner, &buyer, 1, programs::text_hash("฿48 actual"), &program)],
        );
        send_and_confirm(&buyer_wallet, &wire);
        let req_data = {
            let res = rpc(
                LOCALNET,
                json!({"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":[req.to_string(), {"encoding":"base64","commitment":"confirmed"}]}),
            );
            decode(res["result"]["value"]["data"][0].as_str().expect("b64"))
        };
        assert_eq!(req_data[152], 2, "status must be Restocked (2)");

        // 6. reimburse_buyer 0.005 SOL (≤ the 0.01 SOL limit) — by the owner
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::reimburse_buyer(&owner, &owner, &buyer, 1, 5_000_000, &program)],
        );
        send_and_confirm(&wallet, &wire);
        let res = rpc(
            LOCALNET,
            json!({"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":[req.to_string(), {"encoding":"base64","commitment":"confirmed"}]}),
        );
        let req_data = decode(res["result"]["value"]["data"][0].as_str().expect("b64"));
        assert_eq!(req_data[152], 3, "status must be Reimbursed (3)");
        assert_eq!(
            u64::from_le_bytes(req_data[201..209].try_into().unwrap()),
            5_000_000,
            "reimbursed_amount mirror"
        );

        // 7. award_reward 25 pts to the buyer (Owner authorizes)
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::award_reward(
                &owner,
                &owner,
                &buyer,
                25,
                programs::text_hash("helpful shopper"),
                &program,
            )],
        );
        send_and_confirm(&wallet, &wire);

        // 8. set_role buyer → Parent (1)
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::set_role(&owner, &owner, &buyer, 1, &program)],
        );
        send_and_confirm(&wallet, &wire);

        // 9. create request #2 (buyer is still an active Parent member)
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::create_purchase_request(
                &owner,
                &buyer,
                2,
                8_000_000,
                programs::text_hash("ผงซักฟอก 1 ถุง"),
                programs::text_hash("฿45 estimate"),
                &program,
            )],
        );
        send_and_confirm(&wallet, &wire);

        // 10. reject #2 (Owner, private reason hash)
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::reject_purchase_request(
                &owner,
                &owner,
                2,
                programs::text_hash("changed my mind"),
                &program,
            )],
        );
        send_and_confirm(&wallet, &wire);
        let req2 = programs::purchase_pda(&hh, 2, &program);
        let res = rpc(
            LOCALNET,
            json!({"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":[req2.to_string(), {"encoding":"base64","commitment":"confirmed"}]}),
        );
        let req2_data = decode(res["result"]["value"]["data"][0].as_str().expect("b64"));
        assert_eq!(req2_data[152], 4, "status must be Rejected (4)");

        // 11. close #2 — rent flows back to the caller
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::close_purchase_request(&owner, &owner, 2, &program)],
        );
        send_and_confirm(&wallet, &wire);
        let res = rpc(
            LOCALNET,
            json!({"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":[req2.to_string(), {"encoding":"base64","commitment":"confirmed"}]}),
        );
        assert!(
            res.get("result")
                .and_then(|r| r.get("value"))
                .map(|v| v.is_null())
                .unwrap_or(false),
            "closed request account must be reclaimed"
        );

        // 12. remove_member buyer (Owner only)
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::remove_member(&owner, &owner, &buyer, &program)],
        );
        send_and_confirm(&wallet, &wire);

        // 13. withdraw_funds 0.01 SOL back to the admin
        let acc = account(&program, &owner).expect("household before withdraw");
        let lamports_before = acc["lamports"].as_u64().unwrap();
        let wire = tx::build_wire_tx(
            &owner,
            &blockhash,
            &[programs::withdraw_funds(&owner, 10_000_000, &program)],
        );
        send_and_confirm(&wallet, &wire);
        let acc = account(&program, &owner).expect("household after withdraw");
        assert!(
            acc["lamports"].as_u64().unwrap() <= lamports_before - 10_000_000,
            "vault must drop by the withdrawn amount"
        );

        println!("E2E OK — full household lifecycle executed from Rust wire bytes");
    }
}
