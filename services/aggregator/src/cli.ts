// Aggregator runner: load a batch of dual-signed receipts, fetch node info from
// NodeState, build the epoch reward tree, post the root on-chain, and serve
// proofs + Solana Pay. Receipts arrive as a JSON file (a real deployment would
// stream them from relays); everything downstream is the tested core.

import { existsSync, readFileSync } from 'node:fs';
import {
  createSolanaRpc,
  createSolanaRpcSubscriptions,
  createKeyPairSignerFromBytes,
  address,
  type Address,
} from '@solana/kit';
import { weft } from '@weft/sdk';

import { buildEpoch, buildEpochFromByteTotals, type BuildOptions, type ByteTotal } from './rewards';
import { fetchNodeInfos } from './nodes';
import { postEpoch } from './poster';
import { EpochStore } from './store';
import { createAggregatorServer } from './server';
import type { TrafficReceipt } from './receipts';
import {
  buildProfileByteTotals,
  readRelayProfiles,
  readSettledProfileBytes,
  writeSettledProfileBytes,
} from './profileTotals';

interface ReceiptJson {
  client: string;
  operator: string;
  nodeId: string;
  bytes: string;
  windowStart: string;
  windowEnd: string;
  nonce: string;
  clientSig: string;
  relaySig: string;
}

function parseReceipts(path: string): TrafficReceipt[] {
  if (!existsSync(path)) return [];
  const raw = JSON.parse(readFileSync(path, 'utf8')) as ReceiptJson[];
  return raw.map((r) => ({
    client: r.client as Address,
    operator: r.operator as Address,
    nodeId: BigInt(r.nodeId),
    bytes: BigInt(r.bytes),
    windowStart: BigInt(r.windowStart),
    windowEnd: BigInt(r.windowEnd),
    nonce: BigInt(r.nonce),
    clientSig: r.clientSig,
    relaySig: r.relaySig,
  }));
}

function parseTrustedTotals(): ByteTotal[] {
  const raw = process.env.WEFT_TRUSTED_TOTALS;
  if (raw) {
    const totals = JSON.parse(raw) as Array<{ operator: string; nodeId: string; bytes: string }>;
    return totals.map((t) => ({
      operator: address(t.operator) as Address,
      nodeId: BigInt(t.nodeId),
      bytes: BigInt(t.bytes),
    }));
  }
  const operator = process.env.WEFT_TRUSTED_OPERATOR;
  const nodeId = process.env.WEFT_TRUSTED_NODE_ID;
  const bytes = process.env.WEFT_TRUSTED_BYTES;
  if (!operator && !nodeId && !bytes) return [];
  if (!operator || !nodeId || !bytes) {
    throw new Error(
      'WEFT_TRUSTED_OPERATOR, WEFT_TRUSTED_NODE_ID, and WEFT_TRUSTED_BYTES are required together',
    );
  }
  return [{ operator: address(operator) as Address, nodeId: BigInt(nodeId), bytes: BigInt(bytes) }];
}

function trustedTotalsConfigured(): boolean {
  return Boolean(
    process.env.WEFT_TRUSTED_TOTALS ||
    process.env.WEFT_TRUSTED_OPERATOR ||
    process.env.WEFT_TRUSTED_NODE_ID ||
    process.env.WEFT_TRUSTED_BYTES,
  );
}

const CLASSIC_TOKEN_PROGRAM = address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const TOKEN_2022_PROGRAM = address('TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb');

/** Read the reward mint's decimals + owning token program from chain
 *  (distributor → reward_mint → mint account). pump.fun now mints Token-2022;
 *  every ATA + settlement instruction must use the mint's real owner program.
 *  Falls back to classic-SPL 6-decimal $WEFT if the core isn't initialized. */
async function fetchRewardMintInfo(
  rpc: ReturnType<typeof createSolanaRpc>,
): Promise<{ decimals: number; tokenProgram: Address }> {
  const [distributor] = await weft.findDistributorPda();
  const di = await rpc.getAccountInfo(distributor, { encoding: 'base64' }).send();
  if (!di.value) return { decimals: 6, tokenProgram: CLASSIC_TOKEN_PROGRAM };
  const mint = weft
    .getDistributorDecoder()
    .decode(Buffer.from(di.value.data[0], 'base64')).rewardMint;
  const mi = await rpc.getAccountInfo(mint, { encoding: 'base64' }).send();
  if (!mi.value) return { decimals: 6, tokenProgram: CLASSIC_TOKEN_PROGRAM };
  const tokenProgram = mi.value.owner === TOKEN_2022_PROGRAM ? TOKEN_2022_PROGRAM : CLASSIC_TOKEN_PROGRAM;
  return { decimals: Buffer.from(mi.value.data[0], 'base64')[44], tokenProgram }; // decimals at byte 44
}

