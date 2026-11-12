import { useEffect, useMemo } from 'react';
import { GlowRing } from '../components/GlowRing';
import { SeriesChart } from '../components/SeriesChart';
import { ThroughputChart } from '../components/ThroughputChart';
import { Badge, ErrorExplain, Icon, KV, Panel, Segmented, Tween } from '../components/ui';
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
import type { Telemetry } from '../lib/types';

const telemetryStale = () => {
  const s = useStore.getState();
  return isStale(s.telemetryAt, STALE_MS.telemetry);
};

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
    <Panel title="Serving cell" kicker="Tower" className="tower">
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

function ChartPanel({ metric, value }: { metric: Metric; value: number | null | undefined }) {
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
      <SeriesChart metric={metric} />
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

export function SignalPage() {
  const t = useStore((s) => s.telemetry);
  const win = useStore((s) => s.window);
  const setWindow = useStore((s) => s.setWindow);
  const setAim = useStore((s) => s.setAim);
  const conn = useStore((s) => s.conn);
  const telemetryAt = useStore((s) => s.telemetryAt);
  const routerSrc = useStore((s) => s.sources?.router);
  const now = useNow(1000);
  const stale = isStale(telemetryAt, STALE_MS.telemetry, now);

  useEffect(() => {
    void ensureHistory();
  }, [win, conn]);

  return (
    <div className="page page--signal">
      <header className="pagehead">
        <div>
          <div className="pagehead__kicker">
            {t?.network_type ? `${t.network_type} · ` : ''}
            {bandLabel(t?.band)} · PCI {fmt(t?.pci)}
          </div>
          <h1 className="pagehead__title">Signal</h1>
        </div>
        <div className="pagehead__actions">
          {stale && conn !== 'connecting' ? (
            <Badge tone="stale">{telemetryAt ? `no reading for ${fmtAgo(telemetryAt, now).replace(' ago', '')}` : 'waiting for first reading'}</Badge>
          ) : null}
          <button type="button" className="btn btn--accent" onClick={() => setAim(true)}>
            <Icon name="aim" size={18} /> Aim mode
          </button>
        </div>
      </header>

      {routerSrc && !routerSrc.online && routerSrc.error && conn !== 'demo' ? (
        <ErrorExplain e={explainError('router', routerSrc.error)} />
      ) : routerSrc?.online && routerSrc.logged_in === false && routerSrc.error && conn !== 'demo' ? (
        <ErrorExplain e={explainError('router', routerSrc.error)} />
      ) : null}

      <section className="rings" aria-label="Signal gauges">
        <MetricRing metric="rsrp" value={t?.rsrp} />
        <MetricRing metric="sinr" value={t?.sinr} />
        <MetricRing metric="rsrq" value={t?.rsrq} />
      </section>

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

      <div className="signal-grid">
        <div className="signal-grid__charts">
          <div className="charts-head">
            <h2 className="section-title">Timeline</h2>
            <Segmented label="Chart window" value={win} options={WINDOWS} onChange={setWindow} />
          </div>
          <ChartPanel metric="rsrp" value={t?.rsrp} />
          <ChartPanel metric="sinr" value={t?.sinr} />
          <ChartPanel metric="rsrq" value={t?.rsrq} />
          <Panel className="chartpanel" kicker="Router WAN" title="Throughput">
            <ThroughputChart />
          </Panel>
        </div>
        <aside className="signal-grid__side">
          <TowerCard t={t} />
          <HandoverCards />
        </aside>
      </div>
    </div>
  );
}
