// Enumerate an operator's stake positions so a voter can cast its full weight:
// stake lives in per-(operator,node_id) `StakePosition` PDAs with no aggregate,
// so total voting power = the sum across all of an operator's positions, cast one
// `cast_vote` per position.

import { getBase58Decoder, type Address, type Base58EncodedBytes } from '@solana/kit';
import { staking } from '@weft/sdk';

export interface OperatorPosition {
  address: Address;
  nodeId: bigint;
  amount: bigint;
  lockedUntil: bigint;
}

type Rpc = Parameters<typeof staking.fetchStakePosition>[0];

/** All stake positions owned by `operator`, via getProgramAccounts. */
export async function operatorPositions(rpc: Rpc, operator: Address): Promise<OperatorPosition[]> {
  const disc = getBase58Decoder().decode(staking.STAKE_POSITION_DISCRIMINATOR);
  const accounts = await (rpc as ReturnType<typeof import('@solana/kit').createSolanaRpc>)
    .getProgramAccounts(staking.STAKING_PROGRAM_ADDRESS, {
      encoding: 'base64',
      filters: [
        { memcmp: { offset: 0n, bytes: disc as Base58EncodedBytes, encoding: 'base58' } },
        // operator is the first field after the 8-byte discriminator.
        {
          memcmp: {
            offset: 8n,
            bytes: operator as unknown as Base58EncodedBytes,
            encoding: 'base58',
          },
        },
      ],
    })
    .send();
  const decoder = staking.getStakePositionDecoder();
  return accounts.map(({ pubkey, account }) => {
    const bytes = Buffer.from((account.data as readonly [string, string])[0], 'base64');
    const d = decoder.decode(bytes);
    return {
      address: pubkey,
      nodeId: d.nodeId,
      amount: d.amount,
      lockedUntil: d.lockedUntil,
    };
  });
}

/** Total voting weight an operator could cast on a proposal ending at `votingEndsAt`. */
export function eligibleWeight(positions: OperatorPosition[], votingEndsAt: bigint): bigint {
  return positions
    .filter((p) => p.lockedUntil >= votingEndsAt)
    .reduce((sum, p) => sum + p.amount, 0n);
}
