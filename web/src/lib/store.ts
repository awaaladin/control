import { create } from 'zustand';
import type { DeviceView, Health, PhoneStatus, RhcsEvent, Settings, Sources, Telemetry } from './types';
import { loadPrefs, save, type Prefs } from './prefs';

export type Conn = 'connecting' | 'live' | 'reconnecting' | 'offline' | 'demo';
export type WindowKey = '1m' | '10m' | '1h' | '24h';
export const WINDOW_MS: Record<WindowKey, number> = {
  '1m': 60_000,
  '10m': 600_000,
  '1h': 3_600_000,
  '24h': 86_400_000,
};

export interface Toast {
  id: number;
  tone: 'ok' | 'error' | 'info' | 'warn';
  title: string;
  body?: string;
}

export interface ConfirmRequest {
  title: string;
  body: string;
  confirmLabel?: string;
  danger?: boolean;
  resolve: (ok: boolean) => void;
}

export const EVENTS_MAX = 500;

export interface PulseState {
  prefs: Prefs;
  baseUrl: string;
  conn: Conn;
  demoReason: 'user' | 'fallback' | null;
  /** Startup could not reach the backend; show the "use demo data" prompt. */
  unreachable: boolean;
  /** In user-chosen demo mode, the backend answered /api/health. */
  liveAvailable: boolean;
  health: Health | null;
  sources: Sources | null;
  telemetry: Telemetry | null;
  telemetryAt: number | null;
  phone: PhoneStatus | null;
  phoneAt: number | null;
  devices: DeviceView[];
  devicesAt: number | null;
  events: RhcsEvent[];
  settings: Settings | null;
  toasts: Toast[];
  confirmReq: ConfirmRequest | null;
  window: WindowKey;
  aim: boolean;

  setPrefs: (p: Partial<Prefs>) => void;
  setWindow: (w: WindowKey) => void;
  setAim: (on: boolean) => void;
  pushEvents: (evs: RhcsEvent[]) => void;
  resetData: () => void;
}

const eventKey = (e: RhcsEvent) => `${e.ts}|${e.kind}|${e.message ?? ''}`;

export const useStore = create<PulseState>()((set, get) => ({
  prefs: loadPrefs(),
  baseUrl: '',
  conn: 'connecting',
  demoReason: null,
  unreachable: false,
  liveAvailable: false,
  health: null,
  sources: null,
  telemetry: null,
  telemetryAt: null,
  phone: null,
  phoneAt: null,
  devices: [],
  devicesAt: null,
  events: [],
  settings: null,
  toasts: [],
  confirmReq: null,
  window: '10m',
  aim: false,

  setPrefs: (p) => {
    const prefs = { ...get().prefs, ...p };
    for (const [k, v] of Object.entries(p)) save(k, v);
    set({ prefs });
  },
  setWindow: (w) => set({ window: w }),
  setAim: (on) => set({ aim: on }),
  pushEvents: (evs) => {
    if (!evs.length) return;
    const cur = get().events;
    const seen = new Set(cur.map(eventKey));
    const add = evs.filter((e) => e && typeof e.ts === 'number' && !seen.has(eventKey(e)));
    if (!add.length) return;
    const next = cur.concat(add);
    next.sort((a, b) => a.ts - b.ts);
    set({ events: next.length > EVENTS_MAX ? next.slice(next.length - EVENTS_MAX) : next });
  },
  resetData: () =>
    set({
      health: null,
      sources: null,
      telemetry: null,
      telemetryAt: null,
      phone: null,
      phoneAt: null,
      devices: [],
      devicesAt: null,
      events: [],
      settings: null,
    }),
}));

let toastId = 0;
export function toast(tone: Toast['tone'], title: string, body?: string, ms = 5200): void {
  const id = ++toastId;
  useStore.setState((s) => ({ toasts: [...s.toasts.slice(-4), { id, tone, title, body }] }));
  window.setTimeout(() => dismissToast(id), ms);
}
export function dismissToast(id: number): void {
  useStore.setState((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
}

export function confirmAction(opts: Omit<ConfirmRequest, 'resolve'>): Promise<boolean> {
  return new Promise((resolve) => {
    const prev = useStore.getState().confirmReq;
    if (prev) prev.resolve(false);
    useStore.setState({
      confirmReq: {
        ...opts,
        resolve: (ok) => {
          useStore.setState({ confirmReq: null });
          resolve(ok);
        },
      },
    });
  });
}

/** Staleness thresholds (ms) per source. */
export const STALE_MS = { telemetry: 5000, phone: 15000 };

export function isStale(at: number | null, limit: number, now = Date.now()): boolean {
  return at === null || now - at > limit;
}
