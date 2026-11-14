// Formatting, grading and derived values. Everything tolerates null/undefined.

export type Grade = 'excellent' | 'good' | 'fair' | 'poor' | 'none';
export type Metric = 'rsrp' | 'rsrq' | 'sinr';

export const DASH = '—';

export const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

/** Grade thresholds: [excellent, good, fair]; below fair is poor. */
export const THRESHOLDS: Record<Metric, [number, number, number]> = {
  rsrp: [-80, -90, -100],
  rsrq: [-10, -15, -20],
  sinr: [20, 13, 0],
};

export const RANGES: Record<Metric, [number, number]> = {
  rsrp: [-120, -70],
  sinr: [-5, 30],
  rsrq: [-20, -3],
};

export const UNITS: Record<Metric, string> = { rsrp: 'dBm', rsrq: 'dB', sinr: 'dB' };
export const LABELS: Record<Metric, string> = {
  rsrp: 'RSRP',
  rsrq: 'RSRQ',
  sinr: 'SINR',
};
export const DESCRIPTIONS: Record<Metric, string> = {
  rsrp: 'Reference signal received power',
  rsrq: 'Reference signal received quality',
  sinr: 'Signal to interference plus noise',
};

export function grade(metric: Metric, v: number | null | undefined): Grade {
  if (!isNum(v)) return 'none';
  const [e, g, f] = THRESHOLDS[metric];
  if (v >= e) return 'excellent';
  if (v >= g) return 'good';
  if (v >= f) return 'fair';
  return 'poor';
}

/** RGB triplets for each grade; mirrored in CSS custom properties. */
export const GRADE_RGB: Record<Grade, [number, number, number]> = {
  excellent: [94, 242, 170],
  good: [64, 214, 240],
  fair: [255, 184, 77],
  poor: [255, 84, 104],
  none: [110, 124, 138],
};

export const gradeColor = (g: Grade, a = 1): string => {
  const [r, gg, b] = GRADE_RGB[g];
  return `rgba(${r},${gg},${b},${a})`;
};

// ---------------------------------------------------------------- continuous quality colour
// One smooth scale for every dial, chart and meter: 0 = poor (red) … 1 = excellent (green).
// The grade edges sit at 0.30 (poor→fair), 0.60 (fair→good) and 0.85 (good→excellent), so the
// colour always agrees with the grade word. There is no colour at all without a reading.
const SCORE_STOPS: [number, [number, number, number]][] = [
  [0.0, [255, 62, 78]], // red
  [0.3, [255, 122, 56]], // orange-red (fair starts)
  [0.45, [255, 184, 60]], // orange
  [0.6, [247, 226, 66]], // yellow (good starts)
  [0.75, [170, 232, 84]], // yellow-green
  [0.85, [92, 232, 128]], // green (excellent starts)
  [1.0, [46, 240, 150]], // bright green
];

export function scoreRGB(score: number): [number, number, number] {
  const s = Math.max(0, Math.min(1, score));
  for (let i = 0; i < SCORE_STOPS.length - 1; i++) {
    const [a, ca] = SCORE_STOPS[i];
    const [b, cb] = SCORE_STOPS[i + 1];
    if (s <= b) {
      const t = b === a ? 0 : (s - a) / (b - a);
      return [ca[0] + (cb[0] - ca[0]) * t, ca[1] + (cb[1] - ca[1]) * t, ca[2] + (cb[2] - ca[2]) * t];
    }
  }
  return SCORE_STOPS[SCORE_STOPS.length - 1][1];
}

export const scoreColor = (score: number, a = 1): string => {
  const [r, g, b] = scoreRGB(score);
  return `rgba(${Math.round(r)},${Math.round(g)},${Math.round(b)},${a})`;
};

const GRADE_RANK: Record<Grade, number> = { none: -1, poor: 0, fair: 1, good: 2, excellent: 3 };

/**
 * Builds value → score (0..1) for any grading function over a value range, whichever way round
 * it works (higher-is-better like signal, or lower-is-better like ping). The grade edges are
 * found by sampling the grading function, so the colour can never disagree with the grade.
 */
