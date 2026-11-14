// Control page card: shows the engine (rhcsd) and lets you stop it. Starting it again
// needs the platform: on Windows the `pulse:` link registered by the installer does it.

import { useCallback, useEffect, useState } from 'react';
import { fmtDuration } from '../lib/format';
import { detectOs, getEngine, insideApp, startEngineLink, stopEngine, type EngineInfo, type EngineResult } from '../lib/engine';
import { confirmAction, toast, useStore } from '../lib/store';
import { Badge, KV } from './ui';

const MODE: Record<EngineInfo['mode'], string> = { app: 'Android app', windows: 'Windows', termux: 'Termux', other: 'Other' };

export function EngineCard() {
  const conn = useStore((s) => s.conn);
  const [res, setRes] = useState<EngineResult | null>(null);
  const [stopped, setStopped] = useState(false);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    const r = await getEngine();
    setRes(r);
    if (r.kind === 'ok') setStopped(false);
  }, []);

  // Look once each time the page connects; no polling loop.
  useEffect(() => {
    if (conn === 'live') void refresh();
  }, [conn, refresh]);

  const info = res?.kind === 'ok' ? res.info : null;
  const os = detectOs();
  const inApp = insideApp();

  const stop = async () => {
    const ok = await confirmAction({
      title: 'Stop the engine?',
      body: 'PULSE will stop reading your router and devices and this page will disconnect. Nothing is deleted. You can start the engine again from here.',
      confirmLabel: 'Stop engine',
      danger: true,
    });
    if (!ok) return;
    setBusy(true);
    const r = await stopEngine();
    if (r.outcome === 'stopped') {
      setStopped(true);
      toast('ok', 'Engine stopped', r.message || 'The engine is shutting down.');
    } else if (r.outcome === 'auth') {
      toast('error', 'Could not stop the engine', 'It needs the access token. Enter the token in the settings below, then try again.', 8000);
    } else if (r.outcome === 'old') {
      toast('warn', 'Engine too old', 'This engine cannot be stopped from the page. Update it by running the newest installer.', 8000);
    } else if (r.outcome === 'refused') {
      toast('error', 'The engine did not stop', r.message || 'It refused the request.', 8000);
    } else {
      // The connection closed: usually the engine already exited. Check.
      const again = await getEngine();
      if (again.kind === 'down') {
        setStopped(true);
        toast('ok', 'Engine stopped', 'It is no longer answering.');
      } else toast('error', 'Could not stop the engine', 'It is still answering. Try again.', 8000);
    }
    setBusy(false);
  };

  let body;
  if (stopped) {
    body = (
      <>
        <p className="muted">The engine is stopped. PULSE shows "connection interrupted" until it runs again.</p>
        {os === 'windows' && !inApp ? (
          <>
            <button type="button" className="btn btn--accent" onClick={startEngineLink}>
              Start engine
            </button>
            <p className="muted engine-card__small">Your browser may ask to open "PULSE". Allow it once. This page reconnects by itself.</p>
          </>
        ) : inApp ? (
          <p className="muted">Use the Start button in the app.</p>
        ) : (
          <p className="muted">Start rhcsd again where it is installed (for example, run it again in Termux).</p>
        )}
      </>
    );
  } else if (res?.kind === 'old') {
    body = <p className="muted">This engine is older than the engine controls, so it cannot be stopped from here. Run the newest installer to update it.</p>;
  } else if (res?.kind === 'auth') {
    body = <p className="muted">The engine needs the access token before it shows this. Enter it in the settings below.</p>;
  } else if (!info) {
    body = <p className="muted">{res ? 'The engine did not answer.' : 'Checking the engine…'}</p>;
  } else {
    body = (
      <>
        <dl className="kvs">
          <KV k="Status" v={<Badge tone={info.running ? 'live' : 'stale'}>{info.running ? 'Running' : 'Stopped'}</Badge>} />
          <KV k="Runs on" v={MODE[info.mode] ?? info.mode} />
          <KV k="Version" v={info.version || '—'} />
          <KV k="Up for" v={fmtDuration(info.uptime_s)} />
        </dl>
        <div className="actions">
          <button type="button" className="btn btn--danger" onClick={() => void stop()} disabled={busy || !info.can_stop || !info.running}>
            {busy ? 'Stopping…' : 'Stop engine'}
          </button>
        </div>
        {!info.can_stop ? <p className="muted engine-card__small">This engine cannot be stopped from the page.</p> : null}
      </>
    );
  }
  return <div className="engine-card">{body}</div>;
}
