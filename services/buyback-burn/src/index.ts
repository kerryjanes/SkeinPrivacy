// Weft SOL buyback & burn worker.
//
// Every cycle it spends the buyback wallet's SOL (above a fee reserve) to buy $WEFT on
// Jupiter, then splits the bought $WEFT: a node-reward share is forwarded to the node
// payout wallet (topping up node rewards for SOL-paid traffic), and the remainder is
// burned on-chain (BurnChecked). Fully automatic and verifiable on-chain.
//
// SAFETY: does nothing unless WEFT_BUYBACK_ENABLE=1. It only touches its own wallet's
// SOL/$WEFT — it cannot affect the core program, escrow, or node payouts beyond funding
// them. Burns are capped at the wallet's own balance, so it can never overspend.
//
// Jupiter has no devnet liquidity, so the live swap can only be smoke-tested on mainnet
// with a small real-SOL amount before the feature is announced.

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import {
  Connection,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  TransactionMessage,
  VersionedTransaction,
} from '@solana/web3.js';
import {
  createAssociatedTokenAccountIdempotentInstruction,
  createBurnCheckedInstruction,
  createTransferCheckedInstruction,
  getAccount,
  getAssociatedTokenAddressSync,
  getMint,
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
} from '@solana/spl-token';

const SOL_MINT = 'So11111111111111111111111111111111111111112';

function env(key: string, fallback?: string): string {
  const v = process.env[key];
  if (v === undefined || v === '') {
    if (fallback !== undefined) return fallback;
    throw new Error(`${key} is required`);
  }
  return v;
}

function loadKeypair(path: string): Keypair {
  return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(path, 'utf8')) as number[]));
}

const log = (m: string) => console.log(`[buyback] ${new Date().toISOString()} ${m}`);

// Bounded confirmation — polls signature status up to timeoutMs instead of the
// deprecated confirmTransaction(sig) overload (which can hang a cycle indefinitely).
async function confirmSig(conn: Connection, sig: string, timeoutMs = 90_000): Promise<void> {
  const start = Date.now();
  for (;;) {
    const { value } = await conn.getSignatureStatus(sig, { searchTransactionHistory: false });
    if (value) {
      if (value.err) throw new Error(`tx ${sig} failed: ${JSON.stringify(value.err)}`);
      if (value.confirmationStatus === 'confirmed' || value.confirmationStatus === 'finalized') {
        return;
      }
    }
    if (Date.now() - start > timeoutMs)
      throw new Error(`tx ${sig} not confirmed in ${timeoutMs}ms`);
    await new Promise((r) => setTimeout(r, 1500));
  }
}

// ---- config ----
const enabled = process.env.WEFT_BUYBACK_ENABLE === '1';
const rpcUrl = env('WEFT_RPC');
const mint = new PublicKey(env('WEFT_MINT'));
const buyback = loadKeypair(env('WEFT_BUYBACK_KEYPAIR'));
const intervalMs = Number(env('WEFT_BUYBACK_INTERVAL_MS', '600000')); // 10 min
const reserveSol = Number(env('WEFT_BUYBACK_RESERVE_SOL', '0.03')); // keep for fees
const minSol = Number(env('WEFT_BUYBACK_MIN_SOL', '0.05')); // don't buy dust
const maxSolPerCycle = Number(env('WEFT_BUYBACK_MAX_SOL', '5'));
const nodeBps = Number(env('WEFT_BUYBACK_NODE_BPS', '7000')); // 70% to node payout pool
const burnBps = 10_000 - nodeBps; // remainder burned
const slippageBps = Number(env('WEFT_BUYBACK_SLIPPAGE_BPS', '150'));
const jupBase = env('WEFT_JUP_API', 'https://lite-api.jup.ag/swap/v1');
const storePath = env('WEFT_BUYBACK_STORE', '/var/lib/weft/buyback.json');
const payoutWallet = nodeBps > 0 ? new PublicKey(env('WEFT_PAYOUT_WALLET')) : null; // DEg6vvw node-payout pool

if (nodeBps < 0 || nodeBps > 10_000) throw new Error('WEFT_BUYBACK_NODE_BPS must be 0..10000');

