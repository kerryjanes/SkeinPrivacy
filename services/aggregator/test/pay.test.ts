import { getAddressEncoder, type Address, type Blockhash } from '@solana/kit';
import { rewardsSettlement } from '@weft/sdk';
import { describe, expect, it } from 'vitest';

import { buildPayTrafficTransaction, payLabel, type PayConfig } from '../src/pay';
import { makeSigner } from './helpers';

const addrEnc = getAddressEncoder();

function config(): PayConfig {
  return {
    rewardMint: makeSigner().address,
    rewardVault: makeSigner().address,
    treasury: makeSigner().address,
    label: 'Weft VPN traffic',
  };
}

const FAKE_BLOCKHASH = {
  blockhash: '11111111111111111111111111111111' as Blockhash,
  lastValidBlockHeight: 1_000n,
};

describe('Solana Pay pay_traffic', () => {
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
    const programBytes = Buffer.from(
      addrEnc.encode(rewardsSettlement.REWARDS_SETTLEMENT_PROGRAM_ADDRESS) as Uint8Array,
    );
    expect(bytes.includes(programBytes)).toBe(true);
  });

  it('encodes the requested amount in the pay_traffic instruction', async () => {
    const payer = makeSigner();
    const cfg = config();
    const ix = await rewardsSettlement.getPayTrafficInstructionAsync({
      payer: { address: payer.address, signAndSendTransactions: async () => [] } as never,
      rewardMint: cfg.rewardMint,
      payerTokenAccount: makeSigner().address,
      rewardVault: cfg.rewardVault,
      treasury: cfg.treasury,
      amount: 1_234_567n,
    });
    const decoded = rewardsSettlement.getPayTrafficInstructionDataDecoder().decode(ix.data);
    expect(decoded.amount).toBe(1_234_567n);
  });
});
