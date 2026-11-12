import { useEffect, useState, type ReactNode } from 'react';
import { Badge, ErrorExplain, Icon, KV, Panel, Switch } from '../components/ui';
import { explainError, type ErrorSource } from '../lib/explain';
import { fmtAgo, fmtDuration, isNum, str } from '../lib/format';
import { api, enterDemo, goLive, normalizeUrl, refreshSettings } from '../lib/rhcs';
import { confirmAction, toast, useStore } from '../lib/store';
import { useNow } from '../lib/ticker';
import type { RouterAction } from '../lib/types';
import { runScan } from './Network';

const NOT_YET = 'Not available yet: rhcsd answers 501 because this action changes router settings and has not been tested on this router.';

function SourceCard({ name, kind, online, lastOk, error, extra }: { name: string; kind: ErrorSource; online: boolean | undefined; lastOk: number | null | undefined; error: string | null | undefined; extra?: ReactNode }) {
  const now = useNow(2000);
  return (
    <div className={`source ${online ? 'is-ok' : 'is-down'}`}>
      <div className="source__head">
        <span className={`dot ${online ? 'dot--ok' : 'dot--off'}`} aria-hidden="true" />
        <span className="source__name">{name}</span>
        <Badge tone={online ? 'good' : 'poor'}>{online ? 'online' : online === false ? 'offline' : 'unknown'}</Badge>
      </div>
      <dl className="kvs">
        <KV k="Last OK" v={lastOk ? fmtAgo(lastOk, now) : '—'} />
        {extra}
      </dl>
      {error ? <ErrorExplain e={explainError(kind, error)} compact /> : null}
    </div>
  );
}

async function doRouterAction(action: RouterAction, label: string, body: string, value?: 'auto' | 'lte_only') {
  const ok = await confirmAction({ title: `${label}?`, body, confirmLabel: label });
  if (!ok) return;
  const r = await api.routerAction(action, value);
  if (r.ok) toast('ok', label, r.message);
  else if (r.status === 501) toast('warn', `${label}: not available yet`, r.message);
  else if (r.status !== 401) toast('error', `${label} failed`, r.message);
}

