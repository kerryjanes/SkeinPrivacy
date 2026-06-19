import { loadEnv } from './config';
import { provision } from './provision';

const env = loadEnv();
provision(env)
  .then((m) => {
    console.log(`\n[provision] complete on ${m.cluster}`);
    console.log(`  collection:  ${m.collection}`);
    console.log(`  merkleTree:  ${m.merkleTree}`);
    console.log(`  registry:    ${m.registry}`);
    console.log(`  manifest:    services/registry-provision/manifests/${m.cluster}.json`);
  })
  .catch((e) => {
    console.error('[provision] failed:', e);
    process.exit(1);
  });
