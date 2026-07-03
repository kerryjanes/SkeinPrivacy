// Bubblegum V2 mint accounts for register_node. The on-chain program CPIs into
// Bubblegum V2 (mintV2); the kit-generated `getRegisterNodeInstructionAsync`
// auto-resolves registry/node/mplCore/bubblegum/system, so the caller supplies
// the tree + collection + fixed Bubblegum V2 helper programs here. Addresses and
// derivations mirror @metaplex-foundation/mpl-bubblegum v5's mintV2 resolver.

import { getAddressEncoder, getProgramDerivedAddress, type Address } from '@solana/kit';
import { weft } from '@weft/sdk';

/** Bubblegum V2 program. */
export const BUBBLEGUM_PROGRAM = 'BGUMAp9Gq7iTEuizy4pqaxsTyUCBK68MDfK752saRPUY' as Address;
/** MPL-Core program (collection lives here). */
export const MPL_CORE_PROGRAM = 'CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d' as Address;
/** mpl-account-compression program. */
export const MPL_ACCOUNT_COMPRESSION = 'mcmt6YrQEMKw8Mw43FmpRLmf7BqRnFMKmAcbxE3xkAW' as Address;
/** mpl-noop (log wrapper) program. */
export const MPL_NOOP = 'mnoopTCrg4p8ry25e4bcWA9XZjbNjMTfgYVGGEdRsf3' as Address;
/** Bubblegum's fixed mpl-core CPI signer (used when minting into a Core collection). */
export const MPL_CORE_CPI_SIGNER = 'CbNY3JiXdXNE9tPNEk1aRZVEkWdj2v7kfJLNQwZZgpXk' as Address;

/** Canonical off-chain metadata for a node cNFT (per-node name is set on-chain). */
export const NODE_METADATA_URI = 'https://www.weftnetwork.net/nft/node.json';
/** Tree depth (2^14 = 16,384 leaves per shard). */
export const TREE_MAX_DEPTH = 14;
export const TREE_MAX_BUFFER = 64;

const addrEnc = getAddressEncoder();

/** Bubblegum tree-config PDA: seeds `[merkle_tree]` under the Bubblegum program. */
export async function treeConfigPda(merkleTree: Address): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: BUBBLEGUM_PROGRAM,
    seeds: [addrEnc.encode(merkleTree)],
  });
  return pda;
}

/** The subset of `register_node` accounts the caller must supply for the cNFT mint. */
export interface MintAccounts {
  treeShard: Address;
  treeConfig: Address;
  merkleTree: Address;
  coreCollection: Address;
  mplCoreCpiSigner: Address;
  logWrapper: Address;
  compressionProgram: Address;
  metadataUri: string;
}

/** Assemble the mint accounts from the provisioned collection + tree. */
export async function mintAccounts(
  treeShard: Address,
  merkleTree: Address,
  collection: Address,
  metadataUri: string = NODE_METADATA_URI,
): Promise<MintAccounts> {
  return {
    treeShard,
    treeConfig: await treeConfigPda(merkleTree),
    merkleTree,
    coreCollection: collection,
    mplCoreCpiSigner: MPL_CORE_CPI_SIGNER,
    logWrapper: MPL_NOOP,
    compressionProgram: MPL_ACCOUNT_COMPRESSION,
    metadataUri,
  };
}

type RpcLike = {
  getAccountInfo: (
    addr: Address,
    cfg: { encoding: 'base64' },
  ) => { send: () => Promise<{ value: { data: [string, string] } | null }> };
};

/** Read the node-cNFT collection + active tree straight from the on-chain Registry
 *  (the authoritative source) and assemble the register_node mint accounts. Avoids
 *  any bundled manifest so a standalone node-agent works wherever the chain is
 *  provisioned. Throws if the registry has not been NFT-provisioned yet. */
export async function mintAccountsFromChain(
  rpc: RpcLike,
  metadataUri: string = NODE_METADATA_URI,
): Promise<MintAccounts> {
  const [registry] = await weft.findRegistryPda();
  const info = await rpc.getAccountInfo(registry, { encoding: 'base64' }).send();
  if (!info.value) throw new Error('Weft core is not initialized yet.');
  const reg = weft.getRegistryDecoder().decode(Buffer.from(info.value.data[0], 'base64'));
  const ZERO = '11111111111111111111111111111111';
  if (reg.collection === ZERO || reg.activeTree === ZERO || reg.treeCount === 0) {
    throw new Error('Node registry NFT tree is not provisioned yet — run `nft:provision` first.');
  }
  const [treeShard] = await weft.findTreeShardPda({ index: Number(reg.treeCount) - 1 });
  return mintAccounts(treeShard, reg.activeTree as Address, reg.collection as Address, metadataUri);
}
