// Typed rhcsd API client, WebSocket manager and live/demo orchestration.

import { phoneSamples, samples, sampleFromTelemetry } from './buffer';
import { useStore, toast, WINDOW_MS } from './store';
import type {
  ActionResult,
  DeviceView,
  Health,
  PhoneStatus,
  RhcsEvent,
  RouterAction,
  Sample,
  Settings,
  Sources,
  Telemetry,
  WsMessage,
} from './types';
import { DemoSim } from './demo';
import { chime } from './sound';

const DEFAULT_PORT = '7878';
const FALLBACK_URL = `http://localhost:${DEFAULT_PORT}`;

const get = () => useStore.getState();
const set = useStore.setState;

// ---------------------------------------------------------------- clock skew
// Samples carry the daemon's clock. If the dashboard runs on another device
// the clocks can disagree, so charts use an estimate of "server now".
let skew = 0;
let skewInit = false;
function noteServerTs(ts: number) {
  if (!Number.isFinite(ts)) return;
  const d = ts - Date.now();
  if (!skewInit) {
    skew = d;
    skewInit = true;
  } else skew = skew * 0.9 + d * 0.1;
}
export const serverNow = () => Date.now() + skew;

// ---------------------------------------------------------------- URL helpers
export function normalizeUrl(u: string): string {
  let s = u.trim();
  if (!s) return '';
  if (!/^https?:\/\//i.test(s)) s = `http://${s}`;
  return s.replace(/\/+$/, '');
}

function sameOrigin(): string {
  return location.origin;
}

function wsUrl(base: string): string {
  const u = new URL(base + '/ws');
  u.protocol = u.protocol === 'https:' ? 'wss:' : 'ws:';
  return u.toString();
}

async function fetchJson<T>(url: string, init?: RequestInit, timeoutMs = 6000): Promise<{ status: number; data: T | null }> {
  const ctl = new AbortController();
  const t = window.setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const r = await fetch(url, { ...init, signal: ctl.signal });
    let data: T | null = null;
    const ct = r.headers.get('content-type') ?? '';
    if (ct.includes('json')) {
      try {
        data = (await r.json()) as T;
      } catch {
        data = null;
      }
    }
    return { status: r.status, data };
  } finally {
    window.clearTimeout(t);
  }
}

async function probeHealth(base: string, timeoutMs = 3500): Promise<Health | null> {
  try {
    const { status, data } = await fetchJson<Health>(`${base}/api/health`, { cache: 'no-store' }, timeoutMs);
    if (status === 200 && data && typeof data === 'object' && Array.isArray(data.capabilities)) return data;
    return null;
  } catch {
    return null;
  }
}

export async function resolveBaseUrl(): Promise<string> {
  const pref = normalizeUrl(get().prefs.baseUrl);
  if (pref) return pref;
  if (location.port === DEFAULT_PORT) return sameOrigin();
  if (location.protocol.startsWith('http')) {
    const h = await probeHealth(sameOrigin(), 1500);
    if (h) return sameOrigin();
  }
  return FALLBACK_URL;
}

// ---------------------------------------------------------------- REST
export interface PostResult {
  ok: boolean;
  status: number;
  message: string;
  data: ActionResult | null;
}

async function apiGet<T>(path: string): Promise<T | null> {
  const base = get().baseUrl;
  if (!base) return null;
  try {
    const { status, data } = await fetchJson<T>(base + path, { cache: 'no-store' });
    return status === 200 ? data : null;
  } catch {
    return null;
  }
}

async function apiPost(path: string, body: unknown): Promise<PostResult> {
  if (get().conn === 'demo' && demo) return demo.post(path, body);
  const base = get().baseUrl;
  const headers: Record<string, string> = { 'content-type': 'application/json' };
  const token = get().prefs.token.trim();
  if (token) headers['x-rhcs-token'] = token;
  try {
    const { status, data } = await fetchJson<ActionResult>(base + path, {
      method: 'POST',
      headers,
      body: JSON.stringify(body ?? {}),
    });
    const message = data?.message ?? (status === 200 ? 'done' : `HTTP ${status}`);
    if (status === 401) {
      const authRequired = get().health?.auth_required;
      toast(
        'error',
        'Not authorized',
        authRequired
          ? `rhcsd requires a token. Set it in Control → Settings. (${message})`
          : `No token is configured on rhcsd, so control actions are only allowed from the machine it runs on. (${message})`,
        8000,
      );
    }
    return { ok: status === 200 && data?.ok !== false, status, message, data };
  } catch (e) {
    const message = e instanceof DOMException && e.name === 'AbortError' ? 'request timed out' : 'backend unreachable';
    return { ok: false, status: 0, message, data: null };
  }
}

