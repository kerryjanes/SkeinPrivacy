// Minimal @solana/kit send-and-confirm (mirrors services/genesis/src/rpc.ts).

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
  sendAndConfirm: ReturnType<typeof sendAndConfirmTransactionFactory>;
}

export function connect(rpcUrl: string, wsUrl: string): Connection {
  const rpc = createSolanaRpc(rpcUrl);
  const rpcSubscriptions = createSolanaRpcSubscriptions(wsUrl);
  return { rpc, sendAndConfirm: sendAndConfirmTransactionFactory({ rpc, rpcSubscriptions }) };
}

export async function loadSigner(keypairPath: string): Promise<KeyPairSigner> {
  return createKeyPairSignerFromBytes(
    Uint8Array.from(JSON.parse(readFileSync(keypairPath, 'utf8')) as number[]),
  );
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
