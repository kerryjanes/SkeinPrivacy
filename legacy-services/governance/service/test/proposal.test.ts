import { AccountRole, address, type Instruction } from '@solana/kit';
import { describe, expect, it } from 'vitest';

import { encodeInstruction, executeRemainingAccounts } from '../src/proposal';

const PROG = address('q3K9krqiQDL7WHVUzLZrjJLgsM53vSrcfNRTzsVE6eA');
const DAO = address('11111111111111111111111111111111');
const CFG = address('SysvarC1ock11111111111111111111111111111111');

describe('proposal encoding', () => {
  const ix: Instruction = {
    programAddress: PROG,
    accounts: [
      { address: DAO, role: AccountRole.READONLY_SIGNER },
      { address: CFG, role: AccountRole.WRITABLE },
    ],
    data: new Uint8Array([1, 2, 3, 4]),
  };

  it('maps roles into the on-chain TxAccountMeta shape', () => {
    const q = encodeInstruction(ix);
    expect(q.programId).toBe(PROG);
    expect(q.data).toEqual(new Uint8Array([1, 2, 3, 4]));
    expect(q.accounts).toEqual([
      { pubkey: DAO, isSigner: true, isWritable: false },
      { pubkey: CFG, isSigner: false, isWritable: true },
    ]);
  });

  it('drops the DAO PDA signer flag for execute remaining accounts', () => {
    const q = encodeInstruction(ix);
    const remaining = executeRemainingAccounts(q, DAO);
    // program first, then accounts — none marked as a real signer
    expect(remaining[0]).toEqual({ address: PROG, role: AccountRole.READONLY });
    expect(remaining[1].role).toBe(AccountRole.READONLY); // DAO PDA, signer dropped
    expect(remaining[2].role).toBe(AccountRole.WRITABLE); // protocol config
    expect(remaining.every((m) => m.role !== AccountRole.READONLY_SIGNER)).toBe(true);
    expect(remaining.every((m) => m.role !== AccountRole.WRITABLE_SIGNER)).toBe(true);
  });
});
