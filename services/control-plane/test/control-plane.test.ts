import { describe, expect, it } from 'vitest';
import { mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { rewardsSettlement } from '@weft/sdk';
import { loadConfig } from '../src/config.js';
import { math } from '@weft/sdk';
import { costBaseUnits, decodePayTraffic, quotaBytes } from '../src/chain.js';
import { multiHopLink, oneHopLink } from '../src/links.js';
import { parseUsage, renderConfig } from '../src/xray.js';
import { Store, type User } from '../src/store.js';

const cfg = loadConfig();
const user = (over: Partial<User> = {}): User => ({
  wallet: 'Wa11et1111111111111111111111111111111111111',
  uuid: '11111111-2222-3333-4444-555555555555',
  email: 'Wa11et1111111111111111111111111111111111111',
  unsettledBytes: '0',
  servedBytesLifetime: '0',
  balanceBaseUnits: '0',
  quotaBytes: '0',
  active: true,
  createdAt: 0,
  ...over,
});

function withEnv(keys: Record<string, string | undefined>, fn: () => void): void {
  const saved = new Map<string, string | undefined>();
  for (const key of Object.keys(keys)) {
    saved.set(key, process.env[key]);
    if (keys[key] === undefined) delete process.env[key];
    else process.env[key] = keys[key];
  }
  try {
    fn();
  } finally {
    for (const [key, value] of saved) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
}

describe('mainnet config safety', () => {
  it('requires explicit RPC, mint, Reality keys and founder UUID on mainnet', () => {
    withEnv(
      {
        WEFT_CLUSTER: 'mainnet-beta',
        WEFT_RPC: undefined,
        WEFT_MINT: undefined,
        WEFT_REALITY_PBK: undefined,
        WEFT_REALITY_PRIV: undefined,
        WEFT_SID: undefined,
        WEFT_FOUNDER_UUID: undefined,
      },
      () => expect(() => loadConfig()).toThrow(/WEFT_RPC/),
    );
  });

  it('rejects faucet configuration on mainnet', () => {
    withEnv(
      {
        WEFT_CLUSTER: 'mainnet-beta',
        WEFT_RPC: 'https://api.mainnet-beta.solana.com',
        WEFT_MINT: 'So11111111111111111111111111111111111111112',
        WEFT_REALITY_PBK: 'pbk',
        WEFT_REALITY_PRIV: 'priv',
        WEFT_SID: 'abcd',
        WEFT_FOUNDER_UUID: '11111111-2222-3333-4444-555555555555',
        WEFT_FAUCET_KEYPAIR: '/tmp/faucet.json',
      },
      () => expect(() => loadConfig()).toThrow(/FAUCET/),
    );
  });
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

describe('node earnings (earn for usage)', () => {
  it('a node earns 0.1 $WEFT per GB it serves (baseline multipliers)', () => {
    // 1 GB served at baseline reputation (1.0×), no geo/stake bonus → 0.1 WEFT
    expect(math.trafficReward(1_000_000_000n, 10_000n, 0n, 0n)).toBe(100_000_000n); // 0.1 WEFT
  });
  it('earnings scale with served traffic', () => {
    const oneGb = math.trafficReward(1_000_000_000n, 10_000n, 0n, 0n);
    const tenGb = math.trafficReward(10_000_000_000n, 10_000n, 0n, 0n);
    expect(tenGb).toBe(oneGb * 10n);
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
  it('can pin 1-hop egress to a physical interface IP to bypass a host VPN', () => {
    const c = renderConfig({ ...cfg, xraySendThrough: '192.168.0.103' }, []) as any;
    const direct = c.outbounds.find((o: any) => o.tag === 'direct');
    expect(direct.sendThrough).toBe('192.168.0.103');
    expect(direct.settings.domainStrategy).toBe('UseIPv4');
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

describe('payment replay ledger', () => {
  it('persists processed payment signatures and rejects reuse after restart', () => {
    const dir = mkdtempSync(join(tmpdir(), 'weft-cp-'));
    try {
      const path = join(dir, 'users.json');
      const first = new Store(path);
      first.beginPayment('tx-sig-1', user().wallet, 1000);
      first.completePayment('tx-sig-1', user().wallet, 123n, 1001);

      const afterRestart = new Store(path);
      expect(afterRestart.payment('tx-sig-1')?.status).toBe('processed');
      expect(() => afterRestart.beginPayment('tx-sig-1', user().wallet, 1002)).toThrow(
        /already submitted/,
      );
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('removes pending reservations when verification fails', () => {
    const dir = mkdtempSync(join(tmpdir(), 'weft-cp-'));
    try {
      const path = join(dir, 'users.json');
      const store = new Store(path);
      store.beginPayment('tx-sig-2', user().wallet, 1000);
      store.forgetPendingPayment('tx-sig-2', user().wallet);
      expect(store.payment('tx-sig-2')).toBeUndefined();
      expect(() => store.beginPayment('tx-sig-2', user().wallet, 1001)).not.toThrow();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
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
