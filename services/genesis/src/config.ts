// Genesis configuration: tokenomics amounts (mirroring `weft-primitives`,
// asserted to sum to the total supply) and environment/cluster resolution.

export const DECIMALS = 9;
export const ONE_WEFT = 1_000_000_000n; // 10^DECIMALS
export const TOTAL_SUPPLY = 1_000_000_000n * ONE_WEFT; // 1e18 base units

// Allocation buckets (base units) — SPEC.md token distribution table.
export const AMOUNTS = {
  nodeEmissions: 400_000_000n * ONE_WEFT, // 40%
  idoTge: 37_500_000n * ONE_WEFT, //  25% of the 15% IDO bucket (liquid at TGE)
  idoLinear: 112_500_000n * ONE_WEFT, // 75% of the 15% IDO bucket (vested)
  team: 150_000_000n * ONE_WEFT, // 15%
  ecosystem: 150_000_000n * ONE_WEFT, // 15%
  treasury: 100_000_000n * ONE_WEFT, // 10%
  marketing: 50_000_000n * ONE_WEFT, //  5%
} as const;

export const MONTH_SECONDS = 2_592_000n; // 30 days

// Vesting schedules (schedule_id, amount, cliff, duration, revocable).
export const SCHEDULES = [
  {
    key: 'team',
    scheduleId: 0n,
    amount: AMOUNTS.team,
    cliff: 12n * MONTH_SECONDS,
    duration: 36n * MONTH_SECONDS,
    revocable: true,
  },
  {
    key: 'idoLinear',
    scheduleId: 1n,
    amount: AMOUNTS.idoLinear,
    cliff: 0n,
    duration: 12n * MONTH_SECONDS,
    revocable: false,
  },
  {
    key: 'ecosystem',
    scheduleId: 2n,
    amount: AMOUNTS.ecosystem,
    cliff: 0n,
    duration: 36n * MONTH_SECONDS,
    revocable: false,
  },
  {
    key: 'marketing',
    scheduleId: 3n,
    amount: AMOUNTS.marketing,
    cliff: 0n,
    duration: 24n * MONTH_SECONDS,
    revocable: false,
  },
] as const;

// Invariant: every base unit is accounted for exactly once.
export function assertConservation(): void {
  const sum =
    AMOUNTS.nodeEmissions +
    AMOUNTS.idoTge +
    AMOUNTS.idoLinear +
    AMOUNTS.team +
    AMOUNTS.ecosystem +
    AMOUNTS.treasury +
    AMOUNTS.marketing;
  if (sum !== TOTAL_SUPPLY) {
    throw new Error(`allocation sum ${sum} != total supply ${TOTAL_SUPPLY}`);
  }
}

export interface GenesisEnv {
  cluster: string;
  rpcUrl: string;
  wsUrl: string;
  keypairPath: string;
  /** TGE timestamp (i64 seconds) all schedules vest from. */
  tgeTimestamp: bigint;
}

export function deriveWsUrl(rpcUrl: string): string {
  const u = new URL(rpcUrl);
  const secure = u.protocol === 'https:';
  u.protocol = secure ? 'wss:' : 'ws:';
  // Local validator serves websockets on rpcPort + 1 (8899 -> 8900).
  if (u.port === '8899') u.port = '8900';
  return u.toString().replace(/\/$/, '');
}

export function loadEnv(overrides: Partial<GenesisEnv> = {}): GenesisEnv {
  const cluster = overrides.cluster ?? process.env.WEFT_CLUSTER ?? 'localnet';
  const rpcUrl =
    overrides.rpcUrl ??
    process.env.WEFT_RPC_URL ??
    (cluster === 'devnet' ? 'https://api.devnet.solana.com' : 'http://127.0.0.1:8899');
  const keypairPath =
    overrides.keypairPath ??
    process.env.WEFT_KEYPAIR ??
    `${process.env.HOME}/.config/solana/id.json`;
  return {
    cluster,
    rpcUrl,
    wsUrl: overrides.wsUrl ?? deriveWsUrl(rpcUrl),
    keypairPath,
    tgeTimestamp: overrides.tgeTimestamp ?? BigInt(process.env.WEFT_TGE_TS ?? '1700000000'),
  };
}
