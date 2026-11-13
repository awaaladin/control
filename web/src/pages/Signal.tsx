import { useEffect, useMemo } from 'react';
import { GlowRing } from '../components/GlowRing';
import { SeriesChart } from '../components/SeriesChart';
import { ThroughputChart } from '../components/ThroughputChart';
import { Badge, ErrorExplain, Icon, KV, Panel, Segmented, Tween } from '../components/ui';
import { phoneSamples } from '../lib/buffer';
import { useConnections, useConnectionsPolling, type Connection } from '../lib/connections';
import { explainError } from '../lib/explain';
import {
  bandLabel,
  DESCRIPTIONS,
  enb,
  fmt,
  fmtAgo,
  fmtBytes,
  fmtDuration,
  fmtTime,
  grade,
  isNum,
  LABELS,
  type Metric,
  mbps,
  RANGES,
  sector,
  str,
  THRESHOLDS,
  UNITS,
} from '../lib/format';
import { ensureHistory } from '../lib/rhcs';
import { isStale, STALE_MS, useStore, type WindowKey } from '../lib/store';
import { useNow } from '../lib/ticker';
import type { PhoneStatus, Telemetry } from '../lib/types';
import '../styles/extras.css';

const telemetryStale = () => {
  const s = useStore.getState();
  return isStale(s.telemetryAt, STALE_MS.telemetry);
};
const phoneStale = () => isStale(useStore.getState().phoneAt, STALE_MS.phone);

const WINDOWS: { value: WindowKey; label: string }[] = [
  { value: '1m', label: '1 min' },
  { value: '10m', label: '10 min' },
  { value: '1h', label: '1 h' },
  { value: '24h', label: '24 h' },
];

export function MetricRing({ metric, value, size = 'lg', isStaleFn = telemetryStale, sub }: { metric: Metric; value: number | null | undefined; size?: 'sm' | 'md' | 'lg' | 'xl'; isStaleFn?: () => boolean; sub?: string }) {
  return (
    <GlowRing
      label={LABELS[metric]}
      value={value}
      range={RANGES[metric]}
      unit={UNITS[metric]}
      digits={metric === 'rsrp' ? 0 : 1}
      gradeOf={(v) => grade(metric, v)}
      thresholds={THRESHOLDS[metric]}
      isStale={isStaleFn}
      size={size}
      sub={sub}
    />
  );
}

function Bars({ n }: { n: number | null | undefined }) {
  const v = isNum(n) ? Math.max(0, Math.min(5, Math.round(n))) : 0;
  return (
    <span className="bars" role="img" aria-label={isNum(n) ? `${v} of 5 bars` : 'no bar reading'}>
      {[1, 2, 3, 4, 5].map((i) => (
        <span key={i} className={`bars__b ${i <= v ? 'is-on' : ''}`} style={{ height: `${30 + i * 14}%` }} />
      ))}
    </span>
  );
}

export function TowerCard({ t }: { t: Telemetry | null }) {
  const cell = t?.cell_id;
  return (
    <Panel title="Serving cell" kicker="Tower · router" className="tower">
      <div className="tower__hero">
        <div>
          <div className="tower__k">eNB</div>
          <div className="tower__enb num">{fmt(enb(cell))}</div>
        </div>
        <div>
          <div className="tower__k">Sector</div>
          <div className="tower__sector num">{fmt(sector(cell))}</div>
        </div>
        <div>
          <div className="tower__k">PCI</div>
          <div className="tower__sector num">{fmt(t?.pci)}</div>
        </div>
      </div>
      <dl className="kvs">
        <KV k="Band" v={bandLabel(t?.band)} />
        <KV k="EARFCN" v={fmt(t?.earfcn)} />
        <KV k="TAC" v={str(t?.tac)} />
        <KV k="MCC / MNC" v={t?.mcc || t?.mnc ? `${str(t?.mcc)} / ${str(t?.mnc)}` : '—'} />
        <KV k="Cell ID" v={fmt(cell)} />
        <KV k="RSSI" v={isNum(t?.rssi) ? `${fmt(t?.rssi)} dBm` : '—'} />
        <KV k="Bars" v={<Bars n={t?.signal_bars} />} />
        <KV k="Provider" v={str(t?.provider)} mono={false} />
        <KV k="Network" v={str(t?.network_type)} />
        <KV k="PPP" v={str(t?.ppp_status)} />
        <KV k="Modem" v={str(t?.modem_state)} />
        <KV k="Session" v={fmtDuration(t?.session_secs)} />
        <KV k="Session data" v={`↓ ${fmtBytes(t?.session_rx_bytes)}  ↑ ${fmtBytes(t?.session_tx_bytes)}`} />
        <KV k="This month" v={`↓ ${fmtBytes(t?.month_rx_bytes)}  ↑ ${fmtBytes(t?.month_tx_bytes)}`} />
      </dl>
    </Panel>
  );
}

