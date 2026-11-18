import { useEffect, useRef, useState, type ReactElement } from 'react';
import { GlowRing } from './GlowRing';
import { GRADE_RGB, isNum, makeScorer, scoreColor } from '../lib/format';
import { gradeOfMetric, HISTORY_POINTS, metricDigits, rangeOfMetric, type Metric, type SignalConnection } from '../lib/metrics';
import '../styles/extras.css';

/** Tiny rolling line of one metric. Each stretch is coloured by how good that reading was;
 * a missing reading is a gap and grey, never a colour. */
function MetricSpark({ m, values }: { m: Metric; values: (number | null)[] }) {
  const w = 150;
  const h = 26;
  const nums = values.filter(isNum);
  if (nums.length < 2) return <div className="condial__spark condial__spark--empty" aria-hidden="true" />;
  const [a, b] = rangeOfMetric(m);
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  const scorer = makeScorer((v) => gradeOfMetric(m, v), rangeOfMetric(m));
  const x = (i: number) => (i / (HISTORY_POINTS - 1)) * w + (w - ((values.length - 1) / (HISTORY_POINTS - 1)) * w);
  const y = (v: number) => h - 2 - ((Math.min(hi, Math.max(lo, v)) - lo) / (hi - lo || 1)) * (h - 4);
  const segs: ReactElement[] = [];
  for (let i = 1; i < values.length; i++) {
    const p = values[i - 1];
    const q = values[i];
    if (!isNum(p) || !isNum(q)) continue;
    segs.push(
      <line
        key={i}
        x1={x(i - 1)}
        y1={y(p)}
        x2={x(i)}
        y2={y(q)}
        stroke={scoreColor(scorer((p + q) / 2))}
        strokeWidth={2}
        strokeLinecap="round"
      />,
    );
  }
  const last = values[values.length - 1];
  return (
    <svg className="condial__spark" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" role="img" aria-label={`${m.label} over the last minutes`}>
      {segs}
      {isNum(last) ? <circle cx={x(values.length - 1)} cy={y(last)} r={2.4} fill={scoreColor(scorer(last))} /> : null}
    </svg>
  );
}

/**
 * Dials for one connection of any kind: up to three large rings from its `metrics` (any key),
 * the rest smaller, a rolling in-browser history line per metric, the `detail` line and any
 * `note`. A metric with no value is grey. With no metrics at all it says so and why.
 *
 *   <ConnectionDials conn={connection} />
 */
export function ConnectionDials({ conn, className }: { conn: SignalConnection; className?: string }) {
  const metrics = conn.metrics ?? [];
  const [hist, setHist] = useState<Record<string, (number | null)[]>>({});
  const forId = useRef(conn.id);

  // Remember the last minutes of every metric while the page is open (per connection, per browser tab).
  useEffect(() => {
    const reset = forId.current !== conn.id;
    forId.current = conn.id;
    setHist((old) => {
      const base = reset ? {} : old;
      const next: Record<string, (number | null)[]> = {};
      for (const m of metrics) {
        next[m.key] = [...(base[m.key] ?? []), isNum(m.value) ? m.value : null].slice(-HISTORY_POINTS);
      }
      return next;
    });
    // `metrics` is a new array on every poll, which is exactly when a point should be added.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn.metrics, conn.id]);

  const notes = [conn.note, ...metrics.map((m) => m.note)].filter((n, i, all): n is string => !!n && all.indexOf(n) === i);

  if (metrics.length === 0) {
    return (
      <div className={`condials condials--none ${className ?? ''}`}>
        {conn.detail ? <div className="condials__detail">{conn.detail}</div> : null}
        <p className="condials__empty">{conn.note ?? conn.radio_note ?? 'No signal reading is available for this kind of connection.'}</p>
        <p className="muted small">
          What you can do: use the ping from Game Boost for this connection, or switch to a Wi-Fi, cable or 4G link, which all have signal readings.
        </p>
      </div>
    );
  }

  return (
    <div className={`condials ${className ?? ''}`}>
      {conn.detail ? <div className="condials__detail">{conn.detail}</div> : null}
      <div className="condials__rings">
        {metrics.map((m, i) => (
          <div key={m.key} className={`condial ${i >= 3 ? 'condial--minor' : ''}`}>
            <GlowRing
              label={m.label}
              value={m.value}
              range={rangeOfMetric(m)}
              unit={m.unit}
              digits={metricDigits(m)}
              gradeOf={(v) => gradeOfMetric(m, v)}
              thresholds={m.thresholds}
              isStale={() => false}
              size={i >= 3 ? 'sm' : 'md'}
            />
            <MetricSpark m={m} values={hist[m.key] ?? []} />
            {!isNum(m.value) ? (
              <div className="condial__grey" style={{ color: `rgb(${GRADE_RGB.none.join(',')})` }}>
                no reading
              </div>
            ) : null}
          </div>
        ))}
      </div>
      {notes.map((n) => (
        <p key={n} className="muted small condials__note">
          {n}
        </p>
      ))}
      {conn.radio_note && !notes.includes(conn.radio_note) ? <p className="muted small condials__note">{conn.radio_note}</p> : null}
    </div>
  );
}
