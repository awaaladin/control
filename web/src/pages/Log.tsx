import { useMemo, useState } from 'react';
import { Badge, Empty, Icon, Panel } from '../components/ui';
import { samples, samplesToCsv } from '../lib/buffer';
import { fmtDateTime, fmtTime } from '../lib/format';
import { toast, useStore } from '../lib/store';
import { useNow } from '../lib/ticker';
import type { RhcsEvent } from '../lib/types';

const KINDS: { kind: string; label: string; tone: string }[] = [
  { kind: 'handover', label: 'Handover', tone: 'violet' },
  { kind: 'login', label: 'Login', tone: 'accent' },
  { kind: 'router_online', label: 'Router online', tone: 'good' },
  { kind: 'disconnect', label: 'Disconnect', tone: 'poor' },
  { kind: 'error', label: 'Error', tone: 'poor' },
  { kind: 'phone_online', label: 'Phone online', tone: 'good' },
  { kind: 'phone_offline', label: 'Phone offline', tone: 'fair' },
  { kind: 'device_join', label: 'Device join', tone: 'excellent' },
  { kind: 'device_leave', label: 'Device leave', tone: 'neutral' },
];
const toneOf = (k: string) => KINDS.find((x) => x.kind === k)?.tone ?? 'neutral';
const labelOf = (k: string) => KINDS.find((x) => x.kind === k)?.label ?? k;

function downloadCsv() {
  if (samples.len === 0) {
    toast('warn', 'Nothing to export', 'The signal buffer is empty.');
    return;
  }
  const blob = new Blob([samplesToCsv()], { type: 'text/csv' });
  const a = document.createElement('a');
  const stamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
  a.href = URL.createObjectURL(blob);
  a.download = `pulse-signal-${useStore.getState().conn === 'demo' ? 'DEMO-' : ''}${stamp}.csv`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  window.setTimeout(() => URL.revokeObjectURL(a.href), 2000);
  toast('ok', 'CSV exported', `${samples.len.toLocaleString()} samples, ${fmtDateTime(samples.first())} → ${fmtTime(samples.last())}.`);
}

export function LogPage() {
  const events = useStore((s) => s.events);
  const [off, setOff] = useState<Set<string>>(new Set());
  const [q, setQ] = useState('');
  useNow(5000);

  const counts = useMemo(() => {
    const c = new Map<string, number>();
    for (const e of events) c.set(e.kind, (c.get(e.kind) ?? 0) + 1);
    return c;
  }, [events]);

  const kinds = useMemo(() => {
    const extra = [...counts.keys()].filter((k) => !KINDS.some((x) => x.kind === k)).map((k) => ({ kind: k, label: k, tone: 'neutral' }));
    return [...KINDS, ...extra];
  }, [counts]);

  const rows = useMemo(() => {
    const s = q.trim().toLowerCase();
    const out: RhcsEvent[] = [];
    for (let i = events.length - 1; i >= 0; i--) {
      const e = events[i];
      if (off.has(e.kind)) continue;
      if (s && !`${e.kind} ${e.message ?? ''} ${JSON.stringify(e.data ?? '')}`.toLowerCase().includes(s)) continue;
      out.push(e);
    }
    return out;
  }, [events, off, q]);

  const toggle = (k: string) =>
    setOff((prev) => {
      const n = new Set(prev);
      if (n.has(k)) n.delete(k);
      else n.add(k);
      return n;
    });

  // Group by day for an editorial rhythm.
  let lastDay = '';

  return (
    <div className="page page--log">
      <header className="pagehead">
        <div>
          <div className="pagehead__kicker">
            {events.length} events · {samples.len.toLocaleString()} samples buffered
          </div>
          <h1 className="pagehead__title">Log</h1>
        </div>
        <div className="pagehead__actions">
          <button type="button" className="btn" onClick={downloadCsv}>
            <Icon name="download" size={18} /> Export signal CSV
          </button>
        </div>
      </header>

      <Panel>
        <div className="log-filters">
          <div className="chips" role="group" aria-label="Filter by kind">
            {kinds.map((k) => (
              <button
                type="button"
                key={k.kind}
                className={`chip chip--${k.tone} ${off.has(k.kind) ? '' : 'is-on'}`}
                aria-pressed={!off.has(k.kind)}
                onClick={() => toggle(k.kind)}
              >
                {k.label} <span className="num">{counts.get(k.kind) ?? 0}</span>
              </button>
            ))}
          </div>
          <input className="input input--search" type="search" placeholder="Search messages…" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search events" />
        </div>
      </Panel>

      {rows.length === 0 ? (
        <Panel>
          <Empty title={events.length ? 'No events match' : 'No events yet'}>{events.length ? 'Adjust the filters or search.' : 'Events stream in as the daemon notices changes.'}</Empty>
        </Panel>
      ) : (
        <ol className="events" aria-live="polite">
          {rows.slice(0, 500).map((e) => {
            const day = new Date(e.ts).toDateString();
            const header = day !== lastDay ? (lastDay = day) : null;
            return (
              <li key={`${e.ts}-${e.kind}-${e.message}`} className={`event event--${toneOf(e.kind)}`}>
                {header ? <div className="event__day">{new Date(e.ts).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })}</div> : null}
                <div className="event__row">
                  <time className="event__time num" dateTime={new Date(e.ts).toISOString()}>
                    {fmtTime(e.ts)}
                  </time>
                  <Badge tone={toneOf(e.kind)}>{labelOf(e.kind)}</Badge>
                  <span className="event__msg">{e.message || '—'}</span>
                </div>
              </li>
            );
          })}
        </ol>
      )}
    </div>
  );
}
