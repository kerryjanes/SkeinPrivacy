// Solana Pay transaction-request for `pay_traffic`. A user's wallet GETs the
// label, then POSTs its account; we return a base64 wire transaction containing
// exactly one `pay_traffic(amount)` instruction (70% → reward vault, 20% burned,
// 10% → treasury) for the wallet to sign. The user's reward-mint ATA is derived;
// the vault/treasury come from the on-chain `Distributor`.

import { TOKEN_PROGRAM_ADDRESS, findAssociatedTokenPda } from '@solana-program/token';
import {
  appendTransactionMessageInstruction,
  createNoopSigner,
  createTransactionMessage,
  getBase64EncodedWireTransaction,
  pipe,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  compileTransaction,
  type Address,
  type Blockhash,
} from '@solana/kit';
import { rewardsSettlement } from '@weft/sdk';

export interface PayConfig {
  rewardMint: Address;
  rewardVault: Address;
  treasury: Address;
  label: string;
}

export interface Blockhashish {
  blockhash: Blockhash;
  lastValidBlockHeight: bigint;
}

/** The label/icon shown by a wallet on the initial Solana Pay GET. */
export function payLabel(config: PayConfig): { label: string } {
  return { label: config.label };
}

/**
 * Build the unsigned `pay_traffic` transaction for `account` to sign. Pure given
 * a recent blockhash, so it is unit-testable without an RPC.
 */
export async function buildPayTrafficTransaction(
  account: Address,
  amount: bigint,
  config: PayConfig,
  latestBlockhash: Blockhashish,
): Promise<{ transaction: string; message: string }> {
  const payer = createNoopSigner(account);
  const [payerTokenAccount] = await findAssociatedTokenPda({
    owner: account,
    mint: config.rewardMint,
    tokenProgram: TOKEN_PROGRAM_ADDRESS,
  });

  const ix = await rewardsSettlement.getPayTrafficInstructionAsync({
    payer,
    rewardMint: config.rewardMint,
    payerTokenAccount,
    rewardVault: config.rewardVault,
    treasury: config.treasury,
    amount,
  });

  const message = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(payer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(latestBlockhash, m),
    (m) => appendTransactionMessageInstruction(ix, m),
  );

  const compiled = compileTransaction(message);
  return {
    transaction: getBase64EncodedWireTransaction(compiled),
    message: config.label,
  };
}
