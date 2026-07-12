// Node-operator configuration for the Weft control plane. Everything is env-driven so a node
// operator just exports a handful of vars (the deploy script writes them into the systemd unit);
// the defaults point at the launch node so the service runs out of the box on it.

export interface NodeConfig {
  // --- the Reality node this control plane fronts ---
  host: string; // public hostname clients dial (= relay host for a home node behind NAT)
  realityPublicKey: string; // pbk in the vless:// link (the x25519 PUBLIC key)
  realityPrivateKey: string; // server private key (written into the rendered xray config)
  shortId: string; // sid
  sni: string; // Reality serverName / dest (a real TLS1.3 site; ya.ru for RU)
  geo: number;
  // Xray LISTEN ports (local on this box). For a VPS node these equal the public ports; for a
  // home node behind NAT, frpc maps the relay's public ports onto these local ones.
  hop1Port: number; // direct VLESS+Reality (vision flow)
  hopnPort: number; // VLESS+Reality routed through Tor at the node (no flow)
  // PUBLIC endpoint advertised in the vless:// links. Defaults to host/hop1Port/hopnPort (VPS);
  // a home node sets these to the relay host + its assigned relay ports.
  publicHop1Port: number;
  publicHopnPort: number;
  founderUuid: string; // the always-on, unmetered founder link (kept working through migrations)

  // --- node plumbing ---
  xrayConfigPath: string; // where the rendered config.json is written
  xrayApi: string; // gRPC api endpoint (statsquery)
  xrayBin: string;
  reloadCmd: string; // how to make xray pick up a new config (restart)
  xraySendThrough: string; // optional local source IP for 1-hop egress (bypass host VPN)
  storePath: string; // JSON user store
  port: number; // HTTP API the clients/website talk to
  pollMs: number; // metering interval

  // --- chain ---
  cluster: string;
  rpcUrl: string;
  wsUrl: string;
  weftMint: string;

  // --- delegated settlement: the poster key auto-bills users' escrows for metered usage ---
  settleKeypairPath: string; // poster_authority keypair; empty = no auto-settle (manual /settle only)
  settleIntervalMs: number; // how often the control plane auto-bills accrued usage
  settleMinBytes: bigint; // skip the settle tx until a user has accrued at least this much unsettled

  // --- devnet faucet (absent on mainnet) ---
  faucetKeypairPath: string; // empty = faucet disabled
  faucetAmount: bigint; // base units transferred per drip
  faucetSolLamports: bigint; // devnet SOL transferred per fee top-up
  faucetCooldownMs: number;

  // --- relay liveness (this box is also the rendezvous relay) ---
  frpsApi: string; // frps admin API base (empty = no tunnels, only direct endpoints)
  frpsUser: string;
  frpsPass: string;
  relayToken: string;
  relayProfilePath: string;
  relayProfileUrl: string;
  exitProfileTtlMs: number;
}

function env(key: string, fallback: string): string {
  const v = process.env[key];
  return v === undefined || v === '' ? fallback : v;
}

function mainnetCluster(cluster: string): boolean {
  return cluster.startsWith('mainnet');
}

function requiredForMainnet(cluster: string, key: string, fallback: string): string {
  const v = process.env[key];
  if (mainnetCluster(cluster) && (v === undefined || v === '')) {
    throw new Error(`${key} must be set explicitly for ${cluster}`);
  }
  return v === undefined || v === '' ? fallback : v;
}

