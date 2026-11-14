import { useEffect, useState, type ReactNode } from 'react';
import { AppDownload } from '../components/AppDownload';
import { EngineCard } from '../components/EngineCard';
import { Badge, ErrorExplain, Icon, KV, Panel, Switch } from '../components/ui';
import { explainError, type ErrorSource } from '../lib/explain';
import { fmtAgo, fmtDuration, isNum, str } from '../lib/format';
import { addHub, api, blockedByBrowser, goLive, normalizeUrl, pairingLink, probeHub, refreshSettings, removeHub, type HubAuth } from '../lib/rhcs';
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

const AUTH_LABEL: Record<HubAuth, string> = {
  ok: 'answered',
  needs: 'needs token',
  wrong: 'wrong token',
  locked: 'too many attempts',
  forbidden: 'remote access not set up',
  down: 'no answer',
};

async function copyPairing(url: string) {
  const { link, hasToken } = pairingLink(url);
  if (hasToken) {
    const ok = await confirmAction({
      title: 'Copy a link that contains the token?',
      body: 'Anyone who gets this link can control this backend. Send it only to your own devices (for example a message to yourself) and delete it afterwards. The token is in the part after # which is never sent to any website.',
      confirmLabel: 'Copy link',
      danger: true,
    });
    if (!ok) return;
  }
  try {
    await navigator.clipboard.writeText(link);
    toast('ok', 'Pairing link copied', hasToken ? 'Open it on the other device. It includes the token.' : 'No token is saved for this backend, so the link has the address only.');
  } catch {
    toast('error', 'Could not copy', 'The browser blocked clipboard access. Copy the address by hand instead.');
  }
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
  const [probes, setProbes] = useState<Record<string, HubAuth | 'checking'>>({});

  useEffect(() => {
    if (isNum(settings?.interval_s)) setIntervalV(settings!.interval_s!);
  }, [settings?.interval_s]);
  useEffect(() => {
    if (conn === 'live') void refreshSettings();
  }, [conn]);

  const listed = [
    { url: prefs.lastGood, label: 'last one that worked', removable: false },
    { url: 'http://localhost:7878', label: 'this device (phone hub or laptop)', removable: false },
    ...prefs.hubs.map((h) => ({ url: h, label: 'saved', removable: true })),
  ].filter((h, i, a) => h.url && a.findIndex((x) => x.url === h.url) === i);

  const recheck = async () => {
    setProbes(Object.fromEntries(listed.map((h) => [h.url, 'checking' as const])));
    await Promise.all(
      listed.map(async (h) => {
        const r = await probeHub(h.url, 3000);
        setProbes((p) => ({ ...p, [h.url]: r.auth }));
      }),
    );
  };
  useEffect(() => {
    void recheck();
    // once on opening the page; the button re-runs it (wrong tokens count as failed attempts)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
      body: next
        ? `PULSE will remember ${next} and search it together with the other saved backends.`
        : 'PULSE will keep searching its saved backends and this device automatically.',
      confirmLabel: 'Save & reconnect',
    });
    if (!ok) return;
    // The typed address joins the searched list; a forced single address is cleared so
    // PULSE can always fall back to the others (laptop, phone hub…).
    setPrefs({ baseUrl: '', token: token.trim() });
    setUrl('');
    if (next && !addHub(next, token)) {
      toast('error', 'That doesn’t look like an address', 'Try something like 192.168.0.245:7878');
      return;
    }
    if (!next) void goLive();
    toast('ok', 'Backend settings saved', next || 'searching all saved backends');
  };

  const r = sources?.router;
  const routerOn = sources?.router_configured !== false;
  const p = sources?.phone;
  const sc = sources?.scan;

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
        {routerOn ? (
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
        ) : null}

        <Panel title="Polling" kicker="rhcsd settings" className="ctl-poll">
          <label className="field field--wide">
            <span className="field__label">
              {routerOn ? 'Router refresh interval' : 'Refresh interval'} <span className="num accent">{interval.toFixed(1)} s</span>
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
            {routerOn ? (
              <SourceCard name="Router" kind="router" online={r?.online} lastOk={r?.last_ok} error={r?.error} extra={<KV k="Logged in" v={r?.logged_in ? 'yes' : r?.logged_in === false ? 'no' : '—'} />} />
            ) : null}
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

        <Panel title="Engine" kicker="rhcsd" className="ctl-engine">
          <EngineCard />
        </Panel>

        <Panel title="Android app" kicker="Download" className="ctl-app">
          <AppDownload variant="card" />
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
              <input className="input num" type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="access token (RHCSD_TOKEN)" autoComplete="off" />
            </label>
            <div className="row">
              <span className="muted small">Sent with every request to the backend (needed through a tunnel). A pairing link can save a different token per backend.</span>
              <button type="submit" className="btn btn--accent">
                Save & reconnect
              </button>
            </div>
          </form>
          <div className="field field--wide">
            <span className="field__label">Backends PULSE looks for automatically</span>
            <ul className="hubs">
              {listed.map((h) => {
                const pr = probes[h.url];
                return (
                  <li key={h.url}>
                    <span>
                      <span className="num">{h.url.replace(/^https?:\/\//, '')}</span> <span className="muted small">{h.label}</span>{' '}
                      {h.url === baseUrl && conn === 'live' ? <Badge tone="good">connected</Badge> : null}{' '}
                      {pr ? (
                        <Badge tone={pr === 'ok' ? 'good' : pr === 'down' || pr === 'checking' ? 'neutral' : 'fair'}>
                          {pr === 'checking' ? 'checking…' : AUTH_LABEL[pr]}
                        </Badge>
                      ) : null}
                    </span>
                    <span className="row">
                      {blockedByBrowser(h.url) ? (
                        <a className="btn btn--small btn--accent" href={h.url}>
                          Open directly
                        </a>
                      ) : null}
                      <button type="button" className="btn btn--small" onClick={() => void copyPairing(h.url)}>
                        Copy pairing link for this backend
                      </button>
                      {h.removable ? (
                        <button type="button" className="btn btn--small" onClick={() => removeHub(h.url)}>
                          Remove
                        </button>
                      ) : null}
                    </span>
                  </li>
                );
              })}
            </ul>
            <button type="button" className="btn btn--small" onClick={() => void recheck()}>
              Check them again
            </button>
            <span className="muted small">Add one in the box above and press Save &amp; reconnect: it's remembered and searched every time.</span>
          </div>
          <div className="settings-toggles">
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
