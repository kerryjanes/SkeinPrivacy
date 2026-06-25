// Devnet faucet: mint test $WEFT to a wallet so people can try the gated VPN end-to-end. Only
// active when WEFT_FAUCET_KEYPAIR is configured (i.e. the node is gating on a MINTABLE test mint);
// on mainnet (real mint, no faucet key) the route is simply absent.

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
  getMintToInstruction,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import { getTransferSolInstruction } from '@solana-program/system';

const lastDrip = new Map<string, number>();

export class Faucet {
  constructor(
    private rpcUrl: string,
    private wsUrl: string,
    private keypairPath: string,
    private mint: string,
    private amount: bigint,
    private cooldownMs = 6 * 60 * 60 * 1000, // 6h per wallet
  ) {}

  /** Mint `amount` test $WEFT to `wallet` (creating its ATA if needed). Returns the tx signature. */
  async drip(wallet: string): Promise<{ signature: string; amount: string }> {
    const now = Date.now();
    const prev = lastDrip.get(wallet) ?? 0;
    if (now - prev < this.cooldownMs) {
      const mins = Math.ceil((this.cooldownMs - (now - prev)) / 60000);
      throw new Error(`faucet cooldown — try again in ~${mins} min`);
    }
    const faucet = await createKeyPairSignerFromBytes(
      Uint8Array.from(JSON.parse(readFileSync(this.keypairPath, 'utf8')) as number[]),
    );
    const rpc = createSolanaRpc(this.rpcUrl);
    const sendAndConfirm = sendAndConfirmTransactionFactory({
      rpc,
      rpcSubscriptions: createSolanaRpcSubscriptions(this.wsUrl),
    });
    const mint = address(this.mint);
    const owner = address(wallet);
    const [ata] = await findAssociatedTokenPda({
      owner,
      mint,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
    });
    const createAta = await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: faucet,
      owner,
      mint,
    });
    const mintTo = getMintToInstruction({
      mint,
      token: ata,
      mintAuthority: faucet,
      amount: this.amount,
    });
    // Also drip a little SOL so a node operator's key can pay its one-time on-chain registration.
    const sol = getTransferSolInstruction({
      source: faucet,
      destination: owner,
      amount: 20_000_000n,
    });
    const { value: bh } = await rpc.getLatestBlockhash().send();
    const msg = pipe(
      createTransactionMessage({ version: 0 }),
      (m) => setTransactionMessageFeePayerSigner(faucet, m),
      (m) => setTransactionMessageLifetimeUsingBlockhash(bh, m),
      (m) => appendTransactionMessageInstructions([sol, createAta, mintTo], m),
    );
    const signed = await signTransactionMessageWithSigners(msg);
    await sendAndConfirm(signed as Parameters<typeof sendAndConfirm>[0], {
      commitment: 'confirmed',
    });
    lastDrip.set(wallet, now);
    return { signature: getSignatureFromTransaction(signed), amount: this.amount.toString() };
  }
}
