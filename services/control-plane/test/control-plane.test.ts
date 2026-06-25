import { describe, expect, it } from 'vitest';
import { rewardsSettlement } from '@weft/sdk';
import { loadConfig } from '../src/config.js';
import { costBaseUnits, decodePayTraffic, quotaBytes } from '../src/chain.js';
import { multiHopLink, oneHopLink } from '../src/links.js';
import { parseUsage, renderConfig } from '../src/xray.js';
import type { User } from '../src/store.js';

const cfg = loadConfig();
const user = (over: Partial<User> = {}): User => ({
  wallet: 'Wa11et1111111111111111111111111111111111111',
  uuid: '11111111-2222-3333-4444-555555555555',
  email: 'Wa11et1111111111111111111111111111111111111',
  unsettledBytes: '0',
  balanceBaseUnits: '0',
  quotaBytes: '0',
  active: true,
  createdAt: 0,
  ...over,
});

describe('pricing (0.1 WEFT/GB)', () => {
  it('1 WEFT buys 10 GB', () => {
    expect(quotaBytes(1_000_000_000n)).toBe(10_000_000_000n); // 1 WEFT → 10 GB
  });
  it('0.0001 WEFT buys ~1 MB (the E2E cutoff size)', () => {
    expect(quotaBytes(100_000n)).toBe(1_000_000n);
  });
  it('cost is the inverse of quota', () => {
    expect(costBaseUnits(10_000_000_000n)).toBe(1_000_000_000n); // 10 GB costs 1 WEFT
    expect(costBaseUnits(quotaBytes(5_000_000_000n))).toBe(5_000_000_000n);
  });
});

describe('personal links', () => {
  it('1-hop carries the per-user uuid, vision flow, port + Reality params', () => {
    const l = oneHopLink(cfg, 'abc-uuid');
    expect(l).toContain('vless://abc-uuid@');
    expect(l).toContain(`:${cfg.hop1Port}?`);
    expect(l).toContain('flow=xtls-rprx-vision');
    expect(l).toContain(`sni=${cfg.sni}`);
    expect(l).toContain(`pbk=${cfg.realityPublicKey}`);
    expect(l).toContain('#Weft-1hop');
  });
  it('multihop targets the Tor port and omits the vision flow', () => {
    const l = multiHopLink(cfg, 'abc-uuid');
    expect(l).toContain(`:${cfg.hopnPort}?`);
    expect(l).not.toContain('flow=');
    expect(l).toContain('#Weft-multihop');
  });
});

describe('xray config render', () => {
  it('enables the api+stats and always keeps the founder, adding active users', () => {
    const c = renderConfig(cfg, [user({ uuid: 'u-1', email: 'w1' })]) as any;
    expect(c.api.services).toContain('StatsService');
    expect(c.policy.levels['0'].statsUserUplink).toBe(true);
    const hop1 = c.inbounds.find((i: any) => i.tag === 'hop1');
    const hopN = c.inbounds.find((i: any) => i.tag === 'hopN');
    const ids = (i: any) => i.settings.clients.map((cl: any) => cl.id);
    expect(ids(hop1)).toEqual([cfg.founderUuid, 'u-1']);
    expect(ids(hopN)).toEqual([cfg.founderUuid, 'u-1']);
    // 1-hop clients get the vision flow; the Tor hop does not
    expect(hop1.settings.clients.every((cl: any) => cl.flow === 'xtls-rprx-vision')).toBe(true);
    expect(hopN.settings.clients.every((cl: any) => cl.flow === undefined)).toBe(true);
  });
  it('renders a valid founder-only config when there are no active users', () => {
    const c = renderConfig(cfg, []) as any;
    const hop1 = c.inbounds.find((i: any) => i.tag === 'hop1');
    expect(hop1.settings.clients).toHaveLength(1);
    expect(hop1.settings.clients[0].email).toBe('founder');
  });
});

describe('pay_traffic settlement verification', () => {
  const PROGRAM = String(rewardsSettlement.REWARDS_SETTLEMENT_PROGRAM_ADDRESS);
  const PAYER = '2m5CoAk7ioZJbRYqHV9PJMNZN2gwpTPKQXR4GKyVifL7';
  const OTHER = 'FdLn2UPCmGGxzRDvX54qcQSrCyHSTzmaNeYdxY21FxNt';

  const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  function b58(bytes: Uint8Array): string {
    let n = 0n;
    for (const b of bytes) n = n * 256n + BigInt(b);
    let s = '';
    while (n > 0n) {
      s = B58[Number(n % 58n)] + s;
      n /= 58n;
    }
    return s || '1';
  }
  // The real on-chain instruction data (8-byte discriminator + u64 amount), as the SDK encodes it.
  function payTrafficData(amount: bigint): string {
    const bytes = rewardsSettlement.getPayTrafficInstructionDataEncoder().encode({ amount });
    return b58(new Uint8Array(bytes));
  }

  it('decodes the payer + amount from a genuine pay_traffic instruction', () => {
    const keys = [PAYER, 'distributor', 'cfg', 'mint', 'ata', 'vault', 'treasury', PROGRAM];
    const ix = {
      programIdIndex: 7,
      accounts: [0, 1, 2, 3, 4, 5, 6],
      data: payTrafficData(1_234_567n),
    };
    const r = decodePayTraffic(keys, [ix], PAYER);
    expect(r.amount).toBe(1_234_567n);
    expect(String(r.payer)).toBe(PAYER);
  });

  it('rejects a payment whose signer is not the expected wallet (no spoofing another wallet)', () => {
    const keys = [PAYER, 'x', 'y', 'z', 'a', 'b', 'c', PROGRAM];
    const ix = { programIdIndex: 7, accounts: [0, 1, 2, 3, 4, 5, 6], data: payTrafficData(10n) };
    expect(() => decodePayTraffic(keys, [ix], OTHER)).toThrow(/not signed by this wallet/);
  });

  it('rejects a tx that contains no settlement instruction', () => {
    const keys = [PAYER, '11111111111111111111111111111111'];
    const ix = { programIdIndex: 1, accounts: [0], data: payTrafficData(10n) };
    expect(() => decodePayTraffic(keys, [ix], PAYER)).toThrow(/no pay_traffic/);
  });
});

describe('usage parsing', () => {
  it('sums uplink + downlink per user email', () => {
    const raw = JSON.stringify({
      stat: [
        { name: 'user>>>walletA>>>traffic>>>uplink', value: '100' },
        { name: 'user>>>walletA>>>traffic>>>downlink', value: '250' },
        { name: 'user>>>walletB>>>traffic>>>uplink', value: '5' },
      ],
    });
    const m = parseUsage(raw);
    expect(m.get('walletA')).toBe(350n);
    expect(m.get('walletB')).toBe(5n);
  });
  it('handles empty / missing stat array', () => {
    expect(parseUsage('{}').size).toBe(0);
    expect(parseUsage('').size).toBe(0);
  });
});
