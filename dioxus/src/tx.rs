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

/// One instruction with its resolved accounts — the input to
/// [`build_wire_tx`]. `writable`/`signer` come straight from the IDL.
#[derive(Clone, Debug)]
pub struct ProgramIx {
    pub program: Pubkey,
    /// (pubkey, writable, signer) in the order the instruction declares them.
    pub accounts: Vec<(Pubkey, bool, bool)>,
    /// Anchor data = 8-byte discriminator + borsh args.
    pub data: Vec<u8>,
}

/// The System Program id — `init`-bearing instructions always carry it.
pub fn system_program() -> Pubkey {
    Pubkey::from_str("11111111111111111111111111111111").expect("constant parses")
}

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
    let memo_ix = ProgramIx {
        program: memo_program,
        accounts: vec![],
        data: {
            let mut d = Vec::new();
            d.extend_from_slice(memo.as_bytes());
            d
        },
    };
    build_wire_tx(payer, blockhash, &[memo_ix])
}

/// Compile arbitrary instructions into legacy wire bytes
/// (`[sig_count][zeroed sig slots][message]`) exactly like
/// `@solana/web3.js`'s `Transaction.compileMessage` + serialize:
///
/// - the fee payer is the first key (signer + writable);
/// - all signers precede non-signers (first-appearance order within groups);
/// - account metas OR-merge when the same key appears in several places;
/// - instructions reference keys by index.
pub fn build_wire_tx(payer: &Pubkey, blockhash: &[u8; 32], ixs: &[ProgramIx]) -> Vec<u8> {
    #[derive(Clone, Copy, PartialEq)]
    struct Meta {
        signer: bool,
        writable: bool,
    }
    let mut keys: Vec<Pubkey> = vec![*payer];
    let mut metas: Vec<Meta> = vec![Meta {
        signer: true,
        writable: true,
    }];

    let mut push_key =
        |keys: &mut Vec<Pubkey>, metas: &mut Vec<Meta>, pk: Pubkey, m: Meta| -> usize {
            if let Some(i) = keys.iter().position(|k| *k == pk) {
                let existing = metas[i];
                metas[i] = Meta {
                    signer: existing.signer || m.signer,
                    writable: existing.writable || m.writable,
                };
                i
            } else {
                keys.push(pk);
                metas.push(m);
                keys.len() - 1
            }
        };

    let mut compiled: Vec<(usize, usize, Vec<u8>, usize)> = Vec::new(); // (program_idx, acct_idx_start, data, data_len)
    let mut compiled_accounts: Vec<Vec<usize>> = Vec::new();
    for ix in ixs {
        let program_idx = push_key(
            &mut keys,
            &mut metas,
            ix.program,
            Meta {
                signer: false,
                writable: false,
            },
        );
        let mut idxs = Vec::with_capacity(ix.accounts.len());
        for (pk, writable, signer) in &ix.accounts {
            let i = push_key(
                &mut keys,
                &mut metas,
                *pk,
                Meta {
                    signer: *signer,
                    writable: *writable,
                },
            );
            idxs.push(i);
        }
        compiled_accounts.push(idxs);
        compiled.push((program_idx, 0, ix.data.clone(), ix.data.len()));
    }

    // signers must precede non-signers: stable partition, payer stays first.
    let payer_meta = metas[0];
    let mut order: Vec<usize> = (0..keys.len()).collect();
    // web3.js key ordering (the runtime reads writability by POSITION: the
    // last `readonlyUnsigned` keys are readonly!): stable 3-way sort —
    // signers first, then writable non-signers, then readonly non-signers.
    order.sort_by_key(|&i| {
        let m = metas[i];
        if m.signer {
            0
        } else if m.writable {
            1
        } else {
            2
        }
    });
    let mut remap = vec![0usize; keys.len()];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = new;
    }
    let sorted_keys: Vec<Pubkey> = order.iter().map(|&i| keys[i]).collect();
    let sorted_metas: Vec<Meta> = order.iter().map(|&i| metas[i]).collect();
    debug_assert_eq!(sorted_keys[0], *payer, "fee payer must sort first");
    debug_assert!(sorted_metas[0].signer);

    let mut msg = Vec::new();
    let required_sigs = sorted_metas.iter().filter(|m| m.signer).count();
    let readonly_signed = sorted_metas
        .iter()
        .filter(|m| m.signer && !m.writable)
        .count();
    let readonly_unsigned = sorted_metas
        .iter()
        .filter(|m| !m.signer && !m.writable)
        .count();
    msg.push(required_sigs as u8);
    msg.push(readonly_signed as u8);
    msg.push(readonly_unsigned as u8);
    put_shortvec(&mut msg, sorted_keys.len());
    for k in &sorted_keys {
        msg.extend_from_slice(k.as_ref());
    }
    msg.extend_from_slice(blockhash);
    put_shortvec(&mut msg, ixs.len());
    for ((program_idx, _, data, data_len), idxs) in compiled.iter().zip(&compiled_accounts) {
        msg.push(remap[*program_idx] as u8);
        put_shortvec(&mut msg, idxs.len());
        for i in idxs {
            msg.push(remap[*i] as u8);
        }
        put_shortvec(&mut msg, *data_len);
        msg.extend_from_slice(data);
    }

    let mut wire = Vec::with_capacity(msg.len() + 65);
    wire.push(required_sigs as u8);
    wire.extend_from_slice(&vec![0u8; 64 * required_sigs.max(1)]);
    wire.extend_from_slice(&msg);
    let _ = payer_meta;
    wire
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
