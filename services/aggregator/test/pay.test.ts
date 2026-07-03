import { getAddressEncoder, type Address, type Blockhash } from '@solana/kit';
import { weft } from '@weft/sdk';
import { describe, expect, it } from 'vitest';

import {
  buildDepositEscrowTransaction,
  buildPayTrafficFromEscrowTransaction,
  buildPayTrafficTransaction,
  payLabel,
  type PayConfig,
} from '../src/pay';
import { makeSigner } from './helpers';

const addrEnc = getAddressEncoder();

function config(): PayConfig {
  return {
    rewardMint: makeSigner().address,
    rewardVault: makeSigner().address,
    treasury: makeSigner().address,
    tokenProgram: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' as Address,
    label: 'Weft VPN traffic',
  };
}

const FAKE_BLOCKHASH = {
  blockhash: '11111111111111111111111111111111' as Blockhash,
  lastValidBlockHeight: 1_000n,
};

describe('Solana Pay traffic payments', () => {
  it('returns the configured label', () => {
    expect(payLabel(config()).label).toBe('Weft VPN traffic');
  });

  it('builds an unsigned transaction carrying the settlement program', async () => {
    const account = makeSigner().address as Address;
    const cfg = config();
    const { transaction, message } = await buildPayTrafficTransaction(
      account,
      1_000_000n,
      cfg,
      FAKE_BLOCKHASH,
    );
    expect(message).toBe(cfg.label);
    const bytes = Buffer.from(transaction, 'base64');
    // the settlement program id appears among the transaction's static keys
    const programBytes = Buffer.from(addrEnc.encode(weft.WEFT_PROGRAM_ADDRESS) as Uint8Array);
    expect(bytes.includes(programBytes)).toBe(true);
  });

  it('builds unsigned escrow deposit and escrow settlement transactions', async () => {
    const account = makeSigner().address as Address;
    const cfg = config();
    const deposit = await buildDepositEscrowTransaction(account, 2_000_000n, cfg, FAKE_BLOCKHASH);
    const settle = await buildPayTrafficFromEscrowTransaction(
      account,
      1_000_000n,
      cfg,
      FAKE_BLOCKHASH,
    );
    const programBytes = Buffer.from(addrEnc.encode(weft.WEFT_PROGRAM_ADDRESS) as Uint8Array);
    expect(Buffer.from(deposit.transaction, 'base64').includes(programBytes)).toBe(true);
    expect(Buffer.from(settle.transaction, 'base64').includes(programBytes)).toBe(true);
    expect(deposit.message).toBe(cfg.label);
    expect(settle.message).toBe(cfg.label);
  });

  // Regression guard for the launch failure: pump.fun minted a Token-2022 token but the
  // clients hardcoded the classic Token program. Every settlement builder must thread the
  // reward mint's owning token program from PayConfig — never fall back to classic.
  it('threads the reward mint token program (Token-2022) into every settlement instruction', async () => {
    const account = makeSigner().address as Address;
    const T22 = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb' as Address;
    const CLASSIC = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' as Address;
    const cfg: PayConfig = { ...config(), tokenProgram: T22 };
    const t22Bytes = Buffer.from(addrEnc.encode(T22) as Uint8Array);
    const classicBytes = Buffer.from(addrEnc.encode(CLASSIC) as Uint8Array);
    const builders = [
      buildDepositEscrowTransaction,
      buildPayTrafficFromEscrowTransaction,
      buildPayTrafficTransaction,
    ];
    for (const build of builders) {
      const { transaction } = await build(account, 1_000_000n, cfg, FAKE_BLOCKHASH);
      const bytes = Buffer.from(transaction, 'base64');
      expect(bytes.includes(t22Bytes)).toBe(true); // Token-2022 program is referenced
      expect(bytes.includes(classicBytes)).toBe(false); // classic program is NOT wrongly used
    }
  });

  it('encodes the requested amount in the pay_traffic instruction', async () => {
    const payer = makeSigner();
    const cfg = config();
    const ix = await weft.getPayTrafficInstructionAsync({
      payer: { address: payer.address, signAndSendTransactions: async () => [] } as never,
      rewardMint: cfg.rewardMint,
      payerTokenAccount: makeSigner().address,
      rewardVault: cfg.rewardVault,
      treasury: cfg.treasury,
      amount: 1_234_567n,
    });
    const decoded = weft.getPayTrafficInstructionDataDecoder().decode(ix.data);
    expect(decoded.amount).toBe(1_234_567n);
  });

  it('encodes requested amounts in escrow instructions', async () => {
    const owner = makeSigner();
    const cfg = config();
    const signer = { address: owner.address, signAndSendTransactions: async () => [] } as never;
    const deposit = await weft.getDepositEscrowInstructionAsync({
      owner: signer,
      rewardMint: cfg.rewardMint,
      ownerTokenAccount: makeSigner().address,
      amount: 2_345_678n,
    });
    const settle = await weft.getPayTrafficFromEscrowInstructionAsync({
      owner: signer,
      escrowVault: makeSigner().address,
      rewardMint: cfg.rewardMint,
      rewardVault: cfg.rewardVault,
      treasury: cfg.treasury,
      amount: 3_456_789n,
    });
    expect(weft.getDepositEscrowInstructionDataDecoder().decode(deposit.data).amount).toBe(
      2_345_678n,
    );
    expect(weft.getPayTrafficFromEscrowInstructionDataDecoder().decode(settle.data).amount).toBe(
      3_456_789n,
    );
  });
});
