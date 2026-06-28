// Proposal-building helpers: turn an arbitrary on-chain instruction into the
// shape the DAO stores in a `ProposalTransaction`, and reconstruct the account
// list `execute_transaction` must pass as remaining accounts. This is what lets
// a proposal call ANY instruction (update ProtocolConfig, rotate an authority,
// transfer an ecosystem grant) once it passes.

import {
  AccountRole,
  isSignerRole,
  isWritableRole,
  type AccountMeta,
  type Address,
  type Instruction,
  type ReadonlyUint8Array,
} from '@solana/kit';

/** The on-chain `governance::TxAccountMeta` shape. */
export interface QueuedAccount {
  pubkey: Address;
  isSigner: boolean;
  isWritable: boolean;
}

/** A queued instruction, ready for `add_transaction`. */
export interface QueuedInstruction {
  programId: Address;
  accounts: QueuedAccount[];
  data: ReadonlyUint8Array;
}

/** Encode a kit instruction into the `ProposalTransaction` payload. */
export function encodeInstruction(ix: Instruction): QueuedInstruction {
  const accounts = (ix.accounts ?? []).map((a) => ({
    pubkey: a.address,
    isSigner: isSignerRole(a.role),
    isWritable: isWritableRole(a.role),
  }));
  return {
    programId: ix.programAddress,
    accounts,
    data: ix.data ?? new Uint8Array(),
  };
}

/**
 * The account metas `execute_transaction` appends as remaining accounts: the
 * target program (read-only) followed by every queued account — but the DAO PDA
 * is never a real transaction signer (it signs via `invoke_signed`), so all
 * signer flags are dropped to read-only/writable here.
 */
export function executeRemainingAccounts(
  queued: QueuedInstruction,
  governanceAuthority: Address,
): AccountMeta[] {
  const program: AccountMeta = {
    address: queued.programId,
    role: AccountRole.READONLY,
  };
  const metas = queued.accounts.map((a) => ({
    address: a.pubkey,
    role:
      a.pubkey === governanceAuthority
        ? a.isWritable
          ? AccountRole.WRITABLE
          : AccountRole.READONLY
        : a.isWritable
          ? AccountRole.WRITABLE
          : AccountRole.READONLY,
  }));
  return [program, ...metas];
}