const connection = new Connection(rpcUrl, 'confirmed');
// The reward mint may be classic SPL or Token-2022 (pump.fun mints Token-2022). Resolved
// from the mint account owner in main() before any cycle runs; used for every ATA/burn/transfer.
let tokenProgram = TOKEN_PROGRAM_ID;

// ---- persistent stats (bigint stored as strings) ----
interface Store {
  cumulativeBurned: bigint;
  cumulativeToNodes: bigint;
  cumulativeSolSpent: bigint; // lamports
  cycles: number;
}
function loadStore(): Store {
  if (existsSync(storePath)) {
    try {
      const j = JSON.parse(readFileSync(storePath, 'utf8'));
      return {
        cumulativeBurned: BigInt(j.cumulativeBurned ?? '0'),
        cumulativeToNodes: BigInt(j.cumulativeToNodes ?? '0'),
        cumulativeSolSpent: BigInt(j.cumulativeSolSpent ?? '0'),
        cycles: Number(j.cycles ?? 0),
      };
    } catch {
      /* fall through to fresh */
    }
  }
  return { cumulativeBurned: 0n, cumulativeToNodes: 0n, cumulativeSolSpent: 0n, cycles: 0 };
}
const store = loadStore();
function saveStore() {
  writeFileSync(
    storePath,
    JSON.stringify(
      {
        cumulativeBurned: store.cumulativeBurned.toString(),
        cumulativeToNodes: store.cumulativeToNodes.toString(),
        cumulativeSolSpent: store.cumulativeSolSpent.toString(),
        cycles: store.cycles,
        updatedAt: new Date().toISOString(),
      },
      null,
      2,
    ),
  );
}

// ---- Jupiter ----
async function jupQuote(amountLamports: bigint): Promise<unknown> {
  const url =
    `${jupBase}/quote?inputMint=${SOL_MINT}&outputMint=${mint.toBase58()}` +
    `&amount=${amountLamports.toString()}&slippageBps=${slippageBps}`;
  const r = await fetch(url);
  if (!r.ok) throw new Error(`jupiter quote HTTP ${r.status}: ${await r.text()}`);
  return r.json();
}

async function jupSwapAndSend(quote: unknown): Promise<string> {
  const r = await fetch(`${jupBase}/swap`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      quoteResponse: quote,
      userPublicKey: buyback.publicKey.toBase58(),
      wrapAndUnwrapSol: true,
      dynamicComputeUnitLimit: true,
      prioritizationFeeLamports: 'auto',
    }),
  });
  if (!r.ok) throw new Error(`jupiter swap HTTP ${r.status}: ${await r.text()}`);
  const { swapTransaction } = (await r.json()) as { swapTransaction: string };
  const tx = VersionedTransaction.deserialize(Buffer.from(swapTransaction, 'base64'));
  tx.sign([buyback]);
  const sig = await connection.sendRawTransaction(tx.serialize(), { maxRetries: 3 });
  await confirmSig(connection, sig);
  return sig;
}

// ---- balances / split ----
async function ownedWeft(): Promise<bigint> {
  const ata = getAssociatedTokenAddressSync(mint, buyback.publicKey, false, tokenProgram);
  try {
    return (await getAccount(connection, ata, undefined, tokenProgram)).amount;
  } catch {
    return 0n;
  }
}

