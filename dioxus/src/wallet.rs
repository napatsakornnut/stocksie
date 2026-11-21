// Wallet Standard interop (plan 007 signing slice).
//
// Phantom (and friends) inject a provider object on `window.phantom.solana`.
// From Rust we reach it through js-sys reflection, await its promises with
// wasm-bindgen-futures, and hand it pre-built wire bytes (see tx.rs) — the
// wallet fills the zeroed signature slot and submits the transaction itself.

use js_sys::{Function, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

fn js_err_text(e: &JsValue) -> String {
    e.dyn_ref::<js_sys::Error>()
        .map(|err| err.to_string().into())
        .unwrap_or_else(|| format!("{e:?}"))
}

fn get(obj: &JsValue, key: &str) -> Result<JsValue, String> {
    Reflect::get(obj, &JsValue::from_str(key))
        .map_err(|e| format!("property `{key}`: {}", js_err_text(&e)))
}

fn js_str(v: &JsValue) -> String {
    v.as_string().unwrap_or_default()
}

/// Detect an injected Solana provider: `window.phantom.solana` first, then a
/// generic `window.solana` (Solflare and others).
pub fn detect_provider() -> Result<JsValue, String> {
    let global: JsValue = js_sys::global().into();
    for path in [&["phantom", "solana"][..], &["solana"][..]] {
        let mut cur = global.clone();
        let mut found = true;
        for key in path {
            match get(&cur, key) {
                Ok(v) if !v.is_undefined() && !v.is_null() => cur = v,
                _ => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            return Ok(cur);
        }
    }
    Err(
        "No injected Solana wallet found (window.phantom). Open this page in \
         Chrome/Brave with the Phantom extension to try signing."
            .into(),
    )
}

/// `provider.connect()` → base58 public key of the selected account.
pub async fn connect(provider: &JsValue) -> Result<String, String> {
    let connect = get(provider, "connect")?;
    let f: Function = connect
        .dyn_into()
        .map_err(|_| "`connect` is not a function".to_string())?;
    let promise = f
        .call0(provider)
        .map_err(|e| format!("connect() rejected: {}", js_err_text(&e)))?;
    let result = JsFuture::from(promise.unchecked_into::<js_sys::Promise>())
        .await
        .map_err(|e| format!("connect(): {}", js_err_text(&e)))?;
    let pk = get(&result, "publicKey")?;
    let to_string = get(&pk, "toString")?;
    let f: Function = to_string
        .dyn_into()
        .map_err(|_| "publicKey.toString missing".to_string())?;
    f.call0(&pk)
        .map(|v| js_str(&v))
        .map_err(|e| format!("toString(): {}", js_err_text(&e)))
}

/// `provider.signAndSendTransaction({ transaction: bytes })` → signature
/// string. The wallet fills the zeroed signature slot and submits the tx to
/// the cluster itself.
pub async fn sign_and_send(provider: &JsValue, wire: &[u8]) -> Result<String, String> {
    let method = get(provider, "signAndSendTransaction")?;
    let f: Function = method.dyn_into().map_err(|_| {
        "`signAndSendTransaction` is not a function — the injected wallet is \
         too old for the serialized-bytes API"
            .to_string()
    })?;
    let opts = Object::new();
    Reflect::set(
        &opts,
        &JsValue::from_str("transaction"),
        &Uint8Array::from(wire).into(),
    )
    .map_err(|e| format!("option set failed: {}", js_err_text(&e)))?;
    let promise = f
        .call1(provider, &opts.into())
        .map_err(|e| format!("signAndSendTransaction() rejected: {}", js_err_text(&e)))?;
    let result = JsFuture::from(promise.unchecked_into::<js_sys::Promise>())
        .await
        .map_err(|e| format!("signAndSendTransaction(): {}", js_err_text(&e)))?;
    let sig = get(&result, "signature")?;
    Ok(js_str(&sig))
}
