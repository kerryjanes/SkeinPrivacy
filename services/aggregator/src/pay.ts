// Solana Pay transaction-request helpers. A user's wallet GETs the
// label, then POSTs its account; we return a base64 wire transaction containing
// exactly one settlement instruction for the wallet to sign. The user's
// reward-mint ATA is derived; the vault/treasury come from the on-chain
// `Distributor`.

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
import { weft } from '@weft/sdk';

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

async function buildSingleInstructionTransaction(
  account: Address,
  config: PayConfig,
  latestBlockhash: Blockhashish,
  ix: Parameters<typeof appendTransactionMessageInstruction>[0],
): Promise<{ transaction: string; message: string }> {
  const payer = createNoopSigner(account);
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

/**
 * Build the unsigned `deposit_escrow` transaction for `account` to sign. This is the mainnet-first
 * prepaid path: funds move from the wallet's token account into its PDA escrow vault.
 */
export async function buildDepositEscrowTransaction(
  account: Address,
  amount: bigint,
  config: PayConfig,
  latestBlockhash: Blockhashish,
): Promise<{ transaction: string; message: string }> {
  const owner = createNoopSigner(account);
  const [ownerTokenAccount] = await findAssociatedTokenPda({
    owner: account,
    mint: config.rewardMint,
    tokenProgram: TOKEN_PROGRAM_ADDRESS,
  });

  const ix = await weft.getDepositEscrowInstructionAsync({
    owner,
    rewardMint: config.rewardMint,
    ownerTokenAccount,
    amount,
  });

  return buildSingleInstructionTransaction(account, config, latestBlockhash, ix);
}

/**
 * Build the unsigned `pay_traffic_from_escrow` transaction for `account` to sign. It debits prepaid
 * escrow and applies the governed node/treasury/burn split.
 */
export async function buildPayTrafficFromEscrowTransaction(
  account: Address,
  amount: bigint,
  config: PayConfig,
  latestBlockhash: Blockhashish,
): Promise<{ transaction: string; message: string }> {
  const owner = createNoopSigner(account);
  const [escrowVault] = await weft.findEscrowVaultPda({ owner: account });

  const ix = await weft.getPayTrafficFromEscrowInstructionAsync({
    owner,
    escrowVault,
    rewardMint: config.rewardMint,
    rewardVault: config.rewardVault,
    treasury: config.treasury,
    amount,
  });

  return buildSingleInstructionTransaction(account, config, latestBlockhash, ix);
}

/**
 * Build the legacy unsigned `pay_traffic` transaction for `account` to sign. Kept for devnet
 * compatibility; public mainnet UX should prefer deposit + pay-from-escrow.
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

  const ix = await weft.getPayTrafficInstructionAsync({
    payer,
    rewardMint: config.rewardMint,
    payerTokenAccount,
    rewardVault: config.rewardVault,
    treasury: config.treasury,
    amount,
  });

  return buildSingleInstructionTransaction(account, config, latestBlockhash, ix);
}
