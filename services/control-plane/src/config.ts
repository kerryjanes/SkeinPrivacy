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
  storePath: string; // JSON user store
  port: number; // HTTP API the clients/website talk to
  pollMs: number; // metering interval

  // --- chain ---
  rpcUrl: string;
  wsUrl: string;
  weftMint: string;

  // --- devnet faucet (absent on mainnet) ---
  faucetKeypairPath: string; // empty = faucet disabled
  faucetAmount: bigint; // base units minted per drip

  // --- relay liveness (this box is also the rendezvous relay) ---
  frpsApi: string; // frps admin API base (empty = no tunnels, only direct endpoints)
  frpsUser: string;
  frpsPass: string;
}

function env(key: string, fallback: string): string {
  const v = process.env[key];
  return v === undefined || v === '' ? fallback : v;
}

export function loadConfig(): NodeConfig {
  return {
    host: env('WEFT_HOST', 'vpn.weftnetwork.net'),
    realityPublicKey: env('WEFT_REALITY_PBK', 'ag8kOu7UmNIFxKVdjiasZMc2Vj9OtST3PwcFqh1CmWw'),
    realityPrivateKey: env('WEFT_REALITY_PRIV', 'YDedl8FY3Y9XFssAk49TLk-Mq6zmwYiDKdwRmaVSIDE'),
    shortId: env('WEFT_SID', '4ce4af1305de920f'),
    sni: env('WEFT_SNI', 'ya.ru'),
    hop1Port: Number(env('WEFT_HOP1_PORT', '443')),
    hopnPort: Number(env('WEFT_HOPN_PORT', '8443')),
    publicHop1Port: Number(env('WEFT_PUBLIC_HOP1_PORT', env('WEFT_HOP1_PORT', '443'))),
    publicHopnPort: Number(env('WEFT_PUBLIC_HOPN_PORT', env('WEFT_HOPN_PORT', '8443'))),
    founderUuid: env('WEFT_FOUNDER_UUID', 'b5ced6eb-0cba-4001-9679-65f8ba69e74b'),

    xrayConfigPath: env('WEFT_XRAY_CONFIG', '/usr/local/etc/xray/config.json'),
    xrayApi: env('WEFT_XRAY_API', '127.0.0.1:10085'),
    xrayBin: env('WEFT_XRAY_BIN', '/usr/local/bin/xray'),
    reloadCmd: env('WEFT_XRAY_RELOAD', 'systemctl restart xray'),
    storePath: env('WEFT_STORE', '/var/lib/weft/users.json'),
    port: Number(env('WEFT_PORT', '8088')),
    pollMs: Number(env('WEFT_POLL_MS', '10000')),

    rpcUrl: env('WEFT_RPC', 'https://api.devnet.solana.com'),
    wsUrl: env(
      'WEFT_WS',
      env('WEFT_RPC', 'https://api.devnet.solana.com').replace(/^http/, 'ws'),
    ),
    weftMint: env('WEFT_MINT', '8AYQEuGHXXwndyfLCY4quyNoMxTPxzh2CJv6DwpDaC8i'),
    faucetKeypairPath: env('WEFT_FAUCET_KEYPAIR', ''),
    faucetAmount: BigInt(env('WEFT_FAUCET_AMOUNT', '1000000')), // 0.001 $WEFT → ~10 MB quota
    frpsApi: env('WEFT_FRPS_API', ''),
    frpsUser: env('WEFT_FRPS_USER', ''),
    frpsPass: env('WEFT_FRPS_PASS', ''),
  };
}
