// One-time node-cNFT provisioning: create the MPL-Core collection (owned by the
// registry PDA) + a public Bubblegum V2 merkle tree, then point the on-chain
// registry at them (`set_registry_collection` + `register_tree`). Idempotent:
// re-running after a complete manifest is a no-op. The Weft core must already be
// initialized (`core:init`) so the registry PDA exists.

import { generateSigner, publicKey } from '@metaplex-foundation/umi';
import { createTreeV2 } from '@metaplex-foundation/mpl-bubblegum';
import { createCollection } from '@metaplex-foundation/mpl-core';
import { address, type Address } from '@solana/kit';
import { weft } from '@weft/sdk';

import { loadEnv, loadKeypairBytes, type Env } from './config';
import { connect, loadSigner, send } from './kit';
import { loadManifest, saveManifest, type Manifest } from './manifest';
import { TREE_MAX_BUFFER, TREE_MAX_DEPTH } from './nft';
import { createUmiClient } from './umi';

const COLLECTION_URI = 'https://www.weftnetwork.net/nft/collection.json';

export async function provision(env: Env): Promise<Manifest> {
  const existing = loadManifest(env.cluster);
  if (existing?.complete && existing.collection && existing.merkleTree) {
    console.log(
      `[provision] ${env.cluster} already provisioned (collection ${existing.collection}); skipping.`,
    );
    return existing;
  }

  const secret = loadKeypairBytes(env.keypairPath);
  const umi = createUmiClient(env.rpcUrl, secret);
  const conn = connect(env.rpcUrl, env.wsUrl);
  const deployer = await loadSigner(env.keypairPath);
  const [registry] = await weft.findRegistryPda();

  const registryInfo = await conn.rpc.getAccountInfo(registry, { encoding: 'base64' }).send();
  if (!registryInfo.value) {
    throw new Error(
      `[provision] Weft core is not initialized on ${env.cluster} (registry PDA ${registry} ` +
        `missing). Run \`core:init\` first.`,
    );
  }
  console.log(`[provision] deployer ${deployer.address}; registry PDA ${registry}`);

  // 1. MPL-Core collection; update authority = registry PDA so only the Weft
  //    program (signing as the PDA) can mint node cNFTs into it.
  const collection = generateSigner(umi);
  await createCollection(umi, {
    collection,
    name: 'Weft Nodes',
    uri: COLLECTION_URI,
    updateAuthority: publicKey(registry),
    plugins: [{ type: 'BubblegumV2' }],
  }).sendAndConfirm(umi);
  console.log(`[provision] collection ${collection.publicKey}`);

  // 2. Public Bubblegum V2 merkle tree (the collection authority is the mint gate).
  const merkleTree = generateSigner(umi);
  await (
    await createTreeV2(umi, {
      merkleTree,
      maxDepth: TREE_MAX_DEPTH,
      maxBufferSize: TREE_MAX_BUFFER,
      public: true,
    })
  ).sendAndConfirm(umi);
  console.log(`[provision] merkle tree ${merkleTree.publicKey}`);

  const collectionAddr = address(collection.publicKey) as Address;
  const treeAddr = address(merkleTree.publicKey) as Address;

  // 3. Point the registry at the collection.
  const setCollIx = await weft.getSetRegistryCollectionInstructionAsync({
    authority: deployer,
    collection: collectionAddr,
  });
  await send(conn, deployer, [setCollIx]);

  // 4. Register tree shard 0 → active tree.
  const [treeShard] = await weft.findTreeShardPda({ index: 0 });
  const rtIx = await weft.getRegisterTreeInstructionAsync({
    authority: deployer,
    merkleTree: treeAddr,
    index: 0,
    maxDepth: TREE_MAX_DEPTH,
  });
  await send(conn, deployer, [rtIx]);
  console.log(`[provision] registry collection set; tree shard 0 registered (${treeShard})`);

  const manifest: Manifest = {
    cluster: env.cluster,
    complete: true,
    registryProgram: weft.WEFT_PROGRAM_ADDRESS,
    collection: collectionAddr,
    merkleTree: treeAddr,
    treeShard,
    maxDepth: TREE_MAX_DEPTH,
  };
  saveManifest(manifest);
  return manifest;
}

// CLI entry: `pnpm nft:provision`
if (import.meta.url === `file://${process.argv[1]}`) {
  provision(loadEnv())
    .then((m) => {
      console.log(`[provision] done: ${JSON.stringify(m, null, 2)}`);
      process.exit(0);
    })
    .catch((e) => {
      console.error(`[provision] failed:`, e);
      process.exit(1);
    });
}
