// Gasless node-reward payout. The aggregator holds the poster_authority key and, on the operator's
// request, submits `claim_rewards(node_id, earned_total)` — paying the node its cumulative earnings
// (net of what it already withdrew) straight to its token account in ONE transaction. The operator
// signs nothing and pays no fee. Funds can only ever land in `node.operator`'s account (enforced
// on-chain), so this poster-signed call can never redirect a node's rewards. No epoch, no dispute
// window: earnings accrue off-chain in real time (the aggregator's ledger) and settle here on demand.

import { readFileSync } from 'node:fs';
import {
  address,
  appendTransactionMessageInstructions,
  createKeyPairSignerFromBytes,
  createSolanaRpc,
  createSolanaRpcSubscriptions,
  createTransactionMessage,
  getSignatureFromTransaction,
  pipe,
  sendAndConfirmTransactionFactory,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
  type Address,
} from '@solana/kit';
import {
  findAssociatedTokenPda,
  getCreateAssociatedTokenIdempotentInstructionAsync,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import { weft } from '@weft/sdk';

const TOKEN_2022_PROGRAM = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb' as Address;

export interface Claimer {
  poster: string;
  /** Pay `earnedTotal - node.withdrawn` to the node operator. Returns the confirmed signature. */
  claim(operator: string, nodeId: bigint, earnedTotal: bigint): Promise<string>;
}

export async function createClaimer(
  rpcUrl: string,
  wsUrl: string,
  keypairPath: string,
): Promise<Claimer> {
  const rpc = createSolanaRpc(rpcUrl);
  const rpcSubscriptions = createSolanaRpcSubscriptions(wsUrl);
  const poster = await createKeyPairSignerFromBytes(
    Uint8Array.from(JSON.parse(readFileSync(keypairPath, 'utf8')) as number[]),
  );
  const [distributorPda] = await weft.findDistributorPda();
  const dist = await weft.fetchDistributor(rpc, distributorPda);
  if (dist.data.posterAuthority !== poster.address) {
    throw new Error(
      `claim keypair ${poster.address} is not the distributor poster_authority ` +
        `${dist.data.posterAuthority} — refusing to pay rewards`,
    );
  }
  const rewardMint = dist.data.rewardMint;
  const rewardVault = dist.data.rewardVault;
  const mintInfo = await rpc.getAccountInfo(rewardMint, { encoding: 'base64' }).send();
  const tokenProgram =
    mintInfo.value?.owner === TOKEN_2022_PROGRAM ? TOKEN_2022_PROGRAM : TOKEN_PROGRAM_ADDRESS;
  const sendAndConfirm = sendAndConfirmTransactionFactory({ rpc, rpcSubscriptions });

  return {
    poster: poster.address,
    async claim(operator: string, nodeId: bigint, earnedTotal: bigint): Promise<string> {
      const owner = address(operator);
      const [ata] = await findAssociatedTokenPda({ owner, mint: rewardMint, tokenProgram });
      // Create the operator's reward-token account if missing (idempotent) — poster pays the ~0.002
      // SOL rent once per node, so the operator needs nothing to receive rewards.
      const createAtaIx = await getCreateAssociatedTokenIdempotentInstructionAsync({
        payer: poster,
        owner,
        mint: rewardMint,
        tokenProgram,
      });
      const claimIx = await weft.getClaimRewardsInstructionAsync({
        poster,
        operator: owner,
        rewardMint,
        rewardVault,
        operatorTokenAccount: ata,
        tokenProgram,
        nodeId,
        earnedTotal,
      });
      const { value: latestBlockhash } = await rpc.getLatestBlockhash().send();
      const message = pipe(
        createTransactionMessage({ version: 0 }),
        (m) => setTransactionMessageFeePayerSigner(poster, m),
        (m) => setTransactionMessageLifetimeUsingBlockhash(latestBlockhash, m),
        (m) => appendTransactionMessageInstructions([createAtaIx, claimIx], m),
      );
      const signed = await signTransactionMessageWithSigners(message);
      await sendAndConfirm(signed as Parameters<typeof sendAndConfirm>[0], {
        commitment: 'confirmed',
      });
      return getSignatureFromTransaction(signed);
    },
  };
}
