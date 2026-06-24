// One-shot: grow a pre-M8 ProtocolConfig (101 bytes) to the current M8 layout (121 bytes)
// by invoking the governance `migrate_protocol_config` instruction. Idempotent.
//
// Run: WEFT_KEYPAIR=… WEFT_RPC_URL=<helius> pnpm --filter @weft/governance exec tsx src/migrateConfig.ts
import { createHash } from 'node:crypto';
import { AccountRole, address, type Instruction } from '@solana/kit';
import { governance } from '@weft/sdk';
import { connect, loadSigner, send } from './kit';

const RPC = process.env.WEFT_RPC_URL ?? 'https://api.devnet.solana.com';
const WS = process.env.WEFT_RPC_WS ?? RPC.replace(/^http/, 'ws').replace('8899', '8900');
const KEYPAIR = process.env.WEFT_KEYPAIR ?? `${process.env.HOME}/.config/solana/id.json`;

const SYSTEM_PROGRAM = address('11111111111111111111111111111111');

/** Anchor instruction discriminator = sha256("global:<name>")[..8]. */
function discriminator(name: string): Uint8Array {
  return Uint8Array.from(createHash('sha256').update(`global:${name}`).digest().subarray(0, 8));
}

async function main() {
  const conn = connect(RPC, WS);
  const payer = await loadSigner(KEYPAIR);
  const [protocolConfig] = await governance.findProtocolConfigPda();

  const before = await conn.rpc.getAccountInfo(protocolConfig, { encoding: 'base64' }).send();
  const sizeOf = (v: typeof before.value) => (v ? Buffer.from(v.data[0], 'base64').length : 0);
  console.log(`[migrate] ProtocolConfig ${protocolConfig} — current size ${sizeOf(before.value)}`);

  const ix: Instruction = {
    programAddress: governance.GOVERNANCE_PROGRAM_ADDRESS,
    accounts: [
      { address: payer.address, role: AccountRole.WRITABLE_SIGNER },
      { address: protocolConfig, role: AccountRole.WRITABLE },
      { address: SYSTEM_PROGRAM, role: AccountRole.READONLY },
    ],
    data: discriminator('migrate_protocol_config'),
  };

  const sig = await send(conn, payer, [ix]);
  const after = await conn.rpc.getAccountInfo(protocolConfig, { encoding: 'base64' }).send();
  console.log(`[migrate] done · tx ${sig} · new size ${sizeOf(after.value)}`);

  // Sanity: the SDK decoder must now succeed.
  const cfg = await governance.fetchProtocolConfig(conn.rpc, protocolConfig);
  console.log(
    `[migrate] ✅ decodes: bootstrap_node_limit=${cfg.data.bootstrapNodeLimit}, bonus_bps=${cfg.data.bootstrapBonusBps}, end_ts=${cfg.data.bootstrapEndTs}`,
  );
}

main().catch((e) => {
  console.error('[migrate] failed:', e);
  process.exit(1);
});
