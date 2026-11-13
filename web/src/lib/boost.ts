// Game Boost, Towers and hourly-signal API calls. Kept apart from rhcs.ts so the
// connection logic there stays untouched; this file only reads the current
// backend URL and token from the store.

import { useEffect } from 'react';
import { create } from 'zustand';
import { toast, useStore } from './store';

// ---------------------------------------------------------------- HTTP helpers

export interface Answer<T> {
  ok: boolean;
  status: number;
  data: T | null;
  message: string;
}

async function call<T>(path: string, init?: RequestInit, timeoutMs = 8000): Promise<Answer<T>> {
  const base = useStore.getState().baseUrl;
  if (!base || useStore.getState().conn === 'demo') return { ok: false, status: 0, data: null, message: 'not connected' };
  const ctl = new AbortController();
  const timer = window.setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const r = await fetch(base + path, { cache: 'no-store', ...init, signal: ctl.signal });
    let data: T | null = null;
    if ((r.headers.get('content-type') ?? '').includes('json')) {
      try {
        data = (await r.json()) as T;
      } catch {
        data = null;
      }
    }
    const msg = (data as { message?: string } | null)?.message;
    return { ok: r.ok, status: r.status, data, message: msg ?? (r.ok ? 'done' : `HTTP ${r.status}`) };
  } catch (e) {
    const message = e instanceof DOMException && e.name === 'AbortError' ? 'request timed out' : 'backend unreachable';
    return { ok: false, status: 0, data: null, message };
  } finally {
    window.clearTimeout(timer);
  }
}

export const getJson = <T,>(path: string) => call<T>(path);

export async function postJson<T>(path: string, body: unknown): Promise<Answer<T>> {
  const headers: Record<string, string> = { 'content-type': 'application/json' };
  const token = useStore.getState().prefs.token.trim();
  if (token) headers['x-rhcs-token'] = token;
  const a = await call<T>(path, { method: 'POST', headers, body: JSON.stringify(body ?? {}) });
  if (a.status === 401) {
    toast(
      'error',
      'Not authorized',
      useStore.getState().health?.auth_required
        ? 'rhcsd requires a token. Set it in Control → Settings.'
        : 'No token is set on rhcsd, so this is only allowed from the computer it runs on.',
      8000,
    );
  }
  return a;
}

// ---------------------------------------------------------------- Game Boost

export type LinkKind = 'router' | 'phone_usb' | 'phone_hotspot' | 'other';

export interface BoostTarget {
  target: string;
  sent: number;
  received: number;
  refused: number;
  median_ms: number | null;
  jitter_ms: number | null;
}

export interface BoostLink {
  id: string;
  name: string;
  kind: LinkKind;
  ssid?: string | null;
  if_index?: number;
  medium: string;
  adapter: string;
  description: string;
  local_ip: string;
  gateway: string | null;
  metric: number | null;
  is_default: boolean;
  sent: number;
  received: number;
  loss_pct: number | null;
  ping_ms: number | null;
  jitter_ms: number | null;
  best_ms: number | null;
  p90_ms: number | null;
  targets: BoostTarget[];
  radio: { source: 'router' | 'phone'; rsrp: number | null; sinr: number | null } | null;
  score: number | null;
  grade: 'excellent' | 'good' | 'fair' | 'poor' | 'none';
  verdict: string;
  recent: (number | null)[];
  error: string | null;
}

export interface BoostRecommendation {
  best: string;
  best_name: string;
  headline: string;
  explanation: string;
  already_default: boolean;
  close_call: boolean;
  commands: string[];
}

export interface BoostState {
  phase: 'idle' | 'measuring' | 'done' | 'error';
  watch: boolean;
  running?: boolean;
  started: number | null;
  finished: number | null;
  done: number;
  total: number;
  probes: number;
  interval_ms: number;
  targets: string[];
  links: BoostLink[];
  recommendation: BoostRecommendation | null;
  error: string | null;
  /** rhcsd runs as administrator (switching possible). */
  admin?: boolean;
  auto?: AutoView;
  switch?: SwitchStatus;
}

export interface AutoDecision {
  ts: number;
  action: 'stay' | 'wait' | 'hold' | 'switch' | 'error';
  message: string;
  to: string | null;
  to_alias: string | null;
}

