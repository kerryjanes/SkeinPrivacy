import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

type Conn = 'off' | 'connecting' | 'on' | 'disconnecting';
type Mode = 'proxy' | 'tun';

interface Status {
  state: Conn;
  mode: Mode | null;
  inbound: string | null;
  hops: number;
  exitMode: string;
  bytesUp: number;
  bytesDown: number;
}

interface Wallet {
  address: string | null;
}

const fmtBytes = (n: number): string => {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
};

export function App() {
  const [status, setStatus] = useState<Status>({
    state: 'off',
    mode: null,
    inbound: null,
    hops: 3,
    exitMode: '—',
    bytesUp: 0,
    bytesDown: 0,
  });
  const [mode, setMode] = useState<Mode>('proxy');
  const [wallet, setWallet] = useState<Wallet>({ address: null });
  const [keyInput, setKeyInput] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const logRef = useRef<HTMLDivElement>(null);

  const append = useCallback((line: string) => {
    setLog((l) => [...l.slice(-80), `${new Date().toLocaleTimeString()}  ${line}`]);
  }, []);

  // Poll status while connected so the byte counters stay live.
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

  // Stream the sing-box core's own logs + exit notice into the log panel.
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

  const toggle = async () => {
    setErr(null);
    if (status.state === 'off') {
      setStatus((s) => ({ ...s, state: 'connecting' }));
      append(`connecting (${mode}) — building onion circuit + starting core…`);
      try {
        const s = await invoke<Status>('connect', { mode });
        setStatus(s);
        append(
          `connected · ${s.mode === 'tun' ? 'system tunnel' : `proxy ${s.inbound}`} · ${s.hops} hops · exit ${s.exitMode}`,
        );
      } catch (e) {
        setErr(String(e));
        setStatus((s) => ({ ...s, state: 'off' }));
        append(`connect failed: ${e}`);
      }
    } else if (status.state === 'on') {
      setStatus((s) => ({ ...s, state: 'disconnecting' }));
      append('disconnecting…');
      try {
        setStatus(await invoke<Status>('disconnect'));
        append('disconnected');
      } catch (e) {
        setErr(String(e));
      }
    }
  };

  const importWallet = async () => {
    setErr(null);
    try {
      const w = await invoke<Wallet>('import_wallet', { keypair: keyInput.trim() });
      setWallet(w);
      setKeyInput('');
      append(`wallet imported: ${w.address}`);
    } catch (e) {
      setErr(String(e));
    }
  };

  const orbClass =
    status.state === 'on' ? 'orb on' : status.state === 'off' ? 'orb off' : 'orb busy';
  const orbLabel =
    status.state === 'on'
      ? 'CONNECTED'
      : status.state === 'connecting'
        ? 'CONNECTING'
        : status.state === 'disconnecting'
          ? 'STOPPING'
          : 'CONNECT';
  const idle = status.state === 'off';

  return (
    <div className="app">
      <div className="brand">
        <span className="dot" />
        <h1>Weft VPN</h1>
        <small>{status.state === 'on' ? '● secured' : '○ off'}</small>
      </div>

      <div className="power">
        <div className={orbClass} onClick={toggle}>
          {orbLabel}
        </div>
        <div className="state">
          {status.state === 'on' ? (
            <>
              Traffic is onion-routed through <b>{status.hops}</b> Weft nodes
            </>
          ) : (
            'Tap to route your traffic through the Weft network'
          )}
        </div>
      </div>

      {/* Capture mode: a local proxy (no admin) or a system-wide tunnel (admin). */}
      <div className="modes">
        <button
          className={`mode ${mode === 'proxy' ? 'sel' : ''}`}
          onClick={() => idle && setMode('proxy')}
          disabled={!idle}
        >
          Proxy
          <small>local SOCKS/HTTP · no admin</small>
        </button>
        <button
          className={`mode ${mode === 'tun' ? 'sel' : ''}`}
          onClick={() => idle && setMode('tun')}
          disabled={!idle}
        >
          System tunnel
          <small>all traffic · needs admin</small>
        </button>
      </div>

      {err && <div className="err">{err}</div>}

      <div className="panel">
        <h2>Connection</h2>
        <div className="row">
          <span>Status</span>
          <b>{status.state}</b>
        </div>
        <div className="row">
          <span>Mode</span>
          <b>{status.mode ?? mode}</b>
        </div>
        <div className="row">
          <span>Inbound</span>
          <b className="addr">{status.inbound ?? '—'}</b>
        </div>
        <div className="row">
          <span>Exit egress</span>
          <b>{status.exitMode}</b>
        </div>
        <div className="row">
          <span>↑ sent / ↓ received</span>
          <b>
            {fmtBytes(status.bytesUp)} / {fmtBytes(status.bytesDown)}
          </b>
        </div>
        {status.state === 'on' && (
          <div className="hops">
            <span className="hop">you</span>
            {Array.from({ length: status.hops }).map((_, i) => (
              <span key={i}>
                <span className="arrow"> → </span>
                <span className="hop">{i === status.hops - 1 ? 'exit' : `relay ${i + 1}`}</span>
              </span>
            ))}
            <span className="arrow"> → </span>
            <span className="hop">internet</span>
          </div>
        )}
        {status.state === 'on' && status.mode === 'proxy' && status.inbound && (
          <div className="state" style={{ marginTop: 8 }}>
            Point your browser/OS proxy at <b>{status.inbound}</b> (SOCKS5 or HTTP).
          </div>
        )}
        {status.state === 'on' && status.mode === 'tun' && (
          <div className="state" style={{ marginTop: 8 }}>
            All system traffic is captured via the Weft tunnel interface.
          </div>
        )}
      </div>

      <div className="panel">
        <h2>Wallet</h2>
        <div className="row">
          <span>Address</span>
          <b className="addr">{wallet.address ?? 'not connected'}</b>
        </div>
        {!wallet.address && (
          <>
            <input
              className="field"
              placeholder="paste keypair JSON ([1,2,…]) to import"
              value={keyInput}
              onChange={(e) => setKeyInput(e.target.value)}
            />
            <button className="act ghost" onClick={importWallet}>
              Import wallet
            </button>
          </>
        )}
      </div>

      <div className="panel">
        <h2>Log</h2>
        <div className="log" ref={logRef}>
          {log.length ? log.join('\n') : 'idle.'}
        </div>
      </div>
    </div>
  );
}
