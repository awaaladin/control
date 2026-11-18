// Signal readings for ANY kind of connection (Wi-Fi, cable, USB tether, 4G router, a phone's
// mobile signal). The backend (crates/rhcsd/src/connections.rs) sends each reading with its own
// dial range, grade edges and direction, so no per-kind code is needed here.

import { isNum, type Grade } from './format';

export interface Metric {
  /** rsrp | sinr | rsrq | wifi_signal | wifi_quality | link_speed (any key is drawn the same way) */
  key: string;
  label: string;
  unit: string;
  /** null = no reading right now: drawn grey, never coloured. */
  value: number | null;
  min: number;
  max: number;
  /** [excellent, good, fair]; below fair is poor. */
  thresholds: [number, number, number];
  higher_is_better: boolean;
  note?: string | null;
}

/** A /api/connections entry as the backend now sends it (a superset of Connection in connections.ts). */
export interface SignalConnection {
  id: string;
  name: string;
  /** router | phone | other | bluetooth */
  kind: string;
  medium?: string | null;
  in_use?: boolean | null;
  radio_note?: string | null;
  metrics?: Metric[];
  /** One plain line: SSID, standard, channel, band, rates / adapter and speed / device state. */
  detail?: string | null;
  /** Why there is no reading (Bluetooth) or why the readings are grey. */
  note?: string | null;
}

/** Grade of a value against the metric's own edges. */
export function gradeOfMetric(m: Metric, v: number | null | undefined): Grade {
  if (!isNum(v)) return 'none';
  const [e, g, f] = m.thresholds;
  if (m.higher_is_better) return v >= e ? 'excellent' : v >= g ? 'good' : v >= f ? 'fair' : 'poor';
  return v <= e ? 'excellent' : v <= g ? 'good' : v <= f ? 'fair' : 'poor';
}

/** Dial range, worst end first (what makeScorer / GlowRing expect for a lower-is-better metric). */
export function rangeOfMetric(m: Metric): [number, number] {
  return m.higher_is_better ? [m.min, m.max] : [m.max, m.min];
}

/** Digits worth showing: whole numbers for dBm, dB, % and Mbit/s. */
export const metricDigits = (m: Metric): number => (m.unit === 'dB' || m.unit === 'dBm' || m.unit === '%' || m.unit === 'Mbit/s' ? 0 : 1);

/** Rolling history length kept in the browser per metric. */
export const HISTORY_POINTS = 60;
