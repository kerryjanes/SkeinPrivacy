#!/usr/bin/env -S tsx
// CLI: become a node under your wallet.
//   WEFT_NODE_ENDPOINT=relay.host:20001 WEFT_KEYPAIR=~/.config/solana/id.json tsx becomeNodeCli.ts
// or: tsx becomeNodeCli.ts relay.host:20001

import { nodeRegistry } from '@weft/sdk';
import { becomeNode } from './becomeNode';
import { loadEnv } from './config';
import { connect, loadSigner } from './kit';

const env = loadEnv();
const endpoint = process.env.WEFT_NODE_ENDPOINT ?? process.argv[2];
if (!endpoint) {
  console.error('usage: becomeNodeCli <relayHost:port>   (or set WEFT_NODE_ENDPOINT)');
  process.exit(1);
}

const conn = connect(env.rpcUrl, env.wsUrl);
const operator = await loadSigner(env.keypairPath);
console.log(`registering node under operator ${operator.address} (${env.cluster})…`);

const reg = await becomeNode(
  conn,
  operator,
  { endpoint, geo: Number(process.env.WEFT_GEO ?? 0) },
  env.cluster,
);
console.log(
  JSON.stringify({ operator: operator.address, ...reg, nodeId: reg.nodeId.toString() }, null, 2),
);

// Read the node back from chain to confirm it landed under this wallet.
const info = await conn.rpc.getAccountInfo(reg.node, { encoding: 'base64' }).send();
if (info.value) {
  const decoder = nodeRegistry.getNodeStateDecoder();
  let bytes = Buffer.from((info.value.data as readonly [string, string])[0], 'base64');
  // Devnet's registry program predates the M8 `sequence` field; pad it (reads as 0) so the
  // current SDK decoder applies cleanly to pre-M8 accounts.
  const need = (decoder as unknown as { fixedSize: number }).fixedSize;
  if (bytes.length < need) bytes = Buffer.concat([bytes, Buffer.alloc(need - bytes.length)]);
  const d = decoder.decode(bytes);
  console.log('✅ on-chain NodeState:', {
    operator: String(d.operator),
    nodeId: d.nodeId.toString(),
    geo: d.geo,
    capabilities: d.capabilities,
    availability: d.availability,
  });
} else {
  console.log('NodeState not yet readable (may need a moment to index)');
}
