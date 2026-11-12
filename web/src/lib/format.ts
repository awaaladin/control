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
