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
  token: string;
  demo: boolean;
  sound: boolean;
  accentHue: number;
}

export const defaultPrefs: Prefs = {
  baseUrl: '',
  token: '',
  demo: false,
  sound: false,
  accentHue: 168,
};

export function loadPrefs(): Prefs {
  return {
    baseUrl: load('baseUrl', defaultPrefs.baseUrl),
    token: load('token', defaultPrefs.token),
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
}
