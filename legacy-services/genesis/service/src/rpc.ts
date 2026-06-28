// Thin @solana/kit RPC wrapper + a send-and-confirm helper.

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
  type Rpc,
  type RpcSubscriptions,
  type SolanaRpcApi,
  type SolanaRpcSubscriptionsApi,
  type TransactionSigner,
} from '@solana/kit';

export interface Connection {
  rpc: Rpc<SolanaRpcApi>;
  rpcSubscriptions: RpcSubscriptions<SolanaRpcSubscriptionsApi>;
  sendAndConfirm: ReturnType<typeof sendAndConfirmTransactionFactory>;
}

export function connect(rpcUrl: string, wsUrl: string): Connection {
  const rpc = createSolanaRpc(rpcUrl);
  const rpcSubscriptions = createSolanaRpcSubscriptions(wsUrl);
  const sendAndConfirm = sendAndConfirmTransactionFactory({ rpc, rpcSubscriptions });
  return { rpc, rpcSubscriptions, sendAndConfirm };
}

export async function loadSigner(keypairPath: string): Promise<KeyPairSigner> {
  const bytes = Uint8Array.from(JSON.parse(readFileSync(keypairPath, 'utf8')) as number[]);
  return createKeyPairSignerFromBytes(bytes);
}

/** Build, sign (collecting all embedded signers), send, and confirm a tx. */
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
  // kit's generic inference widens the lifetime union; the message was built
  // with a blockhash lifetime, so this is sound at runtime.
  await conn.sendAndConfirm(signed as Parameters<typeof conn.sendAndConfirm>[0], {
    commitment: 'confirmed',
  });
  return getSignatureFromTransaction(signed);
}