async function main(): Promise<void> {
  const cluster = process.env.WEFT_CLUSTER ?? 'devnet';
  const mainnet = cluster.startsWith('mainnet');
  if (mainnet && !process.env.WEFT_RPC) {
    throw new Error(`WEFT_RPC must be set explicitly for ${cluster}`);
  }
  const rpcUrl = process.env.WEFT_RPC ?? 'https://api.devnet.solana.com';
  const wsUrl = process.env.WEFT_RPC_WS ?? rpcUrl.replace(/^http/, 'ws');
  const epoch = BigInt(process.env.WEFT_EPOCH ?? '0');
  const receiptsPath = process.env.WEFT_RECEIPTS ?? 'receipts.json';
  const posterPath = process.env.WEFT_POSTER;
  const port = Number(process.env.PORT ?? '8788');
  const postOnReceipts = process.env.WEFT_POST_ON_RECEIPTS === '1';
  const exitAfterPost = process.env.WEFT_EXIT_AFTER_POST === '1';
  const minStakeToEarn = process.env.WEFT_MIN_STAKE_TO_EARN
    ? BigInt(process.env.WEFT_MIN_STAKE_TO_EARN)
    : undefined;
  const maxBytesPerEpoch = process.env.WEFT_MAX_BYTES_PER_EPOCH
    ? BigInt(process.env.WEFT_MAX_BYTES_PER_EPOCH)
    : undefined;
  const autoSettle = process.env.WEFT_AUTO_SETTLE === '1';
  const autoSettleMs = Number(process.env.WEFT_AUTO_SETTLE_MS ?? '600000');
  const relayProfilePath =
    process.env.WEFT_RELAY_PROFILE_PATH ?? '/var/lib/weft/exit-profiles.json';
  const settledProfilePath =
    process.env.WEFT_SETTLED_PROFILE_BYTES ?? '/var/lib/weft/settled-profile-bytes.json';
  const epochStorePath = process.env.WEFT_EPOCH_STORE ?? '/var/lib/weft/reward-epochs.json';
  const receiptsToken = process.env.WEFT_RECEIPTS_TOKEN ?? '';
  if (mainnet && trustedTotalsConfigured()) {
    throw new Error('WEFT_TRUSTED_* totals are devnet-only and must be unset on mainnet');
  }
  if (mainnet && !receiptsToken) {
    throw new Error('WEFT_RECEIPTS_TOKEN must be set explicitly for mainnet (relay → aggregator auth)');
  }

  const rpc = createSolanaRpc(rpcUrl);
  const rpcSubscriptions = createSolanaRpcSubscriptions(wsUrl);
  const store = new EpochStore(epochStorePath);

  const nodes = await fetchNodeInfos(rpc);
  const receipts = parseReceipts(receiptsPath);
  const trustedTotals = parseTrustedTotals();
  const receiptsByEpoch = new Map<string, TrafficReceipt[]>();
  receiptsByEpoch.set(epoch.toString(), receipts);
  // Read the reward mint's decimals from chain so reward math adapts to the token
  // (6 on mainnet's pump.fun $WEFT, 9 on a devnet test mint) — same code either way.
  const { decimals: rewardDecimals, tokenProgram } = await fetchRewardMintInfo(rpc);
  const opts: BuildOptions = { minStakeToEarn, maxBytesPerEpoch, decimals: rewardDecimals };
  const build =
    trustedTotals.length > 0
      ? buildEpochFromByteTotals(epoch, trustedTotals, nodes, opts)
      : buildEpoch(epoch, receipts, nodes, opts);
  console.log(
    `[aggregator] epoch ${epoch}: ${build.numNodes} nodes, ${build.totalReward} base units, root ${build.root || '(empty)'}`,
  );

  if ((receipts.length > 0 || trustedTotals.length > 0) && build.numNodes > 0) {
    store.put(build);
  }

  const poster = posterPath
    ? await createKeyPairSignerFromBytes(
        Uint8Array.from(JSON.parse(readFileSync(posterPath, 'utf8')) as number[]),
      )
    : null;

  async function maybePost(b: typeof build): Promise<string | null> {
    if (!poster || b.numNodes === 0) return null;
    const sig = await postEpoch({ rpc, rpcSubscriptions, poster }, b);
    console.log(`[aggregator] posted epoch ${b.epoch}: ${sig}`);
    return sig;
  }

  if (!autoSettle && poster && build.numNodes > 0) {
    await maybePost(build);
  }
  if (exitAfterPost) return;

  const [distributor] = await weft.findDistributorPda();
  const distInfo = await rpc.getAccountInfo(distributor, { encoding: 'base64' }).send();
  if (!distInfo.value) throw new Error('distributor not initialized');
  const d = weft.getDistributorDecoder().decode(Buffer.from(distInfo.value.data[0], 'base64'));

  // The next epoch to settle is always derived from on-chain truth (distributor.current_epoch + 1),
  // never from a local counter or the persisted store — so a corrupt/poisoned store can't push the
  // epoch forward, and a failed post retries the same epoch instead of leaving a permanent gap.
  async function fetchCurrentEpoch(): Promise<bigint> {
    const info = await rpc.getAccountInfo(distributor, { encoding: 'base64' }).send();
    if (!info.value) throw new Error('distributor not initialized');
    return BigInt(
      weft.getDistributorDecoder().decode(Buffer.from(info.value.data[0], 'base64')).currentEpoch,
    );
  }

  const server = createAggregatorServer({
    store,
    receiptsToken: receiptsToken || undefined,
    payConfig: {
      rewardMint: d.rewardMint,
      rewardVault: d.rewardVault,
      treasury: d.treasury,
      tokenProgram,
      label: 'Weft VPN traffic',
    },
    getBlockhash: async () => (await rpc.getLatestBlockhash().send()).value,
    onReceipts: async (receivedEpoch, accepted) => {
      const key = receivedEpoch.toString();
      const all = [...(receiptsByEpoch.get(key) ?? []), ...accepted];
      receiptsByEpoch.set(key, all);
      const latestNodes = await fetchNodeInfos(rpc);
      const next = buildEpoch(receivedEpoch, all, latestNodes, opts);
      store.put(next);
      const postedSignature = postOnReceipts ? await maybePost(next) : null;
      return {
        root: next.root,
        totalReward: next.totalReward.toString(),
        numNodes: next.numNodes,
        postedSignature,
      };
    },
  });
  server.listen(port);
  console.log(`[aggregator] serving proofs + Solana Pay on :${port}`);

  let autoSettleRunning = false;
  async function autoSettleOnce(): Promise<void> {
    if (!poster) return;
    if (autoSettleRunning) return; // no overlapping runs — a slow post must not double-count bytes
    autoSettleRunning = true;
    try {
      const profiles = readRelayProfiles(relayProfilePath);
      const latestNodes = await fetchNodeInfos(rpc);
      const settled = readSettledProfileBytes(settledProfilePath);
      const { totals, nextSettled } = buildProfileByteTotals(profiles, latestNodes, settled);
      if (totals.length === 0) return;
      // Cap this epoch at what the reward vault can actually pay (balance − rewards already owed
      // but unclaimed), so post_epoch's solvency guard always passes and settlement never stalls.
      const distInfo = await rpc.getAccountInfo(distributor, { encoding: 'base64' }).send();
      if (!distInfo.value) throw new Error('distributor not initialized');
      const dist = weft.getDistributorDecoder().decode(Buffer.from(distInfo.value.data[0], 'base64'));
      const outstanding = BigInt(dist.cumulativeObligated) - BigInt(dist.cumulativeClaimed);
      const vaultBal = BigInt((await rpc.getTokenAccountBalance(d.rewardVault).send()).value.amount);
      const vaultCap = vaultBal > outstanding ? vaultBal - outstanding : 0n;
      const epochToPost = BigInt(dist.currentEpoch) + 1n;
      const next = buildEpochFromByteTotals(epochToPost, totals, latestNodes, { ...opts, vaultCap });
      if (next.numNodes === 0) {
        // No rewardable node for these bytes → no reward is owed, so advancing the cursor is safe.
        writeSettledProfileBytes(settledProfilePath, nextSettled);
        return;
      }
      // Persist proofs, then post the root, then advance the byte cursor — in that order. If the
      // post fails or the process dies before it lands, the cursor is untouched, so the next run
      // re-derives the same epoch (current+1) from the same cursor and replays identically: no
      // bytes are ever forfeited and no epoch gap opens. The only irreducible window — a crash
      // between on-chain confirmation and the cursor write — is bounded by post_epoch's vault
      // solvency guard and the per-epoch ClaimStatus, which cap any double-count and forbid
      // double-claims.
      store.put(next);
      const sig = await postEpoch({ rpc, rpcSubscriptions, poster }, next);
      writeSettledProfileBytes(settledProfilePath, nextSettled);
      console.log(
        `[aggregator] auto-settled epoch ${next.epoch}: ${next.numNodes} nodes, ${next.totalReward} base units, tx ${sig}`,
      );
    } catch (e) {
      console.error(
        `[aggregator] auto settlement failed; cursor not advanced, will retry: ${(e as Error).message} :: ${((e as { context?: { logs?: string[] } })?.context?.logs ?? (e as { logs?: string[] })?.logs ?? []).join(' | ') || (e as { cause?: { message?: string } })?.cause?.message || JSON.stringify((e as { context?: unknown })?.context ?? {}).slice(0, 800)}`,
      );
    } finally {
      autoSettleRunning = false;
    }
  }

  if (autoSettle) {
    console.log(`[aggregator] auto settlement enabled every ${autoSettleMs}ms`);
    setTimeout(() => {
      void autoSettleOnce().catch((e) =>
        console.error('[aggregator] auto settlement error:', e.message),
      );
    }, 5000).unref();
    setInterval(() => {
      void autoSettleOnce().catch((e) =>
        console.error('[aggregator] auto settlement error:', e.message),
      );
    }, autoSettleMs).unref();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
