import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { fmtAgo, fmtTime } from '../lib/format';
import { addHub, blockedByBrowser, goLive, lastSearch, normalizeUrl, setBackendToken } from '../lib/rhcs';
import { dismissToast, isStale, STALE_MS, useStore } from '../lib/store';
import { useNow } from '../lib/ticker';
import { AppDownload } from './AppDownload';
import { EnginePrompt } from './EnginePrompt';
import { Icon } from './ui';

export type Route = 'signal' | 'boost' | 'towers' | 'phone' | 'network' | 'audio' | 'control' | 'log';
export const ROUTES: { id: Route; label: string; icon: string }[] = [
  { id: 'signal', label: 'Signal', icon: 'signal' },
  { id: 'boost', label: 'Boost', icon: 'boost' },
  { id: 'towers', label: 'Towers', icon: 'tower' },
  { id: 'phone', label: 'Phone', icon: 'phone' },
  { id: 'network', label: 'Network', icon: 'network' },
  { id: 'audio', label: 'Audio', icon: 'audio' },
  { id: 'control', label: 'Control', icon: 'control' },
  { id: 'log', label: 'Log', icon: 'log' },
];

function parse(path: string): Route {
  const seg = path.replace(/^\/+/, '').split('/')[0] as Route;
  return ROUTES.some((r) => r.id === seg) ? seg : 'signal';
}

export function useRoute(): [Route, (r: Route) => void] {
  const [route, setRoute] = useState<Route>(() => parse(location.pathname));
  useEffect(() => {
    const on = () => setRoute(parse(location.pathname));
    window.addEventListener('popstate', on);
    return () => window.removeEventListener('popstate', on);
  }, []);
  const go = (r: Route) => {
    const path = r === 'signal' ? '/' : `/${r}`;
    if (location.pathname !== path) history.pushState(null, '', path);
    setRoute(r);
  };
  return [route, go];
}

