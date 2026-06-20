// On-chain epoch posting. The aggregator holds the `poster_authority` keypair
// and submits the built epoch's merkle root via `post_epoch` (poster-gated and
// solvency-checked on-chain). Claim is permissionless, so payout liveness does
// not depend on this keypair.

import {
  appendTransactionMessageInstruction,
  createTransactionMessage,
  getSignatureFromTransaction,
  pipe,
  sendAndConfirmTransactionFactory,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
  type Rpc,
  type RpcSubscriptions,
  type SolanaRpcApi,
  type SolanaRpcSubscriptionsApi,
  type TransactionSigner,
} from '@solana/kit';
import { math, rewardsSettlement } from '@weft/sdk';

import type { EpochBuild } from './rewards';

export interface PostContext {
  rpc: Rpc<SolanaRpcApi>;
  rpcSubscriptions: RpcSubscriptions<SolanaRpcSubscriptionsApi>;
  poster: TransactionSigner;
}

/** Submit `post_epoch` for a built epoch; returns the confirmed signature. */
export async function postEpoch(ctx: PostContext, build: EpochBuild): Promise<string> {
  const [rewardVault] = await rewardsSettlement.findRewardVaultPda();
  const ix = await rewardsSettlement.getPostEpochInstructionAsync({
    poster: ctx.poster,
    rewardVault,
    epoch: build.epoch,
    merkleRoot: math.fromHex(build.root),
    totalReward: build.totalReward,
    numNodes: build.numNodes,
  });

  const { value: latestBlockhash } = await ctx.rpc.getLatestBlockhash().send();
  const message = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(ctx.poster, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(latestBlockhash, m),
    (m) => appendTransactionMessageInstruction(ix, m),
  );
  const signed = await signTransactionMessageWithSigners(message);
  const send = sendAndConfirmTransactionFactory({
    rpc: ctx.rpc,
    rpcSubscriptions: ctx.rpcSubscriptions,
  });
  // kit's generic inference widens the lifetime union; the message was built
  // with a blockhash lifetime, so this is sound at runtime.
  await send(signed as Parameters<typeof send>[0], { commitment: 'confirmed' });
  return getSignatureFromTransaction(signed);
}
