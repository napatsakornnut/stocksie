// Paced, resumable devnet buffer writer for the Stocksie program.
//
// Public devnet RPC rate-limits aggressively (429 on HTTP and WS), which is
// what kills `solana program deploy`'s write burst. This script walks the
// same on-chain wire under the rate ceiling:
//
//   - reuse an existing Buffer account (pass its pubkey) or create one
//   - diff on-chain bytes vs the local ELF, write ONLY wrong 900-byte chunks
//   - sliding window of 4 sends, batched getSignatureStatuses confirms,
//     exponential backoff (up to 8s) on every 429
//   - final byte-diff must be exact before printing BUFFER_READY
//
// Wire facts (solana-program 1.18.26): both instructions are new_with_bincode
// → data = u32 variant LE + fields, and Write's Vec<u8> has an 8-byte bincode
// length prefix. Write = variant 1, accounts [buffer(w), authority(ro+signer)].
// (Variant 4 is SetAuthority — writing garbage there bricks the buffer.)
//
// Finalize with the CLI once BUFFER_READY:
//   solana program deploy --url devnet --buffer <BUFFER> \
//     --program-id target/deploy/stocksie-keypair.json \
//     --with-compute-unit-price 100000 target/deploy/stocksie.so

const {
  PublicKey,
  Connection,
  Keypair,
  Transaction,
  TransactionInstruction,
  SystemProgram,
  ComputeBudgetProgram,
} = require("@solana/web3.js");
const fs = require("fs");

const RPC = "https://api.devnet.solana.com";
const BPF_LOADER = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");
const ELF_PATH = process.argv[2];
const BUFFER_ARG = process.argv[3] || "";
const MICRO_LAMPORTS = 100000;
const HEADER = 37;
const CHUNK = 900;

const elf = fs.readFileSync(ELF_PATH);
const authority = Keypair.fromSecretKey(
  Buffer.from(JSON.parse(fs.readFileSync(process.env.HOME + "/.config/solana/id.json", "utf8")))
);
const connection = new Connection(RPC, "processed");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Every RPC call goes through here: 429 → doubling backoff, capped, patient.
async function rpc(fn, label) {
  let delay = 600;
  for (let attempt = 1; attempt <= 12; attempt++) {
    try {
      return await fn();
    } catch (e) {
      const is429 = /429|Too many/i.test(String(e?.message));
      if (!is429 || attempt === 12) throw e;
      await sleep(delay);
      delay = Math.min(delay * 2, 8000);
    }
  }
}

function u32(n) {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n, 0);
  return b;
}

function ixWrite(offset, bytes) {
  const len = Buffer.alloc(8);
  len.writeBigUInt64LE(BigInt(bytes.length), 0);
  return new TransactionInstruction({
    programId: BPF_LOADER,
    keys: [
      { pubkey: null, isSigner: false, isWritable: true }, // placeholder
      { pubkey: authority.publicKey, isSigner: true, isWritable: false },
    ],
    data: Buffer.concat([u32(1), u32(offset), len, Buffer.from(bytes)]),
  });
}

function signAndSend(ixs, extraSigners = []) {
  return rpc(async () => {
    const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash();
    const tx = new Transaction().add(
      ComputeBudgetProgram.setComputeUnitPrice({ microLamports: MICRO_LAMPORTS }),
      ...ixs
    );
    tx.recentBlockhash = blockhash;
    tx.feePayer = authority.publicKey;
    tx.sign(authority, ...extraSigners);
    const sig = await connection.sendRawTransaction(tx.serialize(), { skipPreflight: true });
    return { sig, lastValidBlockHeight };
  }, "send");
}

