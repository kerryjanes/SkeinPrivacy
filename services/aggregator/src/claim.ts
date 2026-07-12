// User-paid reward withdrawal with poster attestation. The on-chain `claim_rewards` requires the
// poster_authority as a CO-SIGNER (it vouches for how much a node earned — the amount lives in the
// off-chain ledger), but the OPERATOR is the fee payer and signs first. Flow:
//   1. cabinet builds a claim_rewards tx (operator = fee payer, poster = empty signer slot), signs
//      it with the operator's wallet, and POSTs the partially-signed wire here;
//   2. this module VALIDATES the tx (only a bounded claim_rewards + ATA-create/compute-budget — never
//      a settle_from_escrow or a foreign program, so the poster can't be tricked into draining the
//      vault or billing an escrow), co-signs it with the poster key, and submits it.
// The operator pays the fee (and one-time ATA rent); the poster pays nothing and signs only a claim
// it has verified against the ledger.

import { readFileSync } from 'node:fs';
import {
  createKeyPairFromBytes,
  createSolanaRpc,
  getBase64EncodedWireTransaction,
  getBase64Encoder,
  getCompiledTransactionMessageDecoder,
  getSignatureFromTransaction,
  getTransactionDecoder,
  partiallySignTransaction,
  type Address,
} from '@solana/kit';
import { ASSOCIATED_TOKEN_PROGRAM_ADDRESS } from '@solana-program/token';
import { weft } from '@weft/sdk';

const COMPUTE_BUDGET = 'ComputeBudget111111111111111111111111111111' as Address;
const ALLOWED_PROGRAMS = new Set<string>([
  weft.WEFT_PROGRAM_ADDRESS,
  ASSOCIATED_TOKEN_PROGRAM_ADDRESS,
  COMPUTE_BUDGET,
]);
const CLAIM_DISC = Uint8Array.from(weft.CLAIM_REWARDS_DISCRIMINATOR);

export interface CoSigner {
  poster: string;
  /**
   * Validate an operator-signed claim_rewards tx, co-sign with the poster, and submit. `maxEarned`
   * bounds the payout to the node's ledger earnings; the tx must contain no instruction other than a
   * single matching claim_rewards (+ optional ATA-create / compute-budget). Returns the signature.
   */
  coSign(
    wireBase64: string,
    ctx: { operator: string; nodeId: bigint; maxEarned: bigint },
  ): Promise<string>;
}

function u64le(data: Uint8Array, offset: number): bigint {
  return new DataView(data.buffer, data.byteOffset + offset, 8).getBigUint64(0, true);
}
function eq(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

export async function createCoSigner(rpcUrl: string, keypairPath: string): Promise<CoSigner> {
  const rpc = createSolanaRpc(rpcUrl);
  const bytes = Uint8Array.from(JSON.parse(readFileSync(keypairPath, 'utf8')) as number[]);
  const posterKeyPair = await createKeyPairFromBytes(bytes);
  const [distributorPda] = await weft.findDistributorPda();
  const dist = await weft.fetchDistributor(rpc, distributorPda);
  const posterAddress = dist.data.posterAuthority;

  return {
    poster: posterAddress,
    async coSign(wireBase64, { operator, nodeId, maxEarned }): Promise<string> {
      const tx = getTransactionDecoder().decode(getBase64Encoder().encode(wireBase64));
      const msg = getCompiledTransactionMessageDecoder().decode(tx.messageBytes) as unknown as {
        staticAccounts: readonly Address[];
        instructions: ReadonlyArray<{
          programAddressIndex: number;
          accountIndices?: readonly number[];
          data?: Uint8Array;
        }>;
      };
      const accounts = msg.staticAccounts;

      let sawClaim = false;
      for (const ix of msg.instructions) {
        const programId = accounts[ix.programAddressIndex];
        if (!ALLOWED_PROGRAMS.has(programId)) {
          throw new Error(`disallowed program in claim tx: ${programId}`);
        }
        if (programId !== weft.WEFT_PROGRAM_ADDRESS) continue;
        const data = (ix.data ?? new Uint8Array()) as Uint8Array;
        if (data.length < 24 || !eq(data.slice(0, 8), CLAIM_DISC)) {
          throw new Error('only claim_rewards is permitted from the program in a claim tx');
        }
        const ixNodeId = u64le(data, 8);
        const earnedTotal = u64le(data, 16);
        // claim_rewards accounts: [poster, operator, distributor, node, mint, vault, opAta, tokenProg]
        const opAccount = accounts[(ix.accountIndices ?? [])[1]];
        if (ixNodeId !== nodeId) throw new Error('claim node_id mismatch');
        if (opAccount !== (operator as Address)) throw new Error('claim operator mismatch');
        if (earnedTotal > maxEarned) throw new Error('claim earned_total exceeds ledger');
        sawClaim = true;
      }
      if (!sawClaim) throw new Error('no claim_rewards instruction found');

      const signed = await partiallySignTransaction([posterKeyPair], tx);
      const sig = getSignatureFromTransaction(signed);
      await rpc
        .sendTransaction(getBase64EncodedWireTransaction(signed), {
          encoding: 'base64',
          skipPreflight: false,
          maxRetries: 5n,
        })
        .send();
      for (let i = 0; i < 40; i++) {
        const { value } = await rpc.getSignatureStatuses([sig]).send();
        const st = value[0];
        if (st) {
          if (st.err) throw new Error(`claim tx failed: ${JSON.stringify(st.err)}`);
          if (st.confirmationStatus === 'confirmed' || st.confirmationStatus === 'finalized')
            return sig;
        }
        await new Promise((r) => setTimeout(r, 1500));
      }
      throw new Error('claim tx not confirmed in time');
    },
  };
}
