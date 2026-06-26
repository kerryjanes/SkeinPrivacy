// On-chain reads for the control plane: a wallet's prepaid escrow balance (→ its traffic quota) and
// verification of a user's settlement tx so we can clear their metered tab.

import { address, createSolanaRpc, type Address } from '@solana/kit';
import { math, rewardsSettlement } from '@weft/sdk';

export type Rpc = ReturnType<typeof createSolanaRpc>;

export function rpc(url: string): Rpc {
  return createSolanaRpc(url);
}

/** Bytes a given $WEFT balance (base units, 9 decimals) buys at the 0.1 WEFT/GB base rate. */
export function quotaBytes(balanceBaseUnits: bigint): bigint {
  // inverse of `base = BASE_RATE_PER_GB * bytes / BYTES_PER_GB` (sdk/math.ts, mirrors on-chain)
  return (balanceBaseUnits * math.BYTES_PER_GB) / math.BASE_RATE_PER_GB;
}

/** $WEFT base units a given number of bytes costs (what a node is owed for serving them). */
export function costBaseUnits(bytes: bigint): bigint {
  return (math.BASE_RATE_PER_GB * bytes) / math.BYTES_PER_GB;
}

/** A wallet's prepaid escrow balance in base units. Returns 0 if the escrow doesn't exist yet. */
export async function escrowBalance(r: Rpc, owner: string, mint: string): Promise<bigint> {
  const [escrow] = await rewardsSettlement.findEscrowPda({ owner: address(owner) });
  try {
    const acct = await rewardsSettlement.fetchPaymentEscrow(r, escrow);
    if (acct.data.owner !== owner || acct.data.mint !== mint) return 0n;
    return acct.data.balance;
  } catch {
    return 0n; // no escrow account yet → zero prepaid balance
  }
}

export interface VerifiedPayment {
  payer: Address;
  amount: bigint; // $WEFT base units paid
}

/**
 * Verify a transaction is a finalized settlement signed by `expectedPayer`, and return the paid
 * amount. We decode the settlement program's instruction from the tx (discriminator + u64 amount)
 * rather than trusting the client — the payment must really have landed on chain.
 */
export async function verifyPayTraffic(
  r: Rpc,
  signature: string,
  expectedPayer: string,
): Promise<VerifiedPayment> {
  const tx = await r
    .getTransaction(signature as Parameters<Rpc['getTransaction']>[0], {
      commitment: 'finalized',
      maxSupportedTransactionVersion: 0,
      encoding: 'json',
    })
    .send();
  if (!tx) throw new Error('transaction not found / not finalized');
  if (tx.meta?.err) throw new Error('transaction failed on chain');

  const msg = tx.transaction.message;
  return decodePayTraffic(
    msg.accountKeys.map((k) => String(k)),
    msg.instructions.map((ix) => ({
      programIdIndex: ix.programIdIndex,
      accounts: [...ix.accounts],
      data: ix.data,
    })),
    expectedPayer,
  );
}

export interface RawInstruction {
  programIdIndex: number;
  accounts: number[];
  data: string; // base58 (json encoding)
}

/**
 * Pure decode: find a settlement instruction to the settlement program, confirm the wallet signer
 * (accounts[0]) is `expectedPayer`, and read the u64 amount after the 8-byte discriminator.
 * Accepts both legacy direct `pay_traffic` and escrow-first `pay_traffic_from_escrow`.
 * Extracted from the RPC fetch so it's deterministically testable.
 */
export function decodePayTraffic(
  accountKeys: string[],
  instructions: RawInstruction[],
  expectedPayer: string,
): VerifiedPayment {
  const programId = String(rewardsSettlement.REWARDS_SETTLEMENT_PROGRAM_ADDRESS);
  const discriminators = [
    rewardsSettlement.PAY_TRAFFIC_DISCRIMINATOR,
    rewardsSettlement.PAY_TRAFFIC_FROM_ESCROW_DISCRIMINATOR,
  ];

  for (const ix of instructions) {
    if (accountKeys[ix.programIdIndex] !== programId) continue;
    const data = bs58Decode(ix.data);
    if (data.length < 16) continue;
    if (!discriminators.some((disc) => data.slice(0, 8).every((b, i) => b === disc[i]))) continue;
    // accounts[0] is the payer/owner signer in both PayTraffic and PayTrafficFromEscrow layouts.
    const payer = accountKeys[ix.accounts[0]];
    if (payer !== expectedPayer) throw new Error('payment not signed by this wallet');
    const amount = readU64LE(data, 8);
    return { payer: address(payer), amount };
  }
  throw new Error('no settlement instruction for this wallet in the transaction');
}

function readU64LE(b: Uint8Array, off: number): bigint {
  let v = 0n;
  for (let i = 0; i < 8; i++) v |= BigInt(b[off + i]) << (8n * BigInt(i));
  return v;
}

// Minimal base58 decode (tx instruction data in json encoding is base58).
const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function bs58Decode(s: string): Uint8Array {
  let n = 0n;
  for (const ch of s) {
    const i = B58.indexOf(ch);
    if (i < 0) throw new Error('bad base58');
    n = n * 58n + BigInt(i);
  }
  const bytes: number[] = [];
  while (n > 0n) {
    bytes.unshift(Number(n & 0xffn));
    n >>= 8n;
  }
  for (let i = 0; i < s.length && s[i] === '1'; i++) bytes.unshift(0);
  return Uint8Array.from(bytes);
}
