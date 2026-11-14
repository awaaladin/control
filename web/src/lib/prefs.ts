// Small localStorage helpers. Storage can be unavailable (private windows,
// blocked site data), so every access is guarded and falls back gracefully.

const PREFIX = 'pulse.';

export function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    if (raw === null) return fallback;
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

export function save(key: string, value: unknown): void {
  try {
    if (value === undefined || value === null || value === '') localStorage.removeItem(PREFIX + key);
    else localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    /* storage unavailable: preference lives for this session only */
  }
}

export interface Prefs {
  /** Explicit backend URL; empty means auto-detect. */
  baseUrl: string;
  /** Backends to look for automatically (laptop, phone hub, another place…). */
  hubs: string[];
  /** The backend that answered last time; tried first next time. */
  lastGood: string;
  /** Fallback access token, used for any backend without its own. */
  token: string;
  /** Access token per backend URL (x-rhcs-token). */
  tokens: Record<string, string>;
  demo: boolean;
  sound: boolean;
  accentHue: number;
}

export const defaultPrefs: Prefs = {
  baseUrl: '',
  hubs: [],
  lastGood: '',
  token: '',
  tokens: {},
  demo: false,
  sound: false,
  accentHue: 168,
};

export function loadPrefs(): Prefs {
  return {
    baseUrl: load('baseUrl', defaultPrefs.baseUrl),
    hubs: (() => {
      const v = load<unknown>('hubs', []);
      return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
    })(),
    lastGood: load('lastGood', defaultPrefs.lastGood),
    token: load('token', defaultPrefs.token),
    tokens: (() => {
      const v = load<unknown>('tokens', {});
      if (!v || typeof v !== 'object' || Array.isArray(v)) return {};
      return Object.fromEntries(Object.entries(v as Record<string, unknown>).filter(([, t]) => typeof t === 'string')) as Record<string, string>;
    })(),
    demo: load('demo', defaultPrefs.demo),
    sound: load('sound', defaultPrefs.sound),
    accentHue: load('accentHue', defaultPrefs.accentHue),
  };
}

export interface MarkedSpot {
  id: string;
  label: string;
  ts: number;
  sinr: number | null;
  rsrp: number | null;
  rsrq: number | null;
  pci: number | null;
  cell_id: number | null;
  band: number | null;
  /** Recorded automatically by Aim mode (not marked by hand). */
  auto?: boolean;
  /** Automatic place the phone was at when the spot was marked (if known). */
  place_id?: number | null;
}
