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
  getTransferCheckedInstruction,
} from '@solana-program/token';

export interface PayoutBackend {
  availableBalance(): Promise<bigint>;
  pay(recipient: string, amount: bigint): Promise<{ signature: string }>;
}

export class TokenPayout implements PayoutBackend {
  constructor(
    private rpcUrl: string,
    private wsUrl: string,
    private keypairPath: string,
    private mint: Address,
    private decimals: number,
    // The mint's owning token program (classic SPL or Token-2022), read from the
    // mint account at runtime. Every ATA + transfer must use it or land on the
    // wrong account / wrong program.
    private tokenProgram: Address,
  ) {}

  private async payer() {
    return createKeyPairSignerFromBytes(
      Uint8Array.from(JSON.parse(readFileSync(this.keypairPath, 'utf8')) as number[]),
    );
  }

  async availableBalance(): Promise<bigint> {
    const payer = await this.payer();
    const rpc = createSolanaRpc(this.rpcUrl);
    const [sourceAta] = await findAssociatedTokenPda({
      owner: payer.address,
      mint: this.mint,
      tokenProgram: this.tokenProgram,
    });
    try {
      const { value } = await rpc.getTokenAccountBalance(sourceAta).send();
      return BigInt(value.amount);
    } catch {
      return 0n;
    }
  }

  async pay(recipient: string, amount: bigint): Promise<{ signature: string }> {
    if (amount <= 0n) throw new Error('withdraw amount must be positive');
    const payer = await this.payer();
    const rpc = createSolanaRpc(this.rpcUrl);
    const sendAndConfirm = sendAndConfirmTransactionFactory({
      rpc,
      rpcSubscriptions: createSolanaRpcSubscriptions(this.wsUrl),
    });
    const owner = address(recipient);
    const [sourceAta] = await findAssociatedTokenPda({
      owner: payer.address,
      mint: this.mint,
      tokenProgram: this.tokenProgram,
    });
    const [destinationAta] = await findAssociatedTokenPda({
      owner,
      mint: this.mint,
      tokenProgram: this.tokenProgram,
    });
    const createAta = await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer,
      owner,
      mint: this.mint,
      tokenProgram: this.tokenProgram,
    });
    const transfer = getTransferCheckedInstruction(
      {
        source: sourceAta,
        mint: this.mint,
        destination: destinationAta,
        authority: payer,
        amount,
        decimals: this.decimals,
      },
      { programAddress: this.tokenProgram },
    );
    const { value: bh } = await rpc.getLatestBlockhash().send();
    const msg = pipe(
      createTransactionMessage({ version: 0 }),
      (m) => setTransactionMessageFeePayerSigner(payer, m),
      (m) => setTransactionMessageLifetimeUsingBlockhash(bh, m),
      (m) => appendTransactionMessageInstructions([createAta, transfer], m),
    );
    const signed = await signTransactionMessageWithSigners(msg);
    await sendAndConfirm(signed as Parameters<typeof sendAndConfirm>[0], {
      commitment: 'confirmed',
    });
    return { signature: getSignatureFromTransaction(signed) };
  }
}
