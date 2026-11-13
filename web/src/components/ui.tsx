import { useEffect, useRef, type ReactNode } from 'react';
import { isNum, prefersReducedMotion } from '../lib/format';
import { subscribeTick } from '../lib/ticker';
import type { Explained } from '../lib/explain';

export function Panel({
  title,
  kicker,
  actions,
  children,
  className,
  id,
}: {
  title?: ReactNode;
  kicker?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  id?: string;
}) {
  return (
    <section className={`panel ${className ?? ''}`} id={id} aria-label={typeof title === 'string' ? title : undefined}>
      {title || actions || kicker ? (
        <header className="panel__head">
          <div>
            {kicker ? <div className="panel__kicker">{kicker}</div> : null}
            {title ? <h2 className="panel__title">{title}</h2> : null}
          </div>
          {actions ? <div className="panel__actions">{actions}</div> : null}
        </header>
      ) : null}
      {children}
    </section>
  );
}

export function KV({ k, v, mono = true, hint }: { k: ReactNode; v: ReactNode; mono?: boolean; hint?: string }) {
  return (
    <div className="kv" title={hint}>
      <dt className="kv__k">{k}</dt>
      <dd className={`kv__v ${mono ? 'num' : ''}`}>{v}</dd>
    </div>
  );
}

export function Badge({ tone = 'neutral', children, pulse }: { tone?: string; children: ReactNode; pulse?: boolean }) {
  return <span className={`badge badge--${tone} ${pulse ? 'badge--pulse' : ''}`}>{children}</span>;
}

/** A number that eases toward its target without re-rendering React. */
export function Tween({
  value,
  digits = 0,
  format,
  className,
}: {
  value: number | null | undefined;
  digits?: number;
  format?: (v: number) => string;
  className?: string;
}) {
  const ref = useRef<HTMLSpanElement>(null);
  const shown = useRef<number>(NaN);
  const target = useRef<number>(NaN);
  target.current = isNum(value) ? value : NaN;
  const fmtRef = useRef(format);
  fmtRef.current = format;

  useEffect(() => {
    const render = () => {
      const el = ref.current;
      if (!el) return;
      const v = shown.current;
      const f = fmtRef.current;
      const s = Number.isNaN(v) ? '—' : f ? f(v) : v.toFixed(digits).replace('-', '−');
      if (el.textContent !== s) el.textContent = s;
    };
    return subscribeTick((_n, dt) => {
      const t = target.current;
      if (Number.isNaN(t)) shown.current = NaN;
      else if (Number.isNaN(shown.current) || prefersReducedMotion()) shown.current = t;
      else {
        const d = t - shown.current;
        shown.current = Math.abs(d) < Math.pow(10, -digits) / 4 ? t : shown.current + d * (1 - Math.exp(-dt * 7));
      }
      render();
    });
  }, [digits]);

  return <span ref={ref} className={`num ${className ?? ''}`}>—</span>;
}