export const api = {
  scan: () => apiPost('/api/scan', {}),
  routerAction: (action: RouterAction, value?: 'auto' | 'lte_only') =>
    apiPost('/api/router/action', value ? { action, value } : { action }),
  setInterval: (interval_s: number) => apiPost('/api/settings', { interval_s }),
};

// ---------------------------------------------------------------- message handling
function onStatus(s: Sources) {
  set({ sources: s });
}
function onTelemetry(ts: number, t: Telemetry) {
  noteServerTs(ts);
  set({ telemetry: t, telemetryAt: Date.now() });
  samples.push(sampleFromTelemetry(ts, t));
}
function onPhone(ts: number, p: PhoneStatus) {
  set({ phone: p, phoneAt: Date.now() });
  phoneSamples.push({
    ts,
    rsrp: p.rsrp,
    rsrq: p.rsrq,
    sinr: p.sinr,
    rx_bytes_per_s: null,
    tx_bytes_per_s: null,
    pci: p.pci,
    cell_id: p.cell_id,
    band: p.band,
  });
}
function onDevices(d: DeviceView[]) {
  set({ devices: Array.isArray(d) ? d : [], devicesAt: Date.now() });
}
function onEvent(e: RhcsEvent, live: boolean) {
  get().pushEvents([e]);
  if (live && get().prefs.sound) chime(e.kind);
}

export function dispatch(msg: WsMessage, live = true) {
  if (!msg || typeof msg !== 'object') return;
  switch (msg.type) {
    case 'status':
      onStatus(msg.data ?? {});
      break;
    case 'telemetry':
      if (msg.data) onTelemetry(msg.ts, msg.data);
      break;
    case 'phone':
      if (msg.data) onPhone(msg.ts, msg.data);
      break;
    case 'location':
      if (msg.data) set({ fix: msg.data, fixAt: Date.now() });
      break;
    case 'devices':
      onDevices(msg.data);
      break;
    case 'event':
      onEvent(msg, live);
      break;
    default:
      break;
  }
}

// ---------------------------------------------------------------- history
let historyInflight = false;
export async function ensureHistory(force = false): Promise<void> {
  const st = get();
  if (st.conn === 'demo' || !st.baseUrl || historyInflight) return;
  const windowMs = WINDOW_MS[st.window];
  const want = serverNow() - windowMs;
  if (!force && samples.coveredSince <= want + 5000) return;
  historyInflight = true;
  try {
    const minutes = Math.min(1440, Math.max(1, Math.ceil(windowMs / 60000)));
    const rows = await apiGet<Sample[]>(`/api/signal/history?minutes=${minutes}`);
    if (Array.isArray(rows) && get().conn !== 'demo') {
      samples.merge(rows);
      samples.coveredSince = Math.min(samples.coveredSince, want);
    }
  } finally {
    historyInflight = false;
  }
}

async function bootstrapRest() {
  const [events, settings, health] = await Promise.all([
    apiGet<RhcsEvent[]>('/api/events'),
    apiGet<Settings>('/api/settings'),
    probeHealth(get().baseUrl),
  ]);
  if (get().conn === 'demo') return;
  if (Array.isArray(events)) get().pushEvents(events);
  if (settings) set({ settings });
  if (health) set({ health, sources: health.sources ?? get().sources });
  await ensureHistory(true);
}

export async function refreshSettings() {
  const s = await apiGet<Settings>('/api/settings');
  if (s) set({ settings: s });
}

// ---------------------------------------------------------------- connection lifecycle
let ws: WebSocket | null = null;
let retry = 0;
let reconnectTimer = 0;
let healthTimer = 0;
let generation = 0;
let demo: DemoSim | null = null;

function clearTimers() {
  window.clearTimeout(reconnectTimer);
  window.clearInterval(healthTimer);
  reconnectTimer = 0;
  healthTimer = 0;
}

