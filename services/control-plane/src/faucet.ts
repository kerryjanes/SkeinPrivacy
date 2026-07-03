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
  type Address,
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

// The devnet test mint may be classic SPL or Token-2022 (mirroring pump.fun). Read the
// owning program + decimals from the mint account instead of assuming either.
const TOKEN_2022_PROGRAM_ADDRESS = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb' as Address;

export class Faucet {
  private mintCache?: { tokenProgram: Address; decimals: number };

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

  /** The mint's owning token program + decimals, read from chain once and cached. */
  private async mintInfo(
    rpc: ReturnType<typeof createSolanaRpc>,
  ): Promise<{ tokenProgram: Address; decimals: number }> {
    if (this.mintCache) return this.mintCache;
    const info = await rpc.getAccountInfo(address(this.mint), { encoding: 'base64' }).send();
    if (!info.value) throw new Error(`faucet mint ${this.mint} not found`);
    const tokenProgram =
      info.value.owner === TOKEN_2022_PROGRAM_ADDRESS
        ? TOKEN_2022_PROGRAM_ADDRESS
        : TOKEN_PROGRAM_ADDRESS;
    const decimals = Buffer.from(info.value.data[0], 'base64')[44]; // SPL Mint: decimals @ byte 44
    this.mintCache = { tokenProgram, decimals };
    return this.mintCache;
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
    const { tokenProgram, decimals } = await this.mintInfo(rpc);
    const [sourceAta] = await findAssociatedTokenPda({
      owner: faucet.address,
      mint,
      tokenProgram,
    });
    const [destinationAta] = await findAssociatedTokenPda({
      owner,
      mint,
      tokenProgram,
    });
    const createAta = await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: faucet,
      owner,
      mint,
      tokenProgram,
    });
    const transfer = getTransferCheckedInstruction(
      {
        source: sourceAta,
        mint,
        destination: destinationAta,
        authority: faucet,
        amount: this.amount,
        decimals,
      },
      { programAddress: tokenProgram },
    );
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
