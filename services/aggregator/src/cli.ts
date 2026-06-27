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
import { rewardsSettlement } from '@weft/sdk';

import { buildEpoch, buildEpochFromByteTotals, type BuildOptions, type ByteTotal } from './rewards';
import { fetchNodeInfos } from './nodes';
import { postEpoch } from './poster';
import { EpochStore, PayoutStore } from './store';
import { createAggregatorServer } from './server';
import { TokenPayout } from './payout';
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
    throw new Error('WEFT_TRUSTED_OPERATOR, WEFT_TRUSTED_NODE_ID, and WEFT_TRUSTED_BYTES are required together');
  }
  return [{ operator: address(operator) as Address, nodeId: BigInt(nodeId), bytes: BigInt(bytes) }];
}

async function main(): Promise<void> {
  const cluster = process.env.WEFT_CLUSTER ?? 'devnet';
  if (cluster.startsWith('mainnet') && !process.env.WEFT_RPC) {
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
  const relayProfilePath = process.env.WEFT_RELAY_PROFILE_PATH ?? '/var/lib/weft/exit-profiles.json';
  const settledProfilePath =
    process.env.WEFT_SETTLED_PROFILE_BYTES ?? '/var/lib/weft/settled-profile-bytes.json';
  const epochStorePath = process.env.WEFT_EPOCH_STORE ?? '/var/lib/weft/reward-epochs.json';
  const payoutStorePath = process.env.WEFT_PAYOUT_STORE ?? '/var/lib/weft/payouts.json';
  const payoutKeypairPath = process.env.WEFT_PAYOUT_KEYPAIR;

  const rpc = createSolanaRpc(rpcUrl);
  const rpcSubscriptions = createSolanaRpcSubscriptions(wsUrl);

  const nodes = await fetchNodeInfos(rpc);
  const receipts = parseReceipts(receiptsPath);
  const trustedTotals = parseTrustedTotals();
  const receiptsByEpoch = new Map<string, TrafficReceipt[]>();
  receiptsByEpoch.set(epoch.toString(), receipts);
  const opts: BuildOptions = { minStakeToEarn, maxBytesPerEpoch };
  const build =
    trustedTotals.length > 0
      ? buildEpochFromByteTotals(epoch, trustedTotals, nodes, opts)
      : buildEpoch(epoch, receipts, nodes, opts);
  console.log(
    `[aggregator] epoch ${epoch}: ${build.numNodes} nodes, ${build.totalReward} base units, root ${build.root || '(empty)'}`,
  );

  const store = new EpochStore(epochStorePath);
  const payoutStore = new PayoutStore(payoutStorePath);
  store.put(build);

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

  const [distributor] = await rewardsSettlement.findDistributorPda();
  const distInfo = await rpc.getAccountInfo(distributor, { encoding: 'base64' }).send();
  if (!distInfo.value) throw new Error('distributor not initialized');
  const d = rewardsSettlement
    .getDistributorDecoder()
    .decode(Buffer.from(distInfo.value.data[0], 'base64'));
  const payout = payoutKeypairPath
    ? new TokenPayout(rpcUrl, wsUrl, payoutKeypairPath, d.rewardMint)
    : undefined;
  let nextAutoEpoch = BigInt(d.currentEpoch) + 1n;

  const server = createAggregatorServer({
    store,
    payoutStore,
    payout,
    payConfig: {
      rewardMint: d.rewardMint,
      rewardVault: d.rewardVault,
      treasury: d.treasury,
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

  async function autoSettleOnce(): Promise<void> {
    if (!poster) return;
    const profiles = readRelayProfiles(relayProfilePath);
    const latestNodes = await fetchNodeInfos(rpc);
    const settled = readSettledProfileBytes(settledProfilePath);
    const { totals, nextSettled } = buildProfileByteTotals(profiles, latestNodes, settled);
    if (totals.length === 0) return;
    const next = buildEpochFromByteTotals(nextAutoEpoch, totals, latestNodes, opts);
    if (next.numNodes === 0) {
      writeSettledProfileBytes(settledProfilePath, nextSettled);
      return;
    }
    store.put(next);
    writeSettledProfileBytes(settledProfilePath, nextSettled);
    let postedSignature: string | null = null;
    try {
      postedSignature = await maybePost(next);
    } catch (e) {
      console.error(
        `[aggregator] stored off-chain epoch ${next.epoch}; on-chain post failed: ${(e as Error).message}`,
      );
    }
    console.log(
      `[aggregator] auto-settled epoch ${next.epoch}: ${next.numNodes} nodes, ${next.totalReward} base units, tx ${postedSignature}`,
    );
    nextAutoEpoch += 1n;
  }

  if (autoSettle) {
    console.log(`[aggregator] auto settlement enabled every ${autoSettleMs}ms`);
    setTimeout(() => {
      void autoSettleOnce().catch((e) => console.error('[aggregator] auto settlement error:', e.message));
    }, 5000).unref();
    setInterval(() => {
      void autoSettleOnce().catch((e) => console.error('[aggregator] auto settlement error:', e.message));
    }, autoSettleMs).unref();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