export function loadConfig(): NodeConfig {
  const cluster = env('WEFT_CLUSTER', 'devnet');
  const mainnet = mainnetCluster(cluster);
  const rpcUrl = requiredForMainnet(cluster, 'WEFT_RPC', 'https://api.devnet.solana.com');
  const faucetKeypairPath = env('WEFT_FAUCET_KEYPAIR', '');
  if (mainnet && faucetKeypairPath) {
    throw new Error('WEFT_FAUCET_KEYPAIR must be empty on mainnet');
  }
  return {
    host: env('WEFT_HOST', 'vpn.weftnetwork.net'),
    // Reality identity + founder credential are SECRETS — never a real committed default. Each
    // deployment injects them from its own uncommitted secrets file (see scripts/mainnet-cutover.sh);
    // the devnet fallbacks below are obvious non-working placeholders so a leaked repo grants nothing.
    realityPublicKey: requiredForMainnet(cluster, 'WEFT_REALITY_PBK', 'SET_WEFT_REALITY_PBK'),
    realityPrivateKey: requiredForMainnet(cluster, 'WEFT_REALITY_PRIV', 'SET_WEFT_REALITY_PRIV'),
    shortId: requiredForMainnet(cluster, 'WEFT_SID', '0000000000000000'),
    sni: env('WEFT_SNI', 'ya.ru'),
    geo: Number(env('WEFT_GEO', '0')),
    hop1Port: Number(env('WEFT_HOP1_PORT', '443')),
    hopnPort: Number(env('WEFT_HOPN_PORT', '8443')),
    publicHop1Port: Number(env('WEFT_PUBLIC_HOP1_PORT', env('WEFT_HOP1_PORT', '443'))),
    publicHopnPort: Number(env('WEFT_PUBLIC_HOPN_PORT', env('WEFT_HOPN_PORT', '8443'))),
    founderUuid: requiredForMainnet(
      cluster,
      'WEFT_FOUNDER_UUID',
      '00000000-0000-4000-8000-000000000000',
    ),

    xrayConfigPath: env('WEFT_XRAY_CONFIG', '/usr/local/etc/xray/config.json'),
    xrayApi: env('WEFT_XRAY_API', '127.0.0.1:10085'),
    xrayBin: env('WEFT_XRAY_BIN', '/usr/local/bin/xray'),
    reloadCmd: env('WEFT_XRAY_RELOAD', 'systemctl restart xray'),
    xraySendThrough: env('WEFT_XRAY_SEND_THROUGH', ''),
    storePath: env('WEFT_STORE', '/var/lib/weft/users.json'),
    port: Number(env('WEFT_PORT', '8088')),
    pollMs: Number(env('WEFT_POLL_MS', '10000')),

    cluster,
    rpcUrl,
    wsUrl: env('WEFT_WS', rpcUrl.replace(/^http/, 'ws')),
    weftMint: requiredForMainnet(
      cluster,
      'WEFT_MINT',
      '8AYQEuGHXXwndyfLCY4quyNoMxTPxzh2CJv6DwpDaC8i',
    ),
    settleKeypairPath: env('WEFT_SETTLE_KEYPAIR', ''),
    settleIntervalMs: Number(env('WEFT_SETTLE_MS', '60000')),
    settleMinBytes: BigInt(env('WEFT_SETTLE_MIN_BYTES', '1048576')), // ~1 MB → avoid dust txs
    faucetKeypairPath,
    faucetAmount: BigInt(env('WEFT_FAUCET_AMOUNT', '1000000000000')), // 1000 $WEFT → ~1 GB quota
    faucetSolLamports: BigInt(env('WEFT_FAUCET_SOL_LAMPORTS', '50000000')), // 0.05 SOL for fees
    faucetCooldownMs: Number(env('WEFT_FAUCET_COOLDOWN_MS', '60000')),
    frpsApi: env('WEFT_FRPS_API', ''),
    frpsUser: env('WEFT_FRPS_USER', ''),
    frpsPass: env('WEFT_FRPS_PASS', ''),
    relayToken: requiredForMainnet(cluster, 'WEFT_RELAY_TOKEN', 'SET_WEFT_RELAY_TOKEN'),
    relayProfilePath: env('WEFT_RELAY_PROFILE_PATH', '/var/lib/weft/exit-profiles.json'),
    relayProfileUrl: env('WEFT_RELAY_PROFILE_URL', ''),
    exitProfileTtlMs: Number(env('WEFT_EXIT_PROFILE_TTL_MS', '120000')),
  };
}