/** The same card for the phone's own radio (fewer fields: phones report no session data). */
function PhoneTowerCard({ p }: { p: PhoneStatus | null }) {
  const cell = p?.cell_id;
  return (
    <Panel title="Serving cell" kicker="Tower · phone" className="tower">
      <div className="tower__hero">
        <div>
          <div className="tower__k">eNB</div>
          <div className="tower__enb num">{fmt(enb(cell))}</div>
        </div>
        <div>
          <div className="tower__k">Sector</div>
          <div className="tower__sector num">{fmt(sector(cell))}</div>
        </div>
        <div>
          <div className="tower__k">PCI</div>
          <div className="tower__sector num">{fmt(p?.pci)}</div>
        </div>
      </div>
      <dl className="kvs">
        <KV k="Band" v={bandLabel(p?.band)} />
        <KV k="EARFCN" v={fmt(p?.earfcn)} />
        <KV k="TAC" v={fmt(p?.tac)} />
        <KV k="MCC / MNC" v={p?.mcc || p?.mnc ? `${str(p?.mcc)} / ${str(p?.mnc)}` : '—'} />
        <KV k="Cell ID" v={fmt(cell)} />
        <KV k="RSSI" v={isNum(p?.rssi) ? `${fmt(p?.rssi)} dBm` : '—'} />
        <KV k="Phone" v={str(p?.model)} mono={false} />
      </dl>
    </Panel>
  );
}

function ChartPanel({ metric, value, phone }: { metric: Metric; value: number | null | undefined; phone?: boolean }) {
  const g = grade(metric, value);
  return (
    <Panel
      className="chartpanel"
      kicker={DESCRIPTIONS[metric]}
      title={
        <span className="chartpanel__title">
          {LABELS[metric]}
          <span className={`chartpanel__value num g-${g}`}>
            <Tween value={value} digits={metric === 'rsrp' ? 0 : 1} /> <small>{UNITS[metric]}</small>
          </span>
        </span>
      }
    >
      {phone ? <SeriesChart metric={metric} buffer={phoneSamples} source="phone" /> : <SeriesChart metric={metric} />}
    </Panel>
  );
}

function HandoverCards() {
  const events = useStore((s) => s.events);
  const now = useNow(5000);
  const hos = useMemo(() => events.filter((e) => e.kind === 'handover').slice(-6).reverse(), [events]);
  return (
    <Panel title="Handovers" kicker="Cell changes" className="handovers">
      {hos.length === 0 ? (
        <p className="muted small">No handovers recorded yet. They appear here and as violet markers on every chart.</p>
      ) : (
        <ol className="hos">
          {hos.map((e) => (
            <li key={`${e.ts}-${e.message}`} className="ho">
              <span className="ho__mark" aria-hidden="true">⇄</span>
              <div>
                <div className="ho__msg num">{e.message ?? 'handover'}</div>
                <div className="ho__time muted small num">
                  {fmtTime(e.ts)} · {fmtAgo(e.ts, now)}
                </div>
              </div>
            </li>
          ))}
        </ol>
      )}
    </Panel>
  );
}

/** Live values of a connection's radio: from the WebSocket stores, which are fresher than the poll. */
function radioOf(c: Connection | undefined, t: Telemetry | null, p: PhoneStatus | null) {
  const src = c?.radio?.source ?? (c?.kind === 'phone' ? 'phone' : c?.kind === 'router' ? 'router' : c ? null : 'router');
  if (src === 'router') return { src, rsrp: t?.rsrp, sinr: t?.sinr, rsrq: t?.rsrq };
  if (src === 'phone') return { src, rsrp: p?.rsrp, sinr: p?.sinr, rsrq: p?.rsrq };
  return { src: null, rsrp: null, sinr: null, rsrq: null };
}

