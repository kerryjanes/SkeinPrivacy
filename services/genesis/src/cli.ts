// Entry point: run genesis against the configured cluster.
//   pnpm -F @weft/genesis genesis            # localnet
//   WEFT_CLUSTER=devnet pnpm -F @weft/genesis genesis

import { address, type Address } from '@solana/kit';

import { loadEnv } from './config';
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
