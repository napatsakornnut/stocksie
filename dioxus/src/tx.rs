// Legacy Solana transaction wire-format builder (plan 007 — wallet signing).
//
// Wallet Standard wallets sign SERIALIZED transactions: the app must produce
// the exact wire bytes @solana/web3.js would — `[sig_count][64 zero sig
// slots][message]`. Verified byte-for-byte against web3.js `serializeMessage`
// (golden test below); the wallet fills the signature slot, so Rust never
// touches ed25519.
//
// Layout reference: Solana docs "Anatomy of a Transaction" —
//   message = header(3) | account keys | blockhash(32) | instructions
//   compact-u16 ("shortvec") for all counts/lengths.

use solana_pubkey::Pubkey;
use std::str::FromStr;

/// SPL Memo v1 — takes no accounts, so the instruction needs nothing but the
/// program id and data; the fee payer is the only signer in the message.
const MEMO_V1: &str = "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo";

/// Solana's compact-u16 ("shortvec"): little-endian 7-bit groups, high bit =
/// more bytes follow.
fn put_shortvec(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let mut byte = (n & 0x7f) as u8;
        n >>= 7;
        if n > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if n == 0 {
            break;
        }
    }
}

/// Build the wire bytes of a legacy memo transaction with the signature slot
/// zeroed, ready for `signAndSendTransaction` on an injected provider.
pub fn build_memo_tx(payer: &Pubkey, blockhash: &[u8; 32], memo: &str) -> Vec<u8> {
    let memo_program = Pubkey::from_str(MEMO_V1).expect("constant parses");

    let mut msg = Vec::new();
    // message header: [required sigs, readonly signed, readonly unsigned]
    // payer = writable signer; memo program = readonly unsigned
    msg.extend_from_slice(&[1, 0, 1]);
    put_shortvec(&mut msg, 2);
    msg.extend_from_slice(payer.as_ref());
    msg.extend_from_slice(memo_program.as_ref());
    msg.extend_from_slice(blockhash);
    // instructions: one memo — programIdx 1, no accounts, utf8 data
    put_shortvec(&mut msg, 1);
    msg.push(1);
    put_shortvec(&mut msg, 0);
    let data = memo.as_bytes();
    put_shortvec(&mut msg, data.len());
    msg.extend_from_slice(data);

    let mut tx = Vec::with_capacity(msg.len() + 65);
    tx.push(1);
    tx.extend_from_slice(&[0u8; 64]);
    tx.extend_from_slice(&msg);
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Golden fixture generated with @solana/web3.js 1.98.4:
    /// `serializeMessage()` of a legacy memo tx (empty account list, fee
    /// payer 98Kk…pnod, zero blockhash) prepended by [sig_count=1][64 zeros].
    /// Re-generate with scripts/make-golden-tx.mjs if the memo text changes.
    #[test]
    fn matches_web3js_serialization() {
        let payer = Pubkey::from_str("98KkCG8KnhvkFbbEPx5JkpHpwzDbJKxuzBGK38Fcpnod").unwrap();
        let blockhash = [0u8; 32];
        let tx = build_memo_tx(&payer, &blockhash, "GM from Stocksie RIIR");
        assert_eq!(
            hex(&tx),
            "01000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100010278bdc17c6694649896e5d8dcb188deb2d1a70c9f597515c0dcefaa40668fa7ec054a5350f85dc882d614a55672788a296ddf1eababd0a60678884932f4eef6a0000000000000000000000000000000000000000000000000000000000000000001010015474d2066726f6d2053746f636b7369652052494952"
        );
    }

    #[test]
    fn shortvec_matches_compact_u16() {
        let mut out = Vec::new();
        put_shortvec(&mut out, 0);
        put_shortvec(&mut out, 127);
        put_shortvec(&mut out, 128);
        put_shortvec(&mut out, 16_500);
        // 16500 = 116 | (0 << 7) | (1 << 14)
        assert_eq!(out, vec![0x00, 0x7f, 0x80, 0x01, 0xf4, 0x80, 0x01]);
    }
}