/** Chips naming every connection; picks which one the page shows. */
function ConnectionPicker({ list, selected, onPick }: { list: Connection[]; selected: string | null; onPick: (id: string) => void }) {
  if (list.length === 0) return null;
  return (
    <div className="connpick" role="radiogroup" aria-label="Connection shown">
      {list.map((c) => (
        <button
          key={c.id}
          type="button"
          role="radio"
          aria-checked={selected === c.id}
          className={`connpick__chip ${selected === c.id ? 'is-on' : ''}`}
          onClick={() => onPick(c.id)}
        >
          <span className="connpick__name">{c.name}</span>
          {c.in_use ? <span className="connpick__tag">in use</span> : null}
        </button>
      ))}
    </div>
  );
}

/** All connections side by side, by name: SINR, RSRP, ping. Best one highlighted. */
function ConnectionsStrip({ list, best, t, p, onPick }: { list: Connection[]; best: string | null; t: Telemetry | null; p: PhoneStatus | null; onPick: (id: string) => void }) {
  if (list.length < 1) return null;
  return (
    <Panel kicker="Every internet connection of this computer" title="Connections" className="connstrip">
      <ul className="connstrip__list">
        {list.map((c) => {
          const r = radioOf(c, t, p);
          return (
            <li key={c.id} className={`connstrip__item ${c.id === best ? 'is-best' : ''}`}>
              <button type="button" className="connstrip__name linkbtn" onClick={() => onPick(c.id)}>
                {c.name}
              </button>
              <span className="connstrip__tags">
                {c.id === best ? <Badge tone="excellent">best</Badge> : null}
                {c.in_use ? <Badge tone="accent">in use</Badge> : null}
              </span>
              <span className="connstrip__v">
                <span className="muted small">SINR </span>
                <span className={`num g-${grade('sinr', r.sinr)}`}>{isNum(r.sinr) ? `${fmt(r.sinr, 1)} dB` : '—'}</span>
              </span>
              <span className="connstrip__v">
                <span className="muted small">RSRP </span>
                <span className={`num g-${grade('rsrp', r.rsrp)}`}>{isNum(r.rsrp) ? `${fmt(r.rsrp)} dBm` : '—'}</span>
              </span>
              <span className="connstrip__v">
                <span className="muted small">ping </span>
                <span className="num">{isNum(c.ping?.ping_ms) ? `${fmt(c.ping!.ping_ms)} ms` : '—'}</span>
              </span>
              {!c.radio && c.radio_note ? <span className="connstrip__note muted small">{c.radio_note}</span> : null}
            </li>
          );
        })}
      </ul>
      <p className="muted small">
        SINR and RSRP come from the router or the phone behind each connection; ping from the last Game Boost measurement. "Best" is the
        lowest Game Boost score, or the cleanest signal when nothing has been measured.
      </p>
    </Panel>
  );
}