export function ControlPage() {
  const prefs = useStore((s) => s.prefs);
  const setPrefs = useStore((s) => s.setPrefs);
  const settings = useStore((s) => s.settings);
  const sources = useStore((s) => s.sources);
  const health = useStore((s) => s.health);
  const conn = useStore((s) => s.conn);
  const baseUrl = useStore((s) => s.baseUrl);
  const [interval, setIntervalV] = useState<number>(settings?.interval_s ?? 1);
  const [url, setUrl] = useState(prefs.baseUrl);
  const [token, setToken] = useState(prefs.token);

  useEffect(() => {
    if (isNum(settings?.interval_s)) setIntervalV(settings!.interval_s!);
  }, [settings?.interval_s]);
  useEffect(() => {
    if (conn === 'live') void refreshSettings();
  }, [conn]);

  const applyInterval = async () => {
    const ok = await confirmAction({
      title: 'Change refresh interval?',
      body: `rhcsd will poll the router every ${interval} s. Shorter intervals give smoother charts but put more load on the router's web UI.`,
      confirmLabel: 'Apply',
    });
    if (!ok) return;
    const r = await api.setInterval(interval);
    if (r.ok) {
      const v = r.data?.interval_s ?? interval;
      useStore.setState((s) => ({ settings: { ...s.settings, interval_s: v } }));
      toast('ok', 'Refresh interval updated', `Router is now polled every ${v} s.`);
    } else if (r.status !== 401) toast('error', 'Could not change interval', r.message);
  };

  const saveBackend = async () => {
    const next = normalizeUrl(url);
    const ok = await confirmAction({
      title: 'Save backend settings?',
      body: next ? `PULSE will reconnect to ${next}.` : 'PULSE will auto-detect the backend (same origin if served by rhcsd, otherwise http://localhost:7878).',
      confirmLabel: 'Save & reconnect',
    });
    if (!ok) return;
    setPrefs({ baseUrl: next, token: token.trim() });
    setUrl(next);
    toast('ok', 'Backend settings saved', next || 'auto-detect');
    if (conn !== 'demo') void goLive();
  };

  const toggleDemo = async (on: boolean) => {
    const ok = await confirmAction({
      title: on ? 'Switch to demo data?' : 'Leave demo mode?',
      body: on
        ? 'Live data is cleared and replaced by a simulator. A DEMO badge stays visible the whole time.'
        : 'Simulated data is discarded and PULSE connects to the real backend.',
      confirmLabel: on ? 'Use demo data' : 'Go live',
    });
    if (!ok) return;
    setPrefs({ demo: on });
    if (on) enterDemo('user');
    else void goLive();
    toast('info', on ? 'Demo mode on' : 'Demo mode off', on ? 'All values are simulated.' : 'Connecting to rhcsd…');
  };

  const r = sources?.router;
  const p = sources?.phone;
  const sc = sources?.scan;
  const isDemo = conn === 'demo';

  return (
    <div className="page page--control">
      <header className="pagehead">
        <div>
          <div className="pagehead__kicker">
            rhcsd {str(health?.version)} · up {fmtDuration(health?.uptime_s)} · {health?.auth_required ? 'token required' : 'local-only control'}
          </div>
          <h1 className="pagehead__title">Control</h1>
        </div>
      </header>

      <div className="control-grid">
        <Panel title="Router" kicker={`${str(r?.model)} · ${str(r?.host)}`} className="ctl-router">
          <div className="actions">
            <div className="action">
              <div>
                <div className="action__name">Re-login</div>
                <div className="action__desc muted small">Drop the router web session and log in again on the next reading.</div>
              </div>
              <button type="button" className="btn btn--accent" onClick={() => void doRouterAction('relogin', 'Re-login', 'rhcsd will reset its router session and log in again on the next poll.')}>
                Re-login
              </button>
            </div>
            {(
              [
                ['reconnect', 'Reconnect', 'Drop and re-establish the mobile data connection.'],
                ['disconnect', 'Disconnect', 'Disconnect mobile data.'],
              ] as [RouterAction, string, string][]
            ).map(([a, label, desc]) => (
              <div className="action is-disabled" key={a}>
                <div>
                  <div className="action__name">
                    {label} <Badge tone="neutral">not available yet</Badge>
                  </div>
                  <div className="action__desc muted small">{desc}</div>
                </div>
                <span className="tip" title={NOT_YET} tabIndex={0} aria-label={`${label}: ${NOT_YET}`}>
                  <button type="button" className="btn" disabled aria-disabled="true">
                    {label}
                  </button>
                </span>
              </div>
            ))}
            <div className="action is-disabled">
              <div>
                <div className="action__name">
                  Network mode <Badge tone="neutral">not available yet</Badge>
                </div>
                <div className="action__desc muted small">Auto or LTE only.</div>
              </div>
              <span className="tip" title={NOT_YET} tabIndex={0} aria-label={`Network mode: ${NOT_YET}`}>
                <span className="seg">
                  <button type="button" className="seg__btn" disabled>
                    Auto
                  </button>
                  <button type="button" className="seg__btn" disabled>
                    LTE only
                  </button>
                </span>
              </span>
            </div>
          </div>
        </Panel>

        <Panel title="Polling" kicker="rhcsd settings" className="ctl-poll">
          <label className="field field--wide">
            <span className="field__label">
              Router refresh interval <span className="num accent">{interval.toFixed(1)} s</span>
            </span>
            <input type="range" min={0.5} max={10} step={0.5} value={interval} onChange={(e) => setIntervalV(Number(e.target.value))} aria-valuetext={`${interval} seconds`} />
          </label>
          <div className="row">
            <span className="muted small">Current: <span className="num">{isNum(settings?.interval_s) ? `${settings!.interval_s} s` : '—'}</span> · LAN scan every <span className="num">{isNum(settings?.scan_interval_s) ? fmtDuration(settings!.scan_interval_s) : '—'}</span></span>
            <button type="button" className="btn btn--accent" onClick={() => void applyInterval()} disabled={interval === settings?.interval_s}>
              Apply
            </button>
          </div>
        </Panel>

        <Panel title="LAN scan" kicker={`networks: ${(sc?.networks ?? []).join(', ') || '—'}`} className="ctl-scan">
          <div className="row">
            <span className="muted small">
              {sc?.running ? 'Scan running…' : `Last run ${fmtAgo(sc?.last_run)}`}
            </span>
            <button type="button" className="btn btn--accent" onClick={() => void runScan()} disabled={!!sc?.running}>
              <Icon name="scan" size={18} className={sc?.running ? 'spin' : ''} /> {sc?.running ? 'Scanning…' : 'Scan now'}
            </button>
          </div>
        </Panel>

        <Panel title="Source health" kicker="from rhcsd status" className="ctl-health">
          <div className="sources">
            <SourceCard name="Router" kind="router" online={r?.online} lastOk={r?.last_ok} error={r?.error} extra={<KV k="Logged in" v={r?.logged_in ? 'yes' : r?.logged_in === false ? 'no' : '—'} />} />
            <SourceCard name="Phone" kind="phone" online={p?.online} lastOk={p?.last_ok} error={p?.error} />
            <SourceCard
              name="Scanner"
              kind="router"
              online={sc ? true : undefined}
              lastOk={sc?.last_run}
              error={null}
              extra={<KV k="State" v={sc?.running ? 'scanning' : 'idle'} />}
            />
          </div>
        </Panel>

        <Panel title="Settings" kicker="stored in this browser" className="ctl-settings">
          <form
            className="settings-form"
            onSubmit={(e) => {
              e.preventDefault();
              void saveBackend();
            }}
          >
            <label className="field field--wide">
              <span className="field__label">Backend URL</span>
              <input className="input num" value={url} onChange={(e) => setUrl(e.target.value)} placeholder={`auto (now ${baseUrl || 'http://localhost:7878'})`} spellCheck={false} autoCapitalize="off" />
            </label>
            <label className="field field--wide">
              <span className="field__label">Token {health?.auth_required ? <Badge tone="fair">required by rhcsd</Badge> : null}</span>
              <input className="input num" type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="x-rhcs-token (optional)" autoComplete="off" />
            </label>
            <div className="row">
              <span className="muted small">Token is sent only on control requests.</span>
              <button type="submit" className="btn btn--accent">
                Save & reconnect
              </button>
            </div>
          </form>
          <div className="settings-toggles">
            <Switch checked={isDemo} onChange={(v) => void toggleDemo(v)} label="Demo mode (simulated data)" />
            <Switch
              checked={prefs.sound}
              onChange={(v) => {
                setPrefs({ sound: v });
                toast('info', v ? 'Sound on' : 'Sound off', v ? 'Soft chimes for handovers, disconnects and devices.' : undefined);
              }}
              label="Event sounds"
            />
            <label className="field field--wide">
              <span className="field__label">
                Accent hue <span className="num accent">{prefs.accentHue}°</span>
              </span>
              <input type="range" className="hue-range" min={0} max={359} step={1} value={prefs.accentHue} onChange={(e) => setPrefs({ accentHue: Number(e.target.value) })} />
            </label>
          </div>
        </Panel>
      </div>
    </div>
  );
}
