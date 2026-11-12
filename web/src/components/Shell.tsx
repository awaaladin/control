import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { fmtAgo, fmtTime } from '../lib/format';
import { goLive } from '../lib/rhcs';
import { dismissToast, isStale, STALE_MS, useStore } from '../lib/store';
import { useNow } from '../lib/ticker';
import { Icon } from './ui';

export type Route = 'signal' | 'phone' | 'network' | 'audio' | 'control' | 'log';
export const ROUTES: { id: Route; label: string; icon: string }[] = [
  { id: 'signal', label: 'Signal', icon: 'signal' },
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
export function UnreachableOverlay({ onSettings }: { onSettings: () => void }) {
  const unreachable = useStore((s) => s.unreachable);
  const conn = useStore((s) => s.conn);
  const baseUrl = useStore((s) => s.baseUrl);
  const telemetryAt = useStore((s) => s.telemetryAt);
  const now = useNow(1000);
  const [downSince, setDownSince] = useState<number | null>(null);

  useEffect(() => {
    if (conn === 'live' || conn === 'connecting') setDownSince(null);
    else setDownSince((d) => d ?? Date.now());
  }, [conn]);

  const down = conn !== 'live' && (unreachable || (downSince !== null && now - downSince > 4000));
  if (!down) return null;

  // An https page (e.g. the hosted copy) can't call a plain-http backend other than localhost.
  const blockedByHttps =
    location.protocol === 'https:' && /^http:\/\//.test(baseUrl || '') && !/^http:\/\/(localhost|127\.0\.0\.1)/.test(baseUrl || '');

  return (
    <div className="unreach" role="alertdialog" aria-labelledby="unreach-title">
      <div className="unreach__card panel">
        <div className="panel__kicker">Home system</div>
        <h2 id="unreach-title" className="unreach__title">
          Connection interrupted
        </h2>
        <p className="muted">
          PULSE can't reach your home system right now. The laptop may be off, asleep, or not on this network. PULSE keeps
          trying and reconnects by itself as soon as it's back.
        </p>
        <dl className="kvs">
          <div className="kv">
            <dt className="kv__k">Trying</dt>
            <dd className="kv__v num">{baseUrl || '…'}</dd>
          </div>
          <div className="kv">
            <dt className="kv__k">Last data</dt>
            <dd className="kv__v num">{telemetryAt ? fmtAgo(telemetryAt, now) : 'none yet'}</dd>
          </div>
        </dl>
        <ul className="unreach__tips muted">
          {blockedByHttps ? (
            <li>
              This secure page can't open plain <span className="num">http://</span> addresses on your home network. On the
              same Wi-Fi, open <span className="num">{baseUrl}</span> directly instead.
            </li>
          ) : null}
          <li>On the laptop: make sure rhcsd is running (it prints "listening on … 7878").</li>
          <li>On a phone or another device: be on the same Wi-Fi as the laptop, and check the address in Backend settings.</li>
        </ul>
        <div className="unreach__actions">
          <button type="button" className="btn btn--accent" onClick={() => void goLive()}>
            Retry now
          </button>
          <button type="button" className="btn" onClick={onSettings}>
            Backend settings
          </button>
        </div>
      </div>
    </div>
  );
}