export interface AutoView {
  enabled: boolean;
  last: AutoDecision | null;
  last_switch: number | null;
}

export interface SwitchStatus {
  admin: boolean;
  supported: boolean;
  admin_help: string | null;
  switched: boolean;
  preferred: string | null;
  touched: { alias: string; original: { automatic: boolean; metric: number } }[];
  last_switch: number | null;
  last_message: string | null;
}

export interface HourBucket {
  hour: number;
  samples: number;
  avg_sinr: number | null;
  avg_rsrp: number | null;
  avg_rx_bps: number | null;
}

export interface Hourly {
  hours: HourBucket[];
  covered_h: number;
}

interface BoostStore {
  state: BoostState | null;
  /** Backend answered but has no Boost endpoint (older rhcsd). */
  unsupported: boolean;
  at: number | null;
  hourly: Hourly | null;
}

export const useBoost = create<BoostStore>()(() => ({ state: null, unsupported: false, at: null, hourly: null }));

export async function refreshBoost(): Promise<void> {
  const a = await getJson<BoostState>('/api/boost');
  if (a.ok && a.data) useBoost.setState({ state: a.data, unsupported: false, at: Date.now() });
  else if (a.status === 404) useBoost.setState({ unsupported: true });
}

export async function refreshHourly(): Promise<void> {
  const a = await getJson<Hourly>('/api/signal/hourly');
  if (a.ok && a.data) useBoost.setState({ hourly: a.data });
}

export async function runBoost(watch: boolean): Promise<void> {
  const a = await postJson<{ ok: boolean; message: string }>('/api/boost/run', { watch });
  if (a.ok) {
    useBoost.setState((s) => ({ state: s.state ? { ...s.state, phase: 'measuring', running: true, watch } : s.state }));
  } else if (a.status !== 401) toast('error', 'Could not start Game Boost', a.message);
  void refreshBoost();
}

export async function stopBoostWatch(): Promise<void> {
  const a = await postJson('/api/boost/stop', {});
  if (!a.ok && a.status !== 401) toast('error', 'Could not stop watch mode', a.message);
  void refreshBoost();
}

export async function setAutoSwitch(enabled: boolean): Promise<void> {
  const a = await postJson<{ ok: boolean; message: string }>('/api/boost/auto', { enabled });
  if (a.ok) toast('ok', enabled ? 'Auto-switch on' : 'Auto-switch off', a.message);
  else if (a.status !== 401) toast('error', 'Could not change auto-switch', a.message, 12000);
  void refreshBoost();
}

export async function switchLink(alias: string): Promise<void> {
  const a = await postJson<{ ok: boolean; message: string }>('/api/links/switch', { alias });
  if (a.ok) toast('ok', 'Switched', a.message, 8000);
  else if (a.status !== 401) toast('error', 'Could not switch', a.message, 12000);
  void refreshBoost();
}

export async function restoreLinks(): Promise<void> {
  const a = await postJson<{ ok: boolean; message: string }>('/api/links/restore', {});
  if (a.ok) toast('ok', 'Restored', a.message, 8000);
  else if (a.status !== 401) toast('error', 'Could not restore', a.message, 12000);
  void refreshBoost();
}

/** Polls /api/boost while mounted: fast while measuring, slow otherwise. */
export function useBoostPolling(): void {
  const baseUrl = useStore((s) => s.baseUrl);
  const conn = useStore((s) => s.conn);
  useEffect(() => {
    if (!baseUrl || conn === 'demo') return;
    let alive = true;
    let timer = 0;
    const loop = async () => {
      await refreshBoost();
      if (!alive) return;
      const st = useBoost.getState().state;
      const busy = st?.running || st?.phase === 'measuring' || st?.watch;
      timer = window.setTimeout(loop, busy ? 500 : st?.auto?.enabled ? 2000 : 4000);
    };
    void loop();
    void refreshHourly();
    const hourlyTimer = window.setInterval(() => void refreshHourly(), 5 * 60_000);
    return () => {
      alive = false;
      window.clearTimeout(timer);
      window.clearInterval(hourlyTimer);
    };
  }, [baseUrl, conn]);
}
