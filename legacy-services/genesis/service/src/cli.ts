// Entry point: run genesis against the configured cluster.
//   pnpm -F @weft/genesis genesis            # localnet
//   WEFT_CLUSTER=devnet pnpm -F @weft/genesis genesis

import { address, type Address } from '@solana/kit';

import { loadEnv, SCHEDULES } from './config';
import { runGenesis, type OwnerOverrides } from './genesis';

const env = loadEnv();

// On devnet/mainnet, point custody at the Squads multisig vault addresses
// (vault 0 = treasury, 1 = emissions, 2 = IDO). Absent overrides, ephemeral
// owner addresses are generated and recorded in the manifest (localnet/dev).
const overrides: OwnerOverrides = {};
const envOwner = (k: string): Address | undefined =>
  process.env[k] ? address(process.env[k] as string) : undefined;
overrides.treasury = envOwner('WEFT_TREASURY_OWNER');
overrides.emissions = envOwner('WEFT_EMISSIONS_OWNER');
overrides.ido = envOwner('WEFT_IDO_OWNER');
overrides.scheduleOwners = {};
for (const schedule of SCHEDULES) {
  const prefix = `WEFT_${schedule.key.toUpperCase()}`;
  const beneficiary = envOwner(`${prefix}_BENEFICIARY`);
  const authority = envOwner(`${prefix}_AUTHORITY`);
  if (beneficiary || authority) {
    if (!beneficiary || !authority) {
      throw new Error(`${prefix}_BENEFICIARY and ${prefix}_AUTHORITY must be set together`);
    }
    overrides.scheduleOwners[schedule.key] = { beneficiary, authority };
  }
}

runGenesis(env, overrides)
  .then((m) => {
    console.log(`\n[genesis] complete on ${m.cluster}`);
    console.log(`  mint:              ${m.weftMint}`);
    console.log(`  authority retired: ${m.mintAuthorityRetired}`);
    console.log(`  manifest:          services/genesis/manifests/${m.cluster}.json`);
  })
  .catch((e) => {
    console.error('[genesis] failed:', e);
    process.exit(1);
  });
