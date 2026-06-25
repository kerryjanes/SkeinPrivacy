import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

type Conn = 'off' | 'connecting' | 'on' | 'disconnecting';
type Mode = '1hop' | 'multihop';

interface Status {
  state: Conn;
  mode: Mode | null;
  inbound: string | null;
}

interface Wallet {
  address: string | null;
}

/** The control-plane response: this wallet's $WEFT-gated traffic budget. */
interface Account {
  wallet: string;
  active: boolean;
  balanceWeft: string;
  quotaBytes: string;
  unsettledBytes: string;
  owedWeft: string;
  remainingBytes: string;
  links: { oneHop: string; multiHop: string };
}

function formatBytes(n: string | number): string {
  const b = Number(n);
  if (b < 1024) return `${b} B`;
  if (b < 1_000_000) return `${(b / 1024).toFixed(1)} KB`;
  if (b < 1_000_000_000) return `${(b / 1_000_000).toFixed(2)} MB`;
  return `${(b / 1_000_000_000).toFixed(2)} GB`;
}

/** The brand mark — the site's reticle/bracket motif. */
const Mark = ({ size = 22 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" style={{ display: 'block', flex: 'none' }}>
    <g stroke="#3FE3A4" strokeWidth="1.6" fill="none">
      <path d="M3 7V3h4" />
      <path d="M17 3h4v4" />
      <path d="M21 17v4h-4" />
      <path d="M7 21H3v-4" />
    </g>
    <rect x="9.5" y="9.5" width="5" height="5" fill="#3FE3A4" />
  </svg>
);

const GLYPHS = 'ABCDEFGHJKLMNPQRSTUVWXYZ0123456789#%&/<>=+*·';

/** Decrypt-scramble a string whenever it changes (the site's signature effect). */
function useScrambled(text: string): string {
  const [out, setOut] = useState(text);
  useEffect(() => {
    const n = text.length;
    let t = 0;
    const speed = Math.max(1.4, n / 12);
    const iv = setInterval(() => {
      t += speed;
      let s = '';
      for (let i = 0; i < n; i++) {
        const ch = text[i];
        if (ch === ' ') {
          s += ch;
          continue;
        }
        s += i < t ? ch : GLYPHS[(Math.random() * GLYPHS.length) | 0];
      }
      setOut(s);
      if (t >= n) {
        clearInterval(iv);
        setOut(text);
      }
    }, 28);
    return () => clearInterval(iv);
  }, [text]);
  return out;
}

export function App() {
  const [status, setStatus] = useState<Status>({
    state: 'off',
    mode: null,
    inbound: null,
  });
  const [mode, setMode] = useState<Mode>('1hop');
  const [wallet, setWallet] = useState<Wallet>({ address: null });
  const [account, setAccount] = useState<Account | null>(null);
  const [keyInput, setKeyInput] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const logRef = useRef<HTMLDivElement>(null);
  const grainRef = useRef<HTMLCanvasElement>(null);

  const append = useCallback((line: string) => {
    const d = new Date();
    const ts = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`;
    setLog((l) => [...l.slice(-120), `[${ts}] ${line}`]);
  }, []);

  // Poll live status.
  useEffect(() => {
    const t = setInterval(async () => {
      try {
        setStatus(await invoke<Status>('status'));
      } catch {
        /* ignore */
      }
    }, 1000);
    return () => clearInterval(t);
  }, []);

  // Stream the sing-box core's logs into the terminal.
  useEffect(() => {
    const uns = [
      listen<string>('singbox-log', (e) => append(`core · ${e.payload}`)),
      listen<number | null>('singbox-exit', (e) =>
        append(`core exited (${e.payload ?? 'killed'})`),
      ),
    ];
    return () => {
      uns.forEach((u) => u.then((f) => f()));
    };
  }, [append]);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [log]);

  // Film-grain shimmer overlay (the site's texture, subtle).
  useEffect(() => {
    const c = grainRef.current;
    if (!c) return;
    const ctx = c.getContext('2d');
    if (!ctx) return;
    const resize = () => {
      c.width = innerWidth;
      c.height = innerHeight;
    };
    resize();
    addEventListener('resize', resize);
    const tile = document.createElement('canvas');
    tile.width = tile.height = 140;
    const t = tile.getContext('2d')!;
    let f = 0;
    let raf = 0;
    const draw = () => {
      const id = t.createImageData(140, 140);
      for (let i = 0; i < id.data.length; i += 4) {
        const v = Math.random() * 255;
        id.data[i] = v * 0.45;
        id.data[i + 1] = v;
        id.data[i + 2] = v * 0.75;
        id.data[i + 3] = 22;
      }
      t.putImageData(id, 0, 0);
      ctx.clearRect(0, 0, c.width, c.height);
      const p = ctx.createPattern(tile, 'repeat');
      if (p) {
        ctx.fillStyle = p;
        ctx.save();
        ctx.translate((Math.random() * 140) | 0, (Math.random() * 140) | 0);
        ctx.fillRect(-140, -140, c.width + 280, c.height + 280);
        ctx.restore();
      }
    };
    const loop = () => {
      if (f++ % 4 === 0) draw();
      raf = requestAnimationFrame(loop);
    };
    loop();
    return () => {
      cancelAnimationFrame(raf);
      removeEventListener('resize', resize);
    };
  }, []);

  const connect = async () => {
    setErr(null);
    setStatus((s) => ({ ...s, state: 'connecting' }));
    append(`starting core · mode=${mode}…`);
    try {
      const s = await invoke<Status>('connect', { mode });
      setStatus(s);
      append(
        `connected · ${s.mode}${s.mode === 'multihop' ? ' · via Tor' : ''} · proxy ${s.inbound}`,
      );
    } catch (e) {
      setErr(String(e));
      setStatus((s) => ({ ...s, state: 'off' }));
      append(`connect failed: ${e}`);
    }
  };

  const disconnect = async () => {
    setErr(null);
    setStatus((s) => ({ ...s, state: 'disconnecting' }));
    append('tearing down circuit…');
    try {
      setStatus(await invoke<Status>('disconnect'));
      append('disconnected · trail redacted');
    } catch (e) {
      setErr(String(e));
    }
  };

  const provision = useCallback(
    async (announce = false) => {
      try {
        const a = await invoke<Account>('provision');
        setAccount(a);
        if (announce)
          append(
            a.active
              ? `provisioned · budget ${formatBytes(a.quotaBytes)} (${a.balanceWeft} $WEFT)`
              : `out of $WEFT · top up to connect`,
          );
      } catch (e) {
        if (announce) setErr(String(e));
      }
    },
    [append],
  );

  const importWallet = async () => {
    setErr(null);
    try {
      const w = await invoke<Wallet>('import_wallet', { keypair: keyInput.trim() });
      setWallet(w);
      setKeyInput('');
      append(`identity loaded · ${w.address}`);
      await provision(true); // fetch the $WEFT-gated budget + personal link
    } catch (e) {
      setErr(String(e));
    }
  };

  // Keep the budget fresh (re-reads the on-chain $WEFT balance) while a wallet is loaded.
  useEffect(() => {
    if (!wallet.address) return;
    const t = setInterval(() => void provision(false), 15000);
    return () => clearInterval(t);
  }, [wallet.address, provision]);

  const on = status.state === 'on';
  const busy = status.state === 'connecting' || status.state === 'disconnecting';
  const idle = status.state === 'off';
  const canConnect = !!account && account.active; // gated by $WEFT budget

  const headline =
    status.state === 'on'
      ? 'SECURED'
      : status.state === 'connecting'
        ? 'ROUTING'
        : status.state === 'disconnecting'
          ? 'CLOSING'
          : 'EXPOSED';
  const scrambledHead = useScrambled(headline);
  const sub =
    status.state === 'on'
      ? `your traffic exits at a stranger · origin redacted`
      : status.state === 'connecting'
        ? 'wrapping you in two envelopes…'
        : status.state === 'disconnecting'
          ? 'erasing the path…'
          : 'no route · the network can see you';

  return (
    <>
      <canvas id="grain" ref={grainRef} />
      <div className="app">
        <div className="topbar">
          <Mark size={22} />
          <span className="name">WEFT</span>
          <span className="tag">PRIVACY</span>
          <span className={`chip ${on ? 'on' : ''}`}>{on ? '● secured' : '○ offline'}</span>
        </div>

        {/* power / connect */}
        <div className="power">
          <div className="eyebrow">
            <b>//</b> circuit
          </div>
          <div className={`state ${on ? 'on' : ''}`}>{scrambledHead}</div>
          <div className="substate">{sub}</div>
          <div className="tagline">ROUTE THROUGH STRANGERS · TRUST NO ONE · OWNED BY NO ONE</div>

          <div className="modes">
            <button
              className={`mode ${mode === '1hop' ? 'sel' : ''}`}
              onClick={() => idle && setMode('1hop')}
              disabled={!idle}
            >
              1-HOP
              <small>fast · direct exit</small>
            </button>
            <button
              className={`mode ${mode === 'multihop' ? 'sel' : ''}`}
              onClick={() => idle && setMode('multihop')}
              disabled={!idle}
            >
              MULTIHOP
              <small>max privacy · via Tor · slower</small>
            </button>
          </div>
          <div className="substate" style={{ marginTop: 6 }}>
            1-hop is fast with a direct exit · multihop routes through Tor for max privacy, slower
          </div>

          <button
            className={`connect ${busy ? 'busy' : on ? 'on' : 'off'}`}
            onClick={on ? disconnect : idle && canConnect ? connect : undefined}
            disabled={!on && !busy && !canConnect}
          >
            {status.state === 'connecting'
              ? '◌ NEGOTIATING…'
              : status.state === 'disconnecting'
                ? '◌ CLOSING…'
                : on
                  ? '■ DISCONNECT'
                  : !wallet.address
                    ? '▸ LOAD IDENTITY FIRST'
                    : !account
                      ? '◌ CHECKING BUDGET…'
                      : !account.active
                        ? '✕ OUT OF $WEFT — TOP UP'
                        : '▸ CONNECT'}
          </button>
          {account && !account.active && idle && (
            <div className="substate" style={{ marginTop: 8 }}>
              you've used {formatBytes(account.unsettledBytes)} of {formatBytes(account.quotaBytes)}{' '}
              · top up $WEFT in your wallet to reconnect
            </div>
          )}
        </div>

        {err && <div className="err">! {err}</div>}

        {/* readout */}
        <div className="panel">
          <div className="eyebrow">
            <b>//</b> link
          </div>
          <div className="bx">
            <div className="row">
              <span className="k">status</span>
              <span className={`v ${on ? 'acc' : ''}`}>{status.state}</span>
            </div>
            <div className="row">
              <span className="k">mode</span>
              <span className="v">{status.mode ?? mode}</span>
            </div>
            <div className="row">
              <span className="k">inbound</span>
              <span className="v">{status.inbound ?? '—'}</span>
            </div>
            {on && status.inbound && (
              <div className="substate" style={{ marginTop: 10 }}>
                point your browser / OS proxy at <b style={{ color: '#fff' }}>{status.inbound}</b>
              </div>
            )}
          </div>
        </div>

        {/* $WEFT budget */}
        {account && (
          <div className="panel">
            <div className="eyebrow">
              <b>//</b> budget · 0.1 $WEFT / GB
            </div>
            <div className="bx">
              <div className="row">
                <span className="k">balance</span>
                <span className="v acc">{account.balanceWeft} $WEFT</span>
              </div>
              <div className="row">
                <span className="k">used</span>
                <span className="v">
                  {formatBytes(account.unsettledBytes)} / {formatBytes(account.quotaBytes)}
                </span>
              </div>
              <div className="row">
                <span className="k">remaining</span>
                <span className={`v ${account.active ? 'acc' : ''}`}>
                  {formatBytes(account.remainingBytes)}
                </span>
              </div>
              {Number(account.owedWeft) > 0 && (
                <div className="row">
                  <span className="k">owed</span>
                  <span className="v">{account.owedWeft} $WEFT</span>
                </div>
              )}
              <div
                style={{
                  height: 4,
                  marginTop: 10,
                  background: '#1d2a24',
                  borderRadius: 2,
                  overflow: 'hidden',
                }}
              >
                <div
                  style={{
                    height: '100%',
                    width: `${Math.min(100, (Number(account.unsettledBytes) / Math.max(1, Number(account.quotaBytes))) * 100)}%`,
                    background: account.active ? '#3FE3A4' : '#e3633f',
                  }}
                />
              </div>
              <div className="substate" style={{ marginTop: 10 }}>
                {account.active
                  ? 'metered at the node · cut off when your $WEFT runs out'
                  : 'budget spent — top up $WEFT to reconnect'}
              </div>
            </div>
          </div>
        )}

        {/* identity / wallet */}
        <div className="panel">
          <div className="eyebrow">
            <b>//</b> identity
          </div>
          <div className="bx">
            <div className="row">
              <span className="k">user</span>
              <span className="v addr">{wallet.address ?? 'unknown'}</span>
            </div>
            {!wallet.address && (
              <>
                <input
                  className="field"
                  placeholder="paste keypair JSON ([1,2,…]) to bind identity"
                  value={keyInput}
                  onChange={(e) => setKeyInput(e.target.value)}
                />
                <button className="act" onClick={importWallet}>
                  ▸ LOAD IDENTITY
                </button>
              </>
            )}
          </div>
        </div>

        {/* session log */}
        <div className="panel">
          <div className="eyebrow">
            <b>//</b> session log
          </div>
          <div className="bx">
            <div className="log" ref={logRef}>
              <div className="hdr">// weft@mesh · live</div>
              {log.length ? (
                log.join('\n')
              ) : (
                <span>
                  idle. <span className="caret">▌</span>
                </span>
              )}
            </div>
          </div>
        </div>
      </div>
    </>
  );
}
