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
// Not yet in Rust (the real RIIR frontier, see README): Wallet Standard
// interop for signing, the purchase-request ledger, the AI agent. The
// Next.js app remains the production surface — this crate is additive.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use dioxus::prelude::*;
use gloo_net::http::Request;
use serde_json::json;
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Stocksie program (deployed on devnet — see docs/DEPLOY_DEVNET.md).
const PROGRAM_ID: &str = "At2vd5Mqd9xcHRFAZin1VHb6upEsF1GCXoFG19UHsQoj";

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

fn app() -> Element {
    let mut owner_input = use_signal(|| DEMO_OWNERS[0].0.to_string());
    let mut state = use_signal(|| FetchState::Idle);

    let fetch = move |owner_raw: String| {
        let owner_raw = owner_raw.trim().to_string();
        spawn(async move {
            state.set(FetchState::Loading);
            state.set(match load_household(&owner_raw).await {
                Ok(v) => FetchState::Loaded(Box::new(v)),
                Err(e) => FetchState::Failed(e),
            });
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

/// Minimal JSON-RPC wrapper: getAccountInfo with base64 encoding.
async fn rpc_get_account(pubkey: &Pubkey) -> Result<serde_json::Value, String> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [pubkey.to_string(), {"encoding": "base64", "commitment": "confirmed"}]
    });
    let resp = Request::post("https://api.devnet.solana.com")
        .json(&body)
        .map_err(|e| format!("RPC body encode failed: {e}"))?
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {e}"))?;
    if resp.status() != 200 {
        return Err(format!("RPC returned HTTP {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| format!("RPC read failed: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("bad JSON from RPC: {e}"))?;
    if let Some(err) = value.get("error") {
        return Err(format!("devnet RPC error: {err}"));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| format!("unexpected RPC response: {}", &text[..text.len().min(160)]))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