export function SignalPage() {
  useConnectionsPolling(4000);
  const conns = useConnections((s) => s.view);
  const t = useStore((s) => s.telemetry);
  const p = useStore((s) => s.phone);
  const win = useStore((s) => s.window);
  const setWindow = useStore((s) => s.setWindow);
  const setAim = useStore((s) => s.setAim);
  const conn = useStore((s) => s.conn);
  const telemetryAt = useStore((s) => s.telemetryAt);
  const phoneAt = useStore((s) => s.phoneAt);
  const routerSrc = useStore((s) => s.sources?.router);
  const picked = useStore((s) => s.signalConn);
  const now = useNow(1000);

  const list = conns?.connections ?? [];
  const selected = list.find((c) => c.id === picked) ?? list.find((c) => c.in_use) ?? list.find((c) => c.kind === 'router') ?? list[0];
  const pick = (id: string) => useStore.setState({ signalConn: id });
  const r = radioOf(selected, t, p);
  const isPhone = r.src === 'phone';
  const isRouter = r.src === 'router';
  const at = isPhone ? phoneAt : telemetryAt;
  const stale = isStale(at, isPhone ? STALE_MS.phone : STALE_MS.telemetry, now);
  const staleFn = isPhone ? phoneStale : telemetryStale;

  useEffect(() => {
    void ensureHistory();
  }, [win, conn]);

  return (
    <div className="page page--signal">
      <header className="pagehead">
        <div>
          <div className="pagehead__kicker">
            {selected ? `${selected.name} · ` : ''}
            {isPhone ? `${bandLabel(p?.band)} · PCI ${fmt(p?.pci)}` : isRouter ? `${t?.network_type ? `${t.network_type} · ` : ''}${bandLabel(t?.band)} · PCI ${fmt(t?.pci)}` : 'no radio data'}
          </div>
          <h1 className="pagehead__title">Signal</h1>
        </div>
        <div className="pagehead__actions">
          {r.src && stale && conn !== 'connecting' ? (
            <Badge tone="stale">{at ? `no reading for ${fmtAgo(at, now).replace(' ago', '')}` : 'waiting for first reading'}</Badge>
          ) : null}
          <button type="button" className="btn btn--accent" onClick={() => setAim(true)}>
            <Icon name="aim" size={18} /> Aim mode
          </button>
        </div>
      </header>

      <ConnectionPicker list={list} selected={selected?.id ?? null} onPick={pick} />

      {isRouter && routerSrc && !routerSrc.online && routerSrc.error && conn !== 'demo' ? (
        <ErrorExplain e={explainError('router', routerSrc.error)} />
      ) : isRouter && routerSrc?.online && routerSrc.logged_in === false && routerSrc.error && conn !== 'demo' ? (
        <ErrorExplain e={explainError('router', routerSrc.error)} />
      ) : null}
      {selected && !selected.radio && selected.radio_note && (!isPhone || !p) && !(isRouter && t) ? (
        <p className="connnote muted">
          No radio readings for <strong>{selected.name}</strong>: {selected.radio_note}.
        </p>
      ) : null}

      <section className="rings" aria-label="Signal gauges">
        <MetricRing metric="rsrp" value={r.rsrp} isStaleFn={staleFn} />
        <MetricRing metric="sinr" value={r.sinr} isStaleFn={staleFn} />
        <MetricRing metric="rsrq" value={r.rsrq} isStaleFn={staleFn} />
      </section>

      {isRouter ? (
        <section className="rates" aria-label="Throughput now">
          <div className="rate rate--down">
            <span className="rate__k">Down</span>
            <span className="rate__v">
              <Tween value={mbps(t?.rx_bytes_per_s)} digits={2} /> <small>Mbit/s</small>
            </span>
          </div>
          <div className="rate rate--up">
            <span className="rate__k">Up</span>
            <span className="rate__v">
              <Tween value={mbps(t?.tx_bytes_per_s)} digits={2} /> <small>Mbit/s</small>
            </span>
          </div>
          <div className="rate">
            <span className="rate__k">Session</span>
            <span className="rate__v num">{fmtDuration(t?.session_secs)}</span>
          </div>
          <div className="rate">
            <span className="rate__k">Session data</span>
            <span className="rate__v num">{fmtBytes(isNum(t?.session_rx_bytes) || isNum(t?.session_tx_bytes) ? (t?.session_rx_bytes ?? 0) + (t?.session_tx_bytes ?? 0) : null)}</span>
          </div>
        </section>
      ) : selected ? (
        <p className="connnote muted small">
          Live throughput is only known for the router's connection (the router reports it). Ping for this connection:{' '}
          {isNum(selected.ping?.ping_ms) ? `${fmt(selected.ping!.ping_ms)} ms (last Game Boost run)` : 'run Game Boost to measure it'}.
        </p>
      ) : null}

      <ConnectionsStrip list={list} best={conns?.best ?? null} t={t} p={p} onPick={pick} />

      <div className="signal-grid">
        <div className="signal-grid__charts">
          <div className="charts-head">
            <h2 className="section-title">Timeline{isPhone ? ' · phone (since this page opened)' : ''}</h2>
            <Segmented label="Chart window" value={win} options={WINDOWS} onChange={setWindow} />
          </div>
          <ChartPanel metric="rsrp" value={r.rsrp} phone={isPhone} />
          <ChartPanel metric="sinr" value={r.sinr} phone={isPhone} />
          <ChartPanel metric="rsrq" value={r.rsrq} phone={isPhone} />
          {isRouter ? (
            <Panel className="chartpanel" kicker="Router WAN" title="Throughput">
              <ThroughputChart />
            </Panel>
          ) : null}
        </div>
        <aside className="signal-grid__side">
          {isPhone ? <PhoneTowerCard p={p} /> : <TowerCard t={isRouter ? t : null} />}
          {isRouter ? <HandoverCards /> : null}
        </aside>
      </div>
    </div>
  );
}
