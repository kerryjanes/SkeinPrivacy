// Delegated settlement. The control plane holds the distributor's `poster_authority` key and bills
// each user's metered VPN usage straight from their prepaid escrow via `settle_from_escrow` — no
// per-charge user signature. This is what makes access seamless ("deposit once, use = earn") and
// keeps the reward vault funded so node payouts (aggregator post_epoch → claim) stay solvent: the
// same 1000 $WEFT/GB the user is metered at is split 70% vault / 20% burn / 10% treasury on chain.
//
// The settle authority can ONLY move escrow funds into those protocol splits, bounded by the
// escrow's own balance — it can never withdraw to an arbitrary account. Unspent balance stays
// withdrawable by the owner alone. If no keypair is configured, auto-settle is disabled and the
// legacy user-signed /settle path remains.

import { readFileSync } from 'node:fs';
import {
  address,
  appendTransactionMessageInstruction,
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
  type Rpc,
  type SolanaRpcApi,
} from '@solana/kit';
import { weft } from '@weft/sdk';

const TOKEN_2022_PROGRAM = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb' as Address;
const TOKEN_CLASSIC_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' as Address;

export interface Settler {
  /** The poster_authority pubkey this settler signs with. */
  authority: Address;
  /** Bill `amount` (reward-mint base units) from `wallet`'s escrow into the protocol splits. */
  settle(wallet: string, amount: bigint): Promise<string>;
}

/**
 * Build a settler bound to the on-chain distributor. Verifies the configured keypair really is the
 * distributor's `poster_authority` (fail fast, not at first settle) and caches the mint/vault/
 * treasury + token program so each settle is a single read (escrow) + one tx.
 */
export async function createSettler(
  rpcUrl: string,
  wsUrl: string,
  keypairPath: string,
): Promise<Settler> {
  const rpc: Rpc<SolanaRpcApi> = createSolanaRpc(rpcUrl);
  const rpcSubscriptions = createSolanaRpcSubscriptions(wsUrl);
  const poster = await createKeyPairSignerFromBytes(
    Uint8Array.from(JSON.parse(readFileSync(keypairPath, 'utf8')) as number[]),
  );
  const [distributorPda] = await weft.findDistributorPda();
  const dist = await weft.fetchDistributor(rpc, distributorPda);
  if (dist.data.posterAuthority !== poster.address) {
    throw new Error(
      `settle keypair ${poster.address} is not the distributor poster_authority ` +
        `${dist.data.posterAuthority} — refusing to auto-settle`,
    );
  }
  const rewardMint = dist.data.rewardMint;
  const rewardVault = dist.data.rewardVault;
  const treasury = dist.data.treasury;
  const tokenProgram = await resolveTokenProgram(rpc, rewardMint);

  const sendAndConfirm = sendAndConfirmTransactionFactory({ rpc, rpcSubscriptions });

  return {
    authority: poster.address,
    async settle(wallet: string, amount: bigint): Promise<string> {
      const [escrow] = await weft.findEscrowPda({ owner: address(wallet) });
      const esc = await weft.fetchPaymentEscrow(rpc, escrow);
      const ix = await weft.getSettleFromEscrowInstructionAsync({
        settleAuthority: poster,
        escrow,
        escrowVault: esc.data.vault,
        rewardMint,
        rewardVault,
        treasury,
        tokenProgram,
        amount,
      });
      const { value: latestBlockhash } = await rpc.getLatestBlockhash().send();
      const message = pipe(
        createTransactionMessage({ version: 0 }),
        (m) => setTransactionMessageFeePayerSigner(poster, m),
        (m) => setTransactionMessageLifetimeUsingBlockhash(latestBlockhash, m),
        (m) => appendTransactionMessageInstruction(ix, m),
      );
      const signed = await signTransactionMessageWithSigners(message);
      await sendAndConfirm(signed as Parameters<typeof sendAndConfirm>[0], {
        commitment: 'confirmed',
      });
      return getSignatureFromTransaction(signed);
    },
  };
}

/** The reward mint may be classic SPL or Token-2022; read the owner program from chain. */
async function resolveTokenProgram(rpc: Rpc<SolanaRpcApi>, mint: Address): Promise<Address> {
  const info = await rpc.getAccountInfo(mint, { encoding: 'base64' }).send();
  return info.value?.owner === TOKEN_2022_PROGRAM ? TOKEN_2022_PROGRAM : TOKEN_CLASSIC_PROGRAM;
}
