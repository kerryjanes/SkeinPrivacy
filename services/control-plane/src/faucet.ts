// Devnet faucet: transfer test $WEFT to a wallet so people can try the gated VPN end-to-end.
// The rehearsal mint has a fixed supply and retired mint authority, so this must move tokens from
// a funded devnet faucet/custody wallet instead of minting new tokens. On mainnet the route is
// disabled by config.

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
} from '@solana/kit';
import {
  findAssociatedTokenPda,
  getCreateAssociatedTokenIdempotentInstructionAsync,
  getTransferCheckedInstruction,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import { getTransferSolInstruction } from '@solana-program/system';

const lastDrip = new Map<string, number>();
const lastSolDrip = new Map<string, number>();

export class Faucet {
  constructor(
    private rpcUrl: string,
    private wsUrl: string,
    private keypairPath: string,
    private mint: string,
    private amount: bigint,
    private solLamports: bigint,
    private cooldownMs = 6 * 60 * 60 * 1000, // 6h per wallet
  ) {}

  private async ctx() {
    const faucet = await createKeyPairSignerFromBytes(
      Uint8Array.from(JSON.parse(readFileSync(this.keypairPath, 'utf8')) as number[]),
    );
    const rpc = createSolanaRpc(this.rpcUrl);
    const sendAndConfirm = sendAndConfirmTransactionFactory({
      rpc,
      rpcSubscriptions: createSolanaRpcSubscriptions(this.wsUrl),
    });
    return { faucet, rpc, sendAndConfirm };
  }

  /** Transfer `amount` test $WEFT to `wallet` (creating its ATA if needed). Returns the tx sig. */
  async drip(wallet: string): Promise<{ signature: string; amount: string }> {
    const now = Date.now();
    const prev = lastDrip.get(wallet) ?? 0;
    if (now - prev < this.cooldownMs) {
      const mins = Math.ceil((this.cooldownMs - (now - prev)) / 60000);
      throw new Error(`faucet cooldown — try again in ~${mins} min`);
    }
    const { faucet, rpc, sendAndConfirm } = await this.ctx();
    const mint = address(this.mint);
    const owner = address(wallet);
    const [sourceAta] = await findAssociatedTokenPda({
      owner: faucet.address,
      mint,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
    });
    const [destinationAta] = await findAssociatedTokenPda({
      owner,
      mint,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
    });
    const createAta = await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: faucet,
      owner,
      mint,
    });
    const transfer = getTransferCheckedInstruction({
      source: sourceAta,
      mint,
      destination: destinationAta,
      authority: faucet,
      amount: this.amount,
      decimals: 9,
    });
    const { value: bh } = await rpc.getLatestBlockhash().send();
    const msg = pipe(
      createTransactionMessage({ version: 0 }),
      (m) => setTransactionMessageFeePayerSigner(faucet, m),
      (m) => setTransactionMessageLifetimeUsingBlockhash(bh, m),
      (m) => appendTransactionMessageInstructions([createAta, transfer], m),
    );
    const signed = await signTransactionMessageWithSigners(msg);
    await sendAndConfirm(signed as Parameters<typeof sendAndConfirm>[0], {
      commitment: 'confirmed',
    });
    lastDrip.set(wallet, now);
    return { signature: getSignatureFromTransaction(signed), amount: this.amount.toString() };
  }

  /** Transfer a small amount of devnet SOL for wallet transaction fees. Disabled with faucet. */
  async dripSol(wallet: string): Promise<{ signature: string; lamports: string }> {
    const now = Date.now();
    const prev = lastSolDrip.get(wallet) ?? 0;
    if (now - prev < this.cooldownMs) {
      const mins = Math.ceil((this.cooldownMs - (now - prev)) / 60000);
      throw new Error(`SOL faucet cooldown — try again in ~${mins} min`);
    }
    const { faucet, rpc, sendAndConfirm } = await this.ctx();
    const transfer = getTransferSolInstruction({
      source: faucet,
      destination: address(wallet),
      amount: this.solLamports,
    });
    const { value: bh } = await rpc.getLatestBlockhash().send();
    const msg = pipe(
      createTransactionMessage({ version: 0 }),
      (m) => setTransactionMessageFeePayerSigner(faucet, m),
      (m) => setTransactionMessageLifetimeUsingBlockhash(bh, m),
      (m) => appendTransactionMessageInstructions([transfer], m),
    );
    const signed = await signTransactionMessageWithSigners(msg);
    await sendAndConfirm(signed as Parameters<typeof sendAndConfirm>[0], {
      commitment: 'confirmed',
    });
    lastSolDrip.set(wallet, now);
    return {
      signature: getSignatureFromTransaction(signed),
      lamports: this.solLamports.toString(),
    };
  }
}
