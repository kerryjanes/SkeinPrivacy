import { generateSigner, publicKey } from '@metaplex-foundation/umi';
import { createTreeV2 } from '@metaplex-foundation/mpl-bubblegum';
import { createCollection } from '@metaplex-foundation/mpl-core';
import { address } from '@solana/kit';
import { nodeRegistry } from '@weft/sdk';

import {
  loadKeypairBytes,
  registryPda,
  treeShardPda,
  TREE_MAX_BUFFER,
  TREE_MAX_DEPTH,
  type Env,
} from './config';
import { connect, loadSigner, send } from './kit';
import { loadManifest, saveManifest, type Manifest } from './manifest';
import { createUmiClient } from './umi';

export async function provision(env: Env): Promise<Manifest> {
  const existing = loadManifest(env.cluster);
  if (existing?.complete) {
    console.log(`[provision] ${env.cluster} already complete (collection ${existing.collection}); skipping.`);
    return existing;
  }

  const secret = loadKeypairBytes(env.keypairPath);
  const umi = createUmiClient(env.rpcUrl, secret);
  const conn = connect(env.rpcUrl, env.wsUrl);
  const deployer = await loadSigner(env.keypairPath);
  const registry = await registryPda();
  console.log(`[provision] deployer ${deployer.address}; registry PDA ${registry}`);

  // 1. MPL-Core collection with BubblegumV2 plugin; update authority = registry PDA
  //    so only the node-registry program (signing as the PDA) can mint into it.
  const collection = generateSigner(umi);
  await createCollection(umi, {
    collection,
    name: 'Weft Nodes',
    uri: 'https://weft.network/collection.json',
    updateAuthority: publicKey(registry),
    plugins: [{ type: 'BubblegumV2' }],
  }).sendAndConfirm(umi);
  console.log(`[provision] collection ${collection.publicKey}`);

  // 2. Public V2 merkle tree (the collection authority is the real mint gate).
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

  // 3. Initialize the on-chain registry (kit).
  const collectionAddr = address(collection.publicKey);
  const treeAddr = address(merkleTree.publicKey);
  const initIx = await nodeRegistry.getInitializeRegistryInstructionAsync({
    authority: deployer,
    collection: collectionAddr,
    activeTree: treeAddr,
  });
  await send(conn, deployer, [initIx]);

  // 4. Register tree shard 0 (kit).
  const treeShard = await treeShardPda(0);
  const rtIx = nodeRegistry.getRegisterTreeInstruction({
    authority: deployer,
    registry,
    merkleTree: treeAddr,
    treeShard,
    index: 0,
    maxDepth: TREE_MAX_DEPTH,
  });
  await send(conn, deployer, [rtIx]);
  console.log('[provision] registry initialized; tree shard 0 registered');

  const manifest: Manifest = {
    cluster: env.cluster,
    complete: true,
    registryProgram: nodeRegistry.NODE_REGISTRY_PROGRAM_ADDRESS,
    registry,
    collection: collectionAddr,
    merkleTree: treeAddr,
    treeShard,
    maxDepth: TREE_MAX_DEPTH,
  };
  saveManifest(manifest);
  return manifest;
}
