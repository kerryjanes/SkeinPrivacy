import { keypairIdentity, type Umi } from '@metaplex-foundation/umi';
import { createUmi } from '@metaplex-foundation/umi-bundle-defaults';
import { mplBubblegum } from '@metaplex-foundation/mpl-bubblegum';
import { mplCore } from '@metaplex-foundation/mpl-core';

export function createUmiClient(rpcUrl: string, secret: Uint8Array): Umi {
  const umi = createUmi(rpcUrl).use(mplBubblegum()).use(mplCore());
  umi.use(keypairIdentity(umi.eddsa.createKeypairFromSecretKey(secret)));
  return umi;
}