export function Rail({ route, go }: { route: Route; go: (r: Route) => void }) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: KeyboardEvent, i: number) => {
    const n = ROUTES.length;
    let j = -1;
    if (e.key === 'ArrowDown' || e.key === 'ArrowRight') j = (i + 1) % n;
    if (e.key === 'ArrowUp' || e.key === 'ArrowLeft') j = (i - 1 + n) % n;
    if (e.key === 'Home') j = 0;
    if (e.key === 'End') j = n - 1;
    if (j >= 0) {
      e.preventDefault();
      refs.current[j]?.focus();
    }
  };
  return (
    <nav className="rail" aria-label="Sections">
      <div className="rail__brand" aria-hidden="true">
        <svg viewBox="0 0 32 32" width="30" height="30">
          <circle cx="16" cy="16" r="10" fill="none" stroke="rgba(255,255,255,.12)" strokeWidth="2.5" />
          <path d="M8.9 23.1A10 10 0 1 1 23.1 23.1" fill="none" stroke="var(--accent)" strokeWidth="2.5" strokeLinecap="round" />
          <circle cx="16" cy="16" r="2.6" fill="var(--accent)" />
        </svg>
        <span>PULSE</span>
      </div>
      <ul className="rail__list">
        {ROUTES.map((r, i) => (
          <li key={r.id}>
            <button
              ref={(el) => (refs.current[i] = el)}
              type="button"
              className={`rail__btn ${route === r.id ? 'is-on' : ''}`}
              aria-current={route === r.id ? 'page' : undefined}
              onClick={() => go(r.id)}
              onKeyDown={(e) => onKey(e, i)}
            >
              <Icon name={r.icon} size={21} />
              <span className="rail__label">{r.label}</span>
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}

export function TopBar() {
  const conn = useStore((s) => s.conn);
  const baseUrl = useStore((s) => s.baseUrl);
  const sources = useStore((s) => s.sources);
  const telemetryAt = useStore((s) => s.telemetryAt);
  const now = useNow(1000);
  const stale = conn === 'live' && isStale(telemetryAt, STALE_MS.telemetry, now);

  const state =
    conn === 'live'
      ? stale
        ? { label: 'LIVE · NO DATA', tone: 'stale' }
        : { label: 'LIVE', tone: 'live' }
      : conn === 'connecting'
        ? { label: 'CONNECTING', tone: 'wait' }
        : conn === 'offline'
          ? { label: 'CONNECTION INTERRUPTED', tone: 'off' }
          : { label: 'RECONNECTING', tone: 'wait' };

  const router = sources?.router;
  return (
    <header className="topbar">
      <div className={`conn conn--${state.tone}`} role="status" aria-live="polite">
        <span className="conn__dot" aria-hidden="true" />
        <span className="conn__label">{state.label}</span>
      </div>
      <div className="topbar__url num" title="Backend URL">
        {baseUrl ? baseUrl.replace(/^https?:\/\//, '') : '…'}
      </div>
      <div className="topbar__router" title="Router">
        <span className={`dot ${router?.online ? 'dot--ok' : 'dot--off'}`} aria-hidden="true" />
        <span className="topbar__model">{router?.model || 'router'}</span>
        <span className="topbar__host num">{router?.host || '—'}</span>
      </div>
      <AppDownload variant="pill" />
      <div className="topbar__clock num" aria-label="Local time">
        {fmtTime(now)}
      </div>
    </header>
  );
}

export function DemoBadge() {
  const conn = useStore((s) => s.conn);
  if (conn !== 'demo') return null;
  return (
    <div className="demo-badge" role="note" aria-label="Demo mode: all data is simulated">
      DEMO <small>simulated data</small>
    </div>
  );
}

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  return (
    <div className="toasts" aria-live="polite" aria-atomic="false">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast--${t.tone}`} role={t.tone === 'error' ? 'alert' : 'status'}>
          <div className="toast__body">
            <div className="toast__title">{t.title}</div>
            {t.body ? <div className="toast__text">{t.body}</div> : null}
          </div>
          <button type="button" className="toast__close" aria-label="Dismiss" onClick={() => dismissToast(t.id)}>
            <Icon name="close" size={16} />
          </button>
        </div>
      ))}
    </div>
  );
}

export function ConfirmDialog() {
  const req = useStore((s) => s.confirmReq);
  const ref = useRef<HTMLDialogElement>(null);
  const okRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (req && !d.open) {
      d.showModal();
      window.setTimeout(() => okRef.current?.focus(), 30);
    }
    if (!req && d.open) d.close();
  }, [req]);
  return (
    <dialog
      ref={ref}
      className="dialog"
      aria-labelledby="confirm-title"
      onCancel={(e) => {
        e.preventDefault();
        req?.resolve(false);
      }}
      onClick={(e) => {
        if (e.target === ref.current) req?.resolve(false);
      }}
    >
      {req ? (
        <div className="dialog__inner">
          <h2 id="confirm-title" className="dialog__title">
            {req.title}
          </h2>
          <p className="dialog__body">{req.body}</p>
          <div className="dialog__actions">
            <button type="button" className="btn btn--ghost" onClick={() => req.resolve(false)}>
              Cancel
            </button>
            <button ref={okRef} type="button" className={`btn ${req.danger ? 'btn--danger' : 'btn--accent'}`} onClick={() => req.resolve(true)}>
              {req.confirmLabel ?? 'Confirm'}
            </button>
          </div>
        </div>
      ) : null}
    </dialog>
  );
}

/** Shown when PULSE can't reach rhcsd: at startup, or when a live connection drops for more than a few seconds. */
/** The connection card plus, when no engine is found anywhere, the "get the engine" prompt. */
export function UnreachableOverlay({ onSettings }: { onSettings: () => void }) {
  return (
    <>
      <ConnectionCard onSettings={onSettings} />
      <EnginePrompt />
    </>
  );
}

function ConnectionCard({ onSettings }: { onSettings: () => void }) {
  const unreachable = useStore((s) => s.unreachable);
  const conn = useStore((s) => s.conn);
  const baseUrl = useStore((s) => s.baseUrl);
  const telemetryAt = useStore((s) => s.telemetryAt);
  const now = useNow(1000);
  const [downSince, setDownSince] = useState<number | null>(null);
  const [hidden, setHidden] = useState(false);
  const [busy, setBusy] = useState(false);
  const [addr, setAddr] = useState('');
  const [note, setNote] = useState<string | null>(null);
  const authIssue = useStore((s) => s.authIssue);
  const [tok, setTok] = useState('');

  useEffect(() => {
    if (conn === 'live' || conn === 'connecting') setDownSince(null);
    else setDownSince((d) => d ?? Date.now());
    if (conn === 'live') {
      setHidden(false);
      setNote(null);
    }
  }, [conn]);

  // Escape closes the card (it shrinks to the small bar at the top).
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') setHidden(true);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const down = conn !== 'live' && (unreachable || (downSince !== null && now - downSince > 4000));
  if (!down) return null;

  const retry = async () => {
    setBusy(true);
    setNote(null);
    await goLive();
    setBusy(false);
    if (useStore.getState().conn !== 'live') {
      const n = lastSearch.length;
      setNote(`Looked in ${n} place${n === 1 ? '' : 's'}: no answer yet. It keeps searching on its own.`);
    }
  };

  const connectTo = async () => {
    const url = normalizeUrl(addr);
    if (!addHub(addr)) {
      setNote('That doesn’t look like an address. Try something like 192.168.0.245:7878');
      return;
    }
    setBusy(true);
    setNote(null);
    // addHub started the connection; give it a moment, then say what happened.
    await new Promise((r) => setTimeout(r, 3500));
    setBusy(false);
    if (useStore.getState().conn !== 'live') {
      setNote(
        blockedByBrowser(url)
          ? 'Saved. This secure page isn’t allowed to call that address, so open it directly with the button below.'
          : `Saved ${url}, but nothing answered there yet. Is rhcsd running, and are you on the same network?`,
      );
    }
  };

  const submitToken = () => {
    if (!tok.trim()) return;
    setBackendToken(baseUrl, tok);
    setTok('');
    setNote(null);
  };

  if (authIssue && !hidden) {
    const host = baseUrl.replace(/^https?:\/\//, '');
    const text: Record<NonNullable<typeof authIssue>, [string, string]> = {
      needs: ['This backend needs an access token', `${host} answered, but it only talks to PULSE when you give it the access token (RHCSD_TOKEN in the .env on that device).`],
      wrong: ['Wrong access token', `${host} rejected the token saved in this browser. Check it against RHCSD_TOKEN in the .env on that device, then enter it again.`],
      locked: ['Too many wrong attempts', `${host} is ignoring this address for a few minutes after repeated wrong tokens. Wait, then enter the right token.`],
      forbidden: ['Remote access is not set up on that device', `${host} refuses remote callers until RHCSD_TOKEN of at least 20 characters is set in its .env. Set it there, restart rhcsd, then retry.`],
    };
    const [title, body] = text[authIssue];
    return (
      <div
        className="unreach"
        role="alertdialog"
        aria-labelledby="unreach-title"
        onClick={(e) => {
          if (e.target === e.currentTarget) setHidden(true);
        }}
      >
        <div className="unreach__card panel">
          <button type="button" className="unreach__x" aria-label="Close" onClick={() => setHidden(true)}>
            ✕
          </button>
          <div className="panel__kicker">Home system</div>
          <h2 id="unreach-title" className="unreach__title">
            {title}
          </h2>
          <p className="muted">{body}</p>
          {authIssue !== 'forbidden' ? (
            <form
              className="unreach__add"
              onSubmit={(e) => {
                e.preventDefault();
                submitToken();
              }}
            >
              <label className="field field--wide">
                <span className="field__label">Access token for {host}</span>
                <input
                  className="input num"
                  type="password"
                  value={tok}
                  onChange={(e) => setTok(e.target.value)}
                  autoComplete="off"
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                />
              </label>
              <button type="submit" className="btn btn--accent" disabled={!tok.trim()}>
                Save token & connect
              </button>
            </form>
          ) : null}
          <div className="unreach__actions">
            <button type="button" className="btn btn--accent" onClick={() => void retry()} disabled={busy}>
              {busy ? 'Checking…' : 'Check again'}
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => {
                setHidden(true);
                onSettings();
              }}
            >
              Backend settings
            </button>
            <button type="button" className="btn" onClick={() => setHidden(true)}>
              Hide
            </button>
          </div>
        </div>
      </div>
    );
  }

  const typed = addr.trim() ? normalizeUrl(addr) : '';
  const direct = typed && blockedByBrowser(typed) ? typed : '';

  if (hidden) {
    return (
      <div className="unreach-bar" role="status">
        <span className="dot dot--off" aria-hidden="true" />
        <span>Connection interrupted</span>
        <button type="button" className="btn btn--small btn--accent" onClick={() => void retry()} disabled={busy}>
          {busy ? 'Searching…' : 'Retry'}
        </button>
        <button type="button" className="btn btn--small" onClick={() => setHidden(false)}>
          Details
        </button>
      </div>
    );
  }

  return (
    <div
      className="unreach"
      role="alertdialog"
      aria-labelledby="unreach-title"
      onClick={(e) => {
        if (e.target === e.currentTarget) setHidden(true);
      }}
    >
      <div className="unreach__card panel">
        <button type="button" className="unreach__x" aria-label="Close" onClick={() => setHidden(true)}>
          ✕
        </button>
        <div className="panel__kicker">Home system</div>
        <h2 id="unreach-title" className="unreach__title">
          Connection interrupted
        </h2>
        <p className="muted">
          PULSE can't reach your home system right now. It's looking in several places and reconnects by itself as soon as
          one answers.
        </p>

        <ul className="unreach__looked">
          {(lastSearch.length ? lastSearch : [{ url: baseUrl || '…', ok: false, blocked: false, auth: 'down' as const }]).map((p) => (
            <li key={p.url}>
              <span className={`dot ${p.ok ? 'dot--ok' : 'dot--off'}`} aria-hidden="true" />
              <span className="num unreach__url">{p.url.replace(/^https?:\/\//, '')}</span>
              <span className="muted small">{p.ok ? 'answered' : p.blocked ? 'blocked by browser rule' : p.auth !== 'down' ? 'needs token' : 'no answer'}</span>
              {p.blocked ? (
                <a className="btn btn--small btn--accent" href={p.url}>
                  Open directly
                </a>
              ) : null}
            </li>
          ))}
        </ul>
        <p className="muted small">Last data: {telemetryAt ? fmtAgo(telemetryAt, now) : 'none yet'}</p>

        <AppDownload variant="card" />
        {location.protocol === 'https:' ? (
          <p className="muted small">This secure page can't open addresses like 10.x.x.x directly; open the address as its own page instead.</p>
        ) : null}
        <form
          className="unreach__add"
          onSubmit={(e) => {
            e.preventDefault();
            void connectTo();
          }}
        >
          <label className="field field--wide">
            <span className="field__label">Know where it is? Type its address</span>
            <input
              className="input num"
              value={addr}
              onChange={(e) => setAddr(e.target.value)}
              placeholder="192.168.0.245:7878"
              inputMode="url"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
            />
          </label>
          <button type="submit" className="btn btn--accent" disabled={busy || !addr.trim()}>
            {busy ? 'Connecting…' : 'Save & connect'}
          </button>
        </form>

        {direct ? (
          <a className="btn btn--accent unreach__direct" href={direct}>
            Open {direct.replace(/^https?:\/\//, '')} directly
          </a>
        ) : null}
        {note ? (
          <p className="unreach__note" role="status">
            {note}
          </p>
        ) : null}

        <ul className="unreach__tips muted">
          <li>On the laptop: rhcsd must be running (it prints "listening on … 7878").</li>
          <li>On the phone: start the hub in Termux with <span className="num">~/rhcs/start.sh</span>, then Retry: this device is searched automatically.</li>
          <li>Phone and laptop on the same Wi-Fi or hotspot? Open the laptop's address directly (button above) and it works without any setup.</li>
        </ul>
        <div className="unreach__actions">
          <button type="button" className="btn btn--accent" onClick={() => void retry()} disabled={busy}>
            {busy ? 'Searching…' : 'Retry now'}
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => {
              setHidden(true);
              onSettings();
            }}
          >
            Backend settings
          </button>
          <button type="button" className="btn" onClick={() => setHidden(true)}>
            Hide
          </button>
        </div>
      </div>
    </div>
  );
}