async function splitAndBurn(
  decimals: number,
): Promise<{ burned: bigint; toNodes: bigint; sig?: string }> {
  const bal = await ownedWeft();
  if (bal <= 0n) return { burned: 0n, toNodes: 0n };
  const toNodes = payoutWallet ? (bal * BigInt(nodeBps)) / 10_000n : 0n;
  const toBurn = bal - toNodes;
  const srcAta = getAssociatedTokenAddressSync(mint, buyback.publicKey, false, tokenProgram);
  const ixs = [];
  if (toNodes > 0n && payoutWallet) {
    const destAta = getAssociatedTokenAddressSync(mint, payoutWallet, false, tokenProgram);
    ixs.push(
      createAssociatedTokenAccountIdempotentInstruction(
        buyback.publicKey,
        destAta,
        payoutWallet,
        mint,
        tokenProgram,
      ),
    );
    ixs.push(
      createTransferCheckedInstruction(
        srcAta,
        mint,
        destAta,
        buyback.publicKey,
        toNodes,
        decimals,
        [],
        tokenProgram,
      ),
    );
  }
  if (toBurn > 0n) {
    ixs.push(
      createBurnCheckedInstruction(
        srcAta,
        mint,
        buyback.publicKey,
        toBurn,
        decimals,
        [],
        tokenProgram,
      ),
    );
  }
  if (ixs.length === 0) return { burned: 0n, toNodes: 0n };
  const { blockhash } = await connection.getLatestBlockhash('confirmed');
  const msg = new TransactionMessage({
    payerKey: buyback.publicKey,
    recentBlockhash: blockhash,
    instructions: ixs,
  }).compileToV0Message();
  const tx = new VersionedTransaction(msg);
  tx.sign([buyback]);
  const sig = await connection.sendRawTransaction(tx.serialize(), { maxRetries: 3 });
  await confirmSig(connection, sig);
  return { burned: toBurn, toNodes, sig };
}

async function cycle(decimals: number): Promise<void> {
  // 1. Buy $WEFT with spendable SOL (above the reserve, capped per cycle).
  const lamports = await connection.getBalance(buyback.publicKey);
  const sol = lamports / LAMPORTS_PER_SOL;
  const spendableSol = Math.min(sol - reserveSol, maxSolPerCycle);
  if (spendableSol >= minSol) {
    const amountLamports = BigInt(Math.floor(spendableSol * LAMPORTS_PER_SOL));
    const quote = await jupQuote(amountLamports);
    const sig = await jupSwapAndSend(quote);
    store.cumulativeSolSpent += amountLamports;
    log(`bought ~${spendableSol.toFixed(4)} SOL of $WEFT — tx ${sig}`);
  } else {
    log(`skip buy: ${sol.toFixed(4)} SOL on hand, need > ${(reserveSol + minSol).toFixed(4)}`);
  }

  // 2. Split & burn whatever $WEFT is now in the wallet (self-heals any leftover).
  const res = await splitAndBurn(decimals);
  if (res.burned > 0n || res.toNodes > 0n) {
    store.cumulativeBurned += res.burned;
    store.cumulativeToNodes += res.toNodes;
    store.cycles += 1;
    saveStore();
    log(
      `split: burned ${res.burned} / to-nodes ${res.toNodes} (base units) — tx ${res.sig}. ` +
        `lifetime burned ${store.cumulativeBurned}`,
    );
  }
}

async function main(): Promise<void> {
  if (!enabled) {
    log('disabled — set WEFT_BUYBACK_ENABLE=1 to run. Exiting.');
    return;
  }
  // Detect the mint's owning token program (classic SPL vs Token-2022) once at startup.
  const mintOwner = (await connection.getAccountInfo(mint))?.owner;
  if (mintOwner?.equals(TOKEN_2022_PROGRAM_ID)) tokenProgram = TOKEN_2022_PROGRAM_ID;
  const decimals = (await getMint(connection, mint, undefined, tokenProgram)).decimals;
  const bal = (await connection.getBalance(buyback.publicKey)) / LAMPORTS_PER_SOL;
  log(
    `worker up: wallet ${buyback.publicKey.toBase58()} (${bal.toFixed(4)} SOL), mint ${mint.toBase58()} ` +
      `(${decimals} dec, ${tokenProgram.equals(TOKEN_2022_PROGRAM_ID) ? 'Token-2022' : 'SPL'}), ` +
      `split ${nodeBps / 100}% nodes / ${burnBps / 100}% burn, every ${intervalMs}ms`,
  );
  await cycle(decimals).catch((e) => console.error('[buyback] cycle error:', (e as Error).message));
  setInterval(() => {
    void cycle(decimals).catch((e) =>
      console.error('[buyback] cycle error:', (e as Error).message),
    );
  }, intervalMs).unref();
}

main().catch((e) => {
  console.error('[buyback] fatal:', e);
  process.exit(1);
});