function closeWs() {
  if (ws) {
    ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null;
    try {
      ws.close();
    } catch {
      /* ignore */
    }
    ws = null;
  }
}

function stopDemo() {
  if (demo) {
    demo.stop();
    demo = null;
  }
}

function scheduleReconnect(gen: number) {
  if (gen !== generation) return;
  const delay = Math.min(10_000, 500 * 2 ** retry) * (0.8 + Math.random() * 0.4);
  retry = Math.min(retry + 1, 6);
  window.clearTimeout(reconnectTimer);
  reconnectTimer = window.setTimeout(() => openWs(gen), delay);
}

function openWs(gen: number) {
  if (gen !== generation) return;
  closeWs();
  let sock: WebSocket;
  try {
    sock = new WebSocket(wsUrl(get().baseUrl));
  } catch {
    scheduleReconnect(gen);
    return;
  }
  ws = sock;
  sock.onopen = () => {
    if (gen !== generation) return;
    retry = 0;
    set({ conn: 'live', unreachable: false });
    void bootstrapRest();
  };
  sock.onmessage = (ev) => {
    if (gen !== generation) return;
    try {
      dispatch(JSON.parse(String(ev.data)) as WsMessage, true);
    } catch {
      /* malformed frame: ignore */
    }
  };
  sock.onclose = () => {
    if (gen !== generation) return;
    ws = null;
    const st = get();
    if (st.conn === 'live' || st.conn === 'connecting') set({ conn: st.unreachable ? 'offline' : 'reconnecting' });
    scheduleReconnect(gen);
  };
  sock.onerror = () => {
    /* onclose follows */
  };
}

/** Connect to the real backend. Clears any demo data first. */
export async function goLive(): Promise<void> {
  const gen = ++generation;
  clearTimers();
  closeWs();
  const wasDemo = get().conn === 'demo' || demo !== null;
  stopDemo();
  if (wasDemo) {
    samples.clear();
    phoneSamples.clear();
    get().resetData();
  }
  skewInit = false;
  retry = 0;
  set({ conn: 'connecting', demoReason: null, liveAvailable: false });
  const base = await resolveBaseUrl();
  if (gen !== generation) return;
  if (base !== get().baseUrl) {
    samples.clear();
    phoneSamples.clear();
    get().resetData();
  }
  set({ baseUrl: base });
  const h = await probeHealth(base);
  if (gen !== generation) return;
  if (h) {
    set({ health: h, sources: h.sources ?? null, unreachable: false });
  } else {
    set({ conn: 'offline', unreachable: true });
  }
  openWs(gen);
  healthTimer = window.setInterval(async () => {
    if (gen !== generation || get().conn !== 'live') return;
    const hh = await probeHealth(get().baseUrl);
    if (hh && gen === generation) set({ health: hh });
  }, 15_000);
}

/** Switch to simulated data. Never mixes with live data. */
export function enterDemo(reason: 'user' | 'fallback'): void {
  const gen = ++generation;
  clearTimers();
  closeWs();
  stopDemo();
  samples.clear();
    phoneSamples.clear();
  get().resetData();
  skew = 0;
  skewInit = true;
  set({
    conn: 'demo',
    demoReason: reason,
    unreachable: false,
    liveAvailable: false,
    settings: { interval_s: 1, scan_interval_s: 300 },
    health: { ok: true, version: 'demo', uptime_s: 0, capabilities: [], auth_required: false },
  });
  demo = new DemoSim((m) => {
    if (gen === generation) dispatch(m, true);
  });
  demo.start();
  // Watch for the real backend so we can switch back.
  healthTimer = window.setInterval(async () => {
    if (gen !== generation) return;
    const base = await resolveBaseUrl();
    const h = await probeHealth(base, 2500);
    if (!h || gen !== generation) return;
    if (get().demoReason === 'fallback') {
      toast('ok', 'Backend is up', `rhcsd answered at ${base}. Switched to LIVE data.`);
      void goLive();
    } else {
      set({ liveAvailable: true });
    }
  }, 5000);
}

/** Always connects to the real backend. There is no demo fallback: if the
 *  home system is off, PULSE says the connection is interrupted. */
export function startPulse(): void {
  if (get().prefs.demo) get().setPrefs({ demo: false });
  void goLive();
}

export function stopPulse(): void {
  generation++;
  clearTimers();
  closeWs();
  stopDemo();
}
