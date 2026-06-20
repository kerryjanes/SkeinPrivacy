// Minimal @solana/kit connection + send helpers for the devnet smoke (mirrors
// the genesis service's rpc helper).

import { readFileSync } from 'node:fs';
import {
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
  type Instruction,
  type KeyPairSigner,
  type TransactionSigner,
} from '@solana/kit';

export interface Connection {
  rpc: ReturnType<typeof createSolanaRpc>;
  rpcSubscriptions: ReturnType<typeof createSolanaRpcSubscriptions>;
  sendAndConfirm: ReturnType<typeof sendAndConfirmTransactionFactory>;
}

export function connect(rpcUrl: string, wsUrl: string): Connection {
  const rpc = createSolanaRpc(rpcUrl);
  const rpcSubscriptions = createSolanaRpcSubscriptions(wsUrl);
  const sendAndConfirm = sendAndConfirmTransactionFactory({ rpc, rpcSubscriptions });
  return { rpc, rpcSubscriptions, sendAndConfirm };
}

export async function loadSigner(path: string): Promise<KeyPairSigner> {
  const bytes = Uint8Array.from(JSON.parse(readFileSync(path, 'utf8')) as number[]);
  return createKeyPairSignerFromBytes(bytes);
}

/** The raw 32-byte ed25519 seed from a Solana 64-byte secret-key file. */
export function loadEd25519Seed(path: string): Uint8Array {
  const bytes = Uint8Array.from(JSON.parse(readFileSync(path, 'utf8')) as number[]);
  return bytes.slice(0, 32);
}

export async function send(
  conn: Connection,
  feePayer: TransactionSigner,
  instructions: Instruction[],
): Promise<string> {
  const { value: latestBlockhash } = await conn.rpc.getLatestBlockhash().send();
  const message = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(feePayer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(latestBlockhash, m),
    (m) => appendTransactionMessageInstructions(instructions, m),
  );
  const signed = await signTransactionMessageWithSigners(message);
  await conn.sendAndConfirm(signed as Parameters<typeof conn.sendAndConfirm>[0], {
    commitment: 'confirmed',
  });
  return getSignatureFromTransaction(signed);
}