const paths: Record<string, string> = {
  signal: 'M4 18a8 8 0 0 1 16 0M7.5 18a4.5 4.5 0 0 1 9 0M12 18h.01M2 18a10 10 0 0 1 20 0',
  phone: 'M8 2.5h8a1.5 1.5 0 0 1 1.5 1.5v16a1.5 1.5 0 0 1-1.5 1.5H8A1.5 1.5 0 0 1 6.5 20V4A1.5 1.5 0 0 1 8 2.5zM11 18.5h2',
  network: 'M12 12m-2 0a2 2 0 1 0 4 0a2 2 0 1 0-4 0M5 5m-1.6 0a1.6 1.6 0 1 0 3.2 0a1.6 1.6 0 1 0-3.2 0M19 6m-1.6 0a1.6 1.6 0 1 0 3.2 0a1.6 1.6 0 1 0-3.2 0M6 19m-1.6 0a1.6 1.6 0 1 0 3.2 0a1.6 1.6 0 1 0-3.2 0M18.5 18.5m-1.6 0a1.6 1.6 0 1 0 3.2 0a1.6 1.6 0 1 0-3.2 0M6.2 6.2l4.3 4.3M17.6 7.2l-4.1 3.6M7.2 17.8l3.4-4.4M17.3 17.3l-3.8-3.9',
  audio: 'M3 12h2M7 8v8M11 4v16M15 7v10M19 10v4M21 12h0',
  control: 'M4 7h10M18 7h2M4 17h2M10 17h10M16 4.5v5M8 14.5v5',
  log: 'M5 5h14M5 10h14M5 15h9M5 20h6',
  aim: 'M12 3v3M12 18v3M3 12h3M18 12h3M12 12m-6 0a6 6 0 1 0 12 0a6 6 0 1 0-12 0M12 12m-1 0a1 1 0 1 0 2 0a1 1 0 1 0-2 0',
  close: 'M6 6l12 12M18 6L6 18',
  scan: 'M3 12a9 9 0 0 1 15.5-6.2M21 12a9 9 0 0 1-15.5 6.2M18.5 3v3h-3M5.5 21v-3h3',
  download: 'M12 4v11M7 10l5 5 5-5M5 20h14',
  sound: 'M4 9h4l5-4v14l-5-4H4zM16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12',
  mute: 'M4 9h4l5-4v14l-5-4H4zM17 9l5 5M22 9l-5 5',
  pin: 'M12 21s-6.5-6-6.5-11a6.5 6.5 0 0 1 13 0c0 5-6.5 11-6.5 11zM12 10m-2.2 0a2.2 2.2 0 1 0 4.4 0a2.2 2.2 0 1 0-4.4 0',
  table: 'M4 5h16v14H4zM4 10h16M4 14.5h16M10 5v14',
  orbit: 'M12 12m-2 0a2 2 0 1 0 4 0a2 2 0 1 0-4 0M12 12m-8 0a8 8 0 1 0 16 0a8 8 0 1 0-16 0M18.5 7.5h.01',
  freeze: 'M8 5v14M16 5v14',
  play: 'M7 5l12 7-12 7z',
  mic: 'M12 3a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3zM6 11a6 6 0 0 0 12 0M12 17v4',
  trash: 'M5 7h14M10 7V5h4v2M7 7l1 13h8l1-13',
  boost: 'M13 2.5L5 13.5h6l-1 8 8-11h-6z',
  tower: 'M12 10v11M8.5 21h7M9.5 21L12 10l2.5 11M12 8.5m-1.5 0a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0M8 4.5a6 6 0 0 0 0 8M16 4.5a6 6 0 0 1 0 8M5.2 2.5a10 10 0 0 0 0 12M18.8 2.5a10 10 0 0 1 0 12',
};

export function Icon({ name, size = 20, className }: { name: keyof typeof paths | string; size?: number; className?: string }) {
  return (
    <svg
      className={`icon ${className ?? ''}`}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={paths[name] ?? ''} />
    </svg>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div className="seg" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          className={`seg__btn ${value === o.value ? 'is-on' : ''}`}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Switch({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={checked} className={`switch ${checked ? 'is-on' : ''}`} onClick={() => onChange(!checked)}>
      <span className="switch__track" aria-hidden="true">
        <span className="switch__thumb" />
      </span>
      <span className="switch__label">{label}</span>
    </button>
  );
}

/** A backend error in plain words: what happened, what to do, and the raw text. */
export function ErrorExplain({ e, compact }: { e: Explained | null; compact?: boolean }) {
  if (!e) return null;
  return (
    <div className={`explain ${compact ? 'explain--compact' : ''}`} role="status">
      <div className="explain__title">{e.title}</div>
      <p className="explain__detail">{e.detail}</p>
      <ol className="explain__fixes">
        {e.fixes.map((f) => (
          <li key={f}>{f}</li>
        ))}
      </ol>
      <details className="explain__raw">
        <summary>Technical detail</summary>
        <code>{e.raw}</code>
      </details>
    </div>
  );
}

export function Empty({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      <div className="empty__title">{title}</div>
      {children ? <div className="empty__body">{children}</div> : null}
    </div>
  );
}