async function main() {
  let buffer;
  if (BUFFER_ARG) {
    buffer = new PublicKey(BUFFER_ARG);
    console.log(`resuming buffer ${buffer.toBase58()}`);
  } else {
    buffer = Keypair.generate();
    const space = HEADER + elf.length;
    const rent = await connection.getMinimumBalanceForRentExemption(space);
    console.log(`creating buffer ${buffer.publicKey.toBase58()} (rent ${(rent / 1e9).toFixed(4)} SOL)…`);
    const { sig } = await signAndSend([
      SystemProgram.createAccount({
        fromPubkey: authority.publicKey,
        newAccountPubkey: buffer.publicKey,
        lamports: rent,
        space,
        programId: BPF_LOADER,
      }),
      // InitializeBuffer = variant 0; accounts [buffer(w), authority(ro)]
      new TransactionInstruction({
        programId: BPF_LOADER,
        keys: [
          { pubkey: buffer.publicKey, isSigner: false, isWritable: true },
          { pubkey: authority.publicKey, isSigner: false, isWritable: false },
        ],
        data: Buffer.from([0, 0, 0, 0]),
      }),
    ], [buffer]);
    await rpc(() => connection.confirmTransaction(sig, "confirmed"));
  }

  // Diff loop: write wrong 900B chunks until the buffer matches exactly.
  for (let pass = 1; pass <= 8; pass++) {
    const info = await rpc(() => connection.getAccountInfo(buffer), "fetch");
    if (!info) throw new Error("buffer account missing");
    const onChain = Buffer.from(info.data.subarray(HEADER));
    const bad = [];
    for (let at = 0; at < elf.length; at += CHUNK) {
      const len = Math.min(CHUNK, elf.length - at);
      for (let j = 0; j < len; j++) {
        if (onChain[at + j] !== elf[at + j]) {
          bad.push([at, len]);
          break;
        }
      }
    }
    if (bad.length === 0) {
      console.log("BUFFER_READY (bytes identical)");
      console.log(`BUFFER_ADDRESS ${buffer.toBase58()}`);
      return;
    }
    console.log(`pass ${pass}: ${bad.length} chunks differ — writing…`);

    // sliding window of 4, batched status confirms; entries older than 60s
    // are requeued (their tx was likely dropped — null status forever)
    let queue = [...bad];
    let inflight = new Map(); // sig → { off, len, lastValidBlockHeight, sentAt }
    let lastStatusPoll = 0;
    while (queue.length > 0 || inflight.size > 0) {
      while (queue.length > 0 && inflight.size < 4) {
        const [off, len] = queue.shift();
        const ix = ixWrite(off, elf.subarray(off, off + len));
        ix.keys[0].pubkey = buffer;
        try {
          const { sig, lastValidBlockHeight } = await signAndSend([ix]);
          inflight.set(sig, { off, len, lastValidBlockHeight, sentAt: Date.now() });
        } catch (e) {
          console.log(`send failed (${e.message.slice(0, 80)}) — will retry chunk`);
          queue.push([off, len]);
          await sleep(2000);
        }
      }
      if (inflight.size > 0 && Date.now() - lastStatusPoll > 1500) {
        const now = Date.now();
        for (const [sig, meta] of [...inflight]) {
          if (now - meta.sentAt > 60_000) {
            console.log(`\nchunk at ${meta.off} stalled 60s — requeue`);
            inflight.delete(sig);
            queue.push([meta.off, meta.len]);
          }
        }
        const sigs = [...inflight.keys()];
        const statuses = await rpc(
          () => connection.getSignatureStatuses(sigs, { searchTransactionHistory: false }),
          "statuses"
        );
        for (let k = 0; k < sigs.length; k++) {
          const st = statuses?.value?.[k];
          if (!st) continue;
          const meta = inflight.get(sigs[k]);
          inflight.delete(sigs[k]);
          if (st.err) {
            console.log(`chunk at ${meta.off} failed on-chain (${JSON.stringify(st.err)}) — requeue`);
            queue.push([meta.off, meta.len]);
          } else {
            process.stdout.write(`.`);
          }
        }
        lastStatusPoll = Date.now();
      }
      await sleep(250);
    }
    console.log(`\npass ${pass} complete`);
  }
  throw new Error("buffer still differs after 8 passes");
}

main().catch((e) => {
  console.error("FAILED:", e.message?.slice?.(0, 200));
  process.exit(1);
});
