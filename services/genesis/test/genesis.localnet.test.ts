// End-to-end genesis test against a local validator with weft-vesting deployed.
//
// Run a local validator first and set WEFT_RUN_LOCALNET_TESTS=1:
//   solana-test-validator --reset \
//     --bpf-program FCFZNb2Kqh7ScjikKp73W7BcsfusrZ1hTBhc61Macdsv target/deploy/weft_vesting.so
//   solana airdrop 100 <deployer> --url localhost

import { rmSync } from 'node:fs';
import { beforeAll, describe, expect, it } from 'vitest';

import { AMOUNTS, loadEnv, TOTAL_SUPPLY } from '../src/config';
import { runGenesis } from '../src/genesis';
import { manifestPath } from '../src/manifest';
import { connect, type Connection } from '../src/rpc';
import type { Manifest } from '../src/manifest';

const env = loadEnv({ cluster: 'localnet', rpcUrl: 'http://127.0.0.1:8899' });

async function balance(conn: Connection, ata: string): Promise<bigint> {
  const { value } = await conn.rpc.getTokenAccountBalance(ata as never).send();
  return BigInt(value.amount);
}

describe.skipIf(process.env.WEFT_RUN_LOCALNET_TESTS !== '1')('genesis on localnet', () => {
  let manifest: Manifest;
  let conn: Connection;

  beforeAll(async () => {
    rmSync(manifestPath('localnet'), { force: true });
    manifest = await runGenesis(env);
    conn = connect(env.rpcUrl, env.wsUrl);
  }, 120_000);

  it('mints exactly the fixed total supply', async () => {
    const { value } = await conn.rpc.getTokenSupply(manifest.weftMint as never).send();
    expect(BigInt(value.amount)).toBe(TOTAL_SUPPLY);
  });

  it('retires mint + freeze authorities and keeps 9 decimals', async () => {
    const { value } = await conn.rpc
      .getAccountInfo(manifest.weftMint as never, { encoding: 'jsonParsed' })
      .send();
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const info = (value as any).data.parsed.info;
    expect(info.mintAuthority).toBeNull();
    expect(info.freezeAuthority).toBeNull();
    expect(info.decimals).toBe(9);
  });

  it('routes the three liquid custody buckets exactly', async () => {
    expect(await balance(conn, manifest.custody.treasury.ata)).toBe(AMOUNTS.treasury);
    expect(await balance(conn, manifest.custody.emissions.ata)).toBe(AMOUNTS.nodeEmissions);
    // The full IDO bucket (150M) is custody now — the distributor splits 25/75 on claim.
    expect(await balance(conn, manifest.custody.ido.ata)).toBe(AMOUNTS.idoTge + AMOUNTS.idoLinear);
  });

  it('funds each vesting vault exactly', async () => {
    expect(await balance(conn, manifest.schedules.team.vault)).toBe(AMOUNTS.team);
    expect(await balance(conn, manifest.schedules.ecosystem.vault)).toBe(AMOUNTS.ecosystem);
    expect(await balance(conn, manifest.schedules.marketing.vault)).toBe(AMOUNTS.marketing);
  });

  it('conserves the total supply across all destinations', async () => {
    let sum = 0n;
    for (const k of ['treasury', 'emissions', 'ido'] as const) {
      sum += await balance(conn, manifest.custody[k].ata);
    }
    for (const k of Object.keys(manifest.schedules)) {
      sum += await balance(conn, manifest.schedules[k].vault);
    }
    expect(sum).toBe(TOTAL_SUPPLY);
  });

  it('is idempotent: re-running returns the same mint', async () => {
    const again = await runGenesis(env);
    expect(again.weftMint).toBe(manifest.weftMint);
  });
});
