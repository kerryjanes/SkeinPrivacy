// CLI: `become a node` — register a running weft-node on-chain, or read its status.
//
//   pnpm node:register --manifest <node.json> --key <node.key>
//   pnpm node:status   --manifest <node.json>
//
// Manifest = the daemon's WEFT_MANIFEST output; key = its WEFT_OPERATOR_KEY file.

import { loadEnv } from './config';
import { nodeStatusFromManifest, registerFromManifest } from './nodeOperator';

function flag(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

async function main() {
  const cmd = process.argv[2];
  const manifest = flag('manifest');
  if (!manifest) throw new Error('missing --manifest <path to the daemon WEFT_MANIFEST json>');
  const env = loadEnv();

  if (cmd === 'register') {
    const key = flag('key');
    if (!key) throw new Error('missing --key <path to the daemon WEFT_OPERATOR_KEY file>');
    const r = await registerFromManifest(manifest, key, env);
    if (r.alreadyRegistered) {
      console.log(`[node-op] node ${r.nodeId} already registered (NodeState ${r.nodeState})`);
    } else {
      console.log(`[node-op] ✅ registered node ${r.nodeId} on ${env.cluster}`);
      console.log(`  operator:  ${r.operator}`);
      console.log(`  NodeState: ${r.nodeState}`);
      console.log(`  tx:        ${r.signature}`);
    }
  } else if (cmd === 'status') {
    const s = await nodeStatusFromManifest(manifest, env);
    console.log(`[node-op] node ${s.nodeId} on ${env.cluster}`);
    console.log(`  operator:  ${s.operator}`);
    console.log(`  NodeState: ${s.nodeState}`);
    if (!s.registered || !s.data) {
      console.log('  status:    NOT REGISTERED — run `pnpm node:register` to mint the node cNFT');
      return;
    }
    const d = s.data;
    console.log(`  status:    ${d.status === 0 ? 'active' : d.status}`);
    console.log(`  caps:      ${d.capabilities}  geo: ${d.geo}  availability: ${d.availability}%`);
    console.log(`  stake:     ${d.stakeAmount}  reputation: ${d.reputation} bps`);
    console.log(`  cNFT:      ${d.assetId}`);
    console.log(
      `  endpoint_hash: ${d.endpointHashHex} ${d.endpointHashMatchesManifest ? '(matches manifest ✓)' : '(MISMATCH ✗)'}`,
    );
  } else {
    throw new Error(`unknown command "${cmd}" — use "register" or "status"`);
  }
}

main().catch((e) => {
  console.error('[node-op] failed:', e instanceof Error ? e.message : e);
  process.exit(1);
});
