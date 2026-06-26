// Aggregator runner: load a batch of dual-signed receipts, fetch node info from
// NodeState, build the epoch reward tree, post the root on-chain, and serve
// proofs + Solana Pay. Receipts arrive as a JSON file (a real deployment would
// stream them from relays); everything downstream is the tested core.

import { existsSync, readFileSync } from 'node:fs';
import {
  createSolanaRpc,
  createSolanaRpcSubscriptions,
  createKeyPairSignerFromBytes,
  type Address,
} from '@solana/kit';
import { rewardsSettlement } from '@weft/sdk';

import { buildEpoch, type BuildOptions } from './rewards';
import { fetchNodeInfos } from './nodes';
import { postEpoch } from './poster';
import { EpochStore } from './store';
import { createAggregatorServer } from './server';
import type { TrafficReceipt } from './receipts';

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

  const rpc = createSolanaRpc(rpcUrl);
  const rpcSubscriptions = createSolanaRpcSubscriptions(wsUrl);

  const nodes = await fetchNodeInfos(rpc);
  const receipts = parseReceipts(receiptsPath);
  const receiptsByEpoch = new Map<string, TrafficReceipt[]>();
  receiptsByEpoch.set(epoch.toString(), receipts);
  const opts: BuildOptions = {};
  const build = buildEpoch(epoch, receipts, nodes, opts);
  console.log(
    `[aggregator] epoch ${epoch}: ${build.numNodes} nodes, ${build.totalReward} base units, root ${build.root || '(empty)'}`,
  );

  const store = new EpochStore();
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

  if (poster && build.numNodes > 0) {
    await maybePost(build);
  }

  const [distributor] = await rewardsSettlement.findDistributorPda();
  const distInfo = await rpc.getAccountInfo(distributor, { encoding: 'base64' }).send();
  if (!distInfo.value) throw new Error('distributor not initialized');
  const d = rewardsSettlement
    .getDistributorDecoder()
    .decode(Buffer.from(distInfo.value.data[0], 'base64'));

  const server = createAggregatorServer({
    store,
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
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