export function makeScorer(
  gradeOf: (v: number | null | undefined) => Grade,
  range: [number, number],
): (v: number) => number {
  const [a, b] = range;
  const N = 96;
  const at = (i: number) => a + ((b - a) * i) / N;
  const forward = GRADE_RANK[gradeOf(b)] >= GRADE_RANK[gradeOf(a)];
  const worst = forward ? a : b;
  const best = forward ? b : a;
  const edge = (rank: number, fallback: number): number => {
    let prev = worst;
    for (let k = 0; k <= N; k++) {
      const v = forward ? at(k) : at(N - k);
      if (GRADE_RANK[gradeOf(v)] >= rank) return k === 0 ? v : (prev + v) / 2;
      prev = v;
    }
    return fallback;
  };
  const e1 = edge(1, worst);
  const e2 = edge(2, e1);
  const e3 = edge(3, e2);
  const pts: [number, number][] = [
    [worst, 0],
    [e1, 0.3],
    [e2, 0.6],
    [e3, 0.85],
    [best, 1],
  ];
  return (v: number) => {
    const t = best === worst ? 1 : (v - worst) / (best - worst);
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    for (let i = 0; i < pts.length - 1; i++) {
      const [v0, s0] = pts[i];
      const [v1, s1] = pts[i + 1];
      if (v0 === v1) continue;
      if (v >= Math.min(v0, v1) && v <= Math.max(v0, v1)) return s0 + ((v - v0) / (v1 - v0)) * (s1 - s0);
    }
    return t;
  };
}

export const GRADE_LABEL: Record<Grade, string> = {
  excellent: 'Excellent',
  good: 'Good',
  fair: 'Fair',
  poor: 'Poor',
  none: 'No data',
};

export const BAND_MHZ: Record<number, string> = {
  1: '2100 MHz',
  3: '1800 MHz',
  5: '850 MHz',
  7: '2600 MHz',
  8: '900 MHz',
  20: '800 MHz',
  28: '700 MHz',
  38: '2600 MHz TDD',
  40: '2300 MHz TDD',
  41: '2500 MHz TDD',
};

export function bandLabel(band: number | null | undefined): string {
  if (!isNum(band)) return DASH;
  const mhz = BAND_MHZ[band];
  return mhz ? `B${band} · ${mhz}` : `B${band}`;
}

export const enb = (cell: number | null | undefined): number | null =>
  isNum(cell) ? Math.floor(cell / 256) : null;
export const sector = (cell: number | null | undefined): number | null =>
  isNum(cell) ? cell % 256 : null;

export const mbps = (bps: number | null | undefined): number | null =>
  isNum(bps) ? (bps * 8) / 1e6 : null;

export function fmt(v: number | null | undefined, digits = 0): string {
  if (!isNum(v)) return DASH;
  return v.toFixed(digits).replace('-', '−');
}

export function fmtSigned(v: number | null | undefined, digits = 0): string {
  if (!isNum(v)) return DASH;
  const s = Math.abs(v).toFixed(digits);
  if (Number(s) === 0) return `±${s}`;
  return v > 0 ? `+${s}` : `−${s}`;
}

export const str = (v: unknown): string =>
  v === null || v === undefined || v === '' ? DASH : String(v);

export function fmtBytes(v: number | null | undefined): string {
  if (!isNum(v)) return DASH;
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let x = v;
  let i = 0;
  while (x >= 1000 && i < units.length - 1) {
    x /= 1000;
    i++;
  }
  return `${x.toFixed(x >= 100 || i === 0 ? 0 : x >= 10 ? 1 : 2)} ${units[i]}`;
}

export function fmtRate(bps: number | null | undefined): string {
  const m = mbps(bps);
  if (m === null) return DASH;
  if (m < 1) return `${(m * 1000).toFixed(0)} kbit/s`;
  return `${m.toFixed(m >= 100 ? 0 : m >= 10 ? 1 : 2)} Mbit/s`;
}

export function fmtDuration(secs: number | null | undefined): string {
  if (!isNum(secs) || secs < 0) return DASH;
  const s = Math.floor(secs);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = s % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  if (d > 0) return `${d}d ${pad(h)}h ${pad(m)}m`;
  if (h > 0) return `${h}h ${pad(m)}m ${pad(ss)}s`;
  return `${m}m ${pad(ss)}s`;
}

export function fmtTime(ms: number | null | undefined, withSeconds = true): string {
  if (!isNum(ms)) return DASH;
  const d = new Date(ms);
  return d.toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: withSeconds ? '2-digit' : undefined,
    hour12: false,
  });
}

export function fmtDateTime(ms: number | null | undefined): string {
  if (!isNum(ms)) return DASH;
  const d = new Date(ms);
  return `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${fmtTime(ms)}`;
}

export function fmtAgo(ms: number | null | undefined, now = Date.now()): string {
  if (!isNum(ms)) return DASH;
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 5) return 'just now';
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ${m % 60}m ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Normalised 0..1 position of value in range. */
export const norm = (v: number, [lo, hi]: [number, number]) => clamp((v - lo) / (hi - lo), 0, 1);

export const prefersReducedMotion = (): boolean =>
  typeof window !== 'undefined' &&
  !!window.matchMedia &&
  window.matchMedia('(prefers-reduced-motion: reduce)').matches;
