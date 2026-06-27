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
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';

export interface PayoutBackend {
  pay(recipient: string, amount: bigint): Promise<{ signature: string }>;
}

export class TokenPayout implements PayoutBackend {
  constructor(
    private rpcUrl: string,
    private wsUrl: string,
    private keypairPath: string,
    private mint: Address,
  ) {}

  async pay(recipient: string, amount: bigint): Promise<{ signature: string }> {
    if (amount <= 0n) throw new Error('withdraw amount must be positive');
    const payer = await createKeyPairSignerFromBytes(
      Uint8Array.from(JSON.parse(readFileSync(this.keypairPath, 'utf8')) as number[]),
    );
    const rpc = createSolanaRpc(this.rpcUrl);
    const sendAndConfirm = sendAndConfirmTransactionFactory({
      rpc,
      rpcSubscriptions: createSolanaRpcSubscriptions(this.wsUrl),
    });
    const owner = address(recipient);
    const [sourceAta] = await findAssociatedTokenPda({
      owner: payer.address,
      mint: this.mint,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
    });
    const [destinationAta] = await findAssociatedTokenPda({
      owner,
      mint: this.mint,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
    });
    const createAta = await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer,
      owner,
      mint: this.mint,
    });
    const transfer = getTransferCheckedInstruction({
      source: sourceAta,
      mint: this.mint,
      destination: destinationAta,
      authority: payer,
      amount,
      decimals: 9,
    });
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
