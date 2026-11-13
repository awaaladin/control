// Typed rhcsd API client, WebSocket manager and live/demo orchestration.

import { phoneSamples, samples, sampleFromTelemetry } from './buffer';
import { useStore, toast, WINDOW_MS, type AuthIssue } from './store';
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
  // Browsers can't set headers on a WebSocket, so the token goes in the query.
  const token = tokenFor(base);
  if (token) u.searchParams.set('token', token);
  return u.toString();
}

// ---------------------------------------------------------------- access tokens
/** The token for this backend: its own if saved, else the global one. */
export function tokenFor(base: string): string {
  const p = get().prefs;
  return (p.tokens[normalizeUrl(base)] ?? '').trim() || p.token.trim();
}

/** Headers carrying the access token (empty when none is saved). */
export function authHeaders(base: string): Record<string, string> {
  const t = tokenFor(base);
  return t ? { 'x-rhcs-token': t } : {};
}

/** Save the token for one backend ('' removes it) and reconnect. */
export function setBackendToken(base: string, token: string): void {
  const url = normalizeUrl(base);
  const tokens = { ...get().prefs.tokens };
  if (token.trim()) tokens[url] = token.trim();
  else delete tokens[url];
  get().setPrefs({ tokens });
  void goLive();
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

/** How a backend answered: fine, wants a token, rejected ours, locked us out, remote access off, or silence. */
export type HubAuth = 'ok' | AuthIssue | 'down';

export async function probeHub(base: string, timeoutMs = 3500): Promise<{ auth: HubAuth; health: Health | null }> {
  try {
    const { status, data } = await fetchJson<Health & { error?: string }>(
      `${base}/api/health`,
      { cache: 'no-store', headers: authHeaders(base) },
      timeoutMs,
    );
    const obj = data && typeof data === 'object' ? data : null;
    if (status === 200 && obj && Array.isArray(obj.capabilities)) return { auth: 'ok', health: obj };
    if (status === 200 && obj?.auth_required === true) return { auth: 'needs', health: null };
    if (status === 401) return { auth: obj?.error === 'token_wrong' ? 'wrong' : 'needs', health: null };
    if (status === 429) return { auth: 'locked', health: null };
    if (status === 403 && obj?.error === 'remote_token_not_configured') return { auth: 'forbidden', health: null };
    return { auth: 'down', health: null };
  } catch {
    return { auth: 'down', health: null };
  }
}

async function probeHealth(base: string, timeoutMs = 3500): Promise<Health | null> {
  return (await probeHub(base, timeoutMs)).health;
}

/**
 * Every place PULSE looks for the home system, best first:
 * the address you set, the one that worked last time, your saved backends,
 * this page's own server (when rhcsd serves PULSE), then this device (the
 * phone hub or the laptop itself).
 */
export function candidateUrls(): string[] {
  const p = get().prefs;
  const list = [
    normalizeUrl(p.baseUrl),
    normalizeUrl(p.lastGood),
    ...p.hubs.map(normalizeUrl),
    location.port === DEFAULT_PORT || /^https?:$/.test(location.protocol) ? sameOrigin() : '',
    FALLBACK_URL,
  ].filter(Boolean);
  // A hosted https page can't call a plain-http address on your network (browser rule),
  // except this device itself; those are still listed so the card can explain it.
  return [...new Set(list)];
}

/** True when the browser will refuse to call this address from this page. */
export function blockedByBrowser(url: string): boolean {
  return location.protocol === 'https:' && /^http:\/\//.test(url) && !/^http:\/\/(localhost|127\.0\.0\.1)(:|\/|$)/.test(url);
}

export interface HubProbe {
  url: string;
  ok: boolean;
  blocked: boolean;
  auth: HubAuth;
}

/** Last search result, shown on the "connection interrupted" card. */
export let lastSearch: HubProbe[] = [];

export async function resolveBaseUrl(): Promise<string> {
  const urls = candidateUrls();
  const probes = await Promise.all(
    urls.map(async (url): Promise<HubProbe> => {
      const blocked = blockedByBrowser(url);
      if (blocked) return { url, ok: false, blocked, auth: 'down' };
      const r = await probeHub(url, 2500);
      return { url, ok: r.auth === 'ok', blocked, auth: r.auth };
    }),
  );
  lastSearch = probes;
  const hit = probes.find((p) => p.ok);
  if (hit) {
    useStore.setState({ authIssue: null });
    if (get().prefs.lastGood !== hit.url) get().setPrefs({ lastGood: hit.url });
    return hit.url;
  }
  // Something answered but wants a token: stay on it and say so.
  const gated = probes.find((p) => p.auth !== 'down' && p.auth !== 'ok');
  if (gated) {
    useStore.setState({ authIssue: gated.auth as AuthIssue });
    return gated.url;
  }
  useStore.setState({ authIssue: null });
  return urls[0] ?? FALLBACK_URL;
}

/** Save a backend address and connect to it now. Returns false if it isn't a usable address. */
export function addHub(raw: string, token?: string): boolean {
  const url = normalizeUrl(raw);
  try {
    new URL(url);
  } catch {
    return false;
  }
  if (!url) return false;
  const p = get().prefs;
  if (!p.hubs.includes(url)) get().setPrefs({ hubs: [...p.hubs, url] });
  if (token?.trim()) get().setPrefs({ tokens: { ...p.tokens, [url]: token.trim() } });
  get().setPrefs({ lastGood: url });
  void goLive();
  return true;
}

export function removeHub(url: string): void {
  const p = get().prefs;
  const tokens = { ...p.tokens };
  delete tokens[url];
  get().setPrefs({ hubs: p.hubs.filter((h) => h !== url), tokens, lastGood: p.lastGood === url ? '' : p.lastGood });
}

/**
 * Pairing link: https://.../#hub=<url>&token=<token>. The fragment never goes to
 * any server. Saves the backend (and token), clears the address bar, and says so.
 * Returns true when a pairing was applied (addHub has already started connecting).
 */
export function applyPairingFromHash(): boolean {
  const raw = location.hash.replace(/^#/, '');
  if (!raw.includes('hub=')) return false;
  const q = new URLSearchParams(raw);
  const hub = q.get('hub') ?? '';
  const token = q.get('token') ?? '';
  try {
    history.replaceState(null, '', location.pathname + location.search);
  } catch {
    /* ignore */
  }
  if (!/^https?:\/\//i.test(hub) || !addHub(hub, token)) {
    toast('error', 'Pairing link not understood', 'It needs a hub address such as https://name.trycloudflare.com');
    return false;
  }
  let host = hub;
  try {
    host = new URL(hub).host;
  } catch {
    /* keep raw */
  }
  toast('ok', `Paired with ${host}`, token ? 'Address and access token saved in this browser.' : 'Address saved. Enter the access token if asked.');
  return true;
}

/** Copy-ready pairing link for a saved backend (contains the token if one is saved). */
export function pairingLink(base: string): { link: string; hasToken: boolean } {
  const url = normalizeUrl(base);
  const token = tokenFor(url);
  const origin = location.protocol === 'https:' ? location.origin : 'https://rhcs-pulse.vercel.app';
  let frag = `hub=${encodeURIComponent(url)}`;
  if (token) frag += `&token=${encodeURIComponent(token)}`;
  return { link: `${origin}/#${frag}`, hasToken: !!token };
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
    const { status, data } = await fetchJson<T>(base + path, { cache: 'no-store', headers: authHeaders(base) });
    return status === 200 ? data : null;
  } catch {
    return null;
  }
}

async function apiPost(path: string, body: unknown): Promise<PostResult> {
  if (get().conn === 'demo' && demo) return demo.post(path, body);
  const base = get().baseUrl;
  const headers: Record<string, string> = { 'content-type': 'application/json', ...authHeaders(base) };
  try {
    const { status, data } = await fetchJson<ActionResult>(base + path, {
      method: 'POST',
      headers,
      body: JSON.stringify(body ?? {}),
    });
    const message = data?.message ?? (status === 200 ? 'done' : `HTTP ${status}`);
    if (status === 429) {
      toast('error', 'Too many wrong token attempts', 'This backend is ignoring you for a few minutes. Check the token, then try again later.', 8000);
    } else if (status === 401) {
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
  reconnectTimer = window.setTimeout(() => void reconnectNow(gen), delay);
}

/** Look for the home system again (all known places), then reopen the live feed. */
async function reconnectNow(gen: number) {
  if (gen !== generation) return;
  const base = await resolveBaseUrl();
  if (gen !== generation) return;
  if (base !== get().baseUrl) {
    samples.clear();
    phoneSamples.clear();
    get().resetData();
    set({ baseUrl: base });
  }
  if (get().authIssue) {
    // Retrying with a missing or wrong token would only add failed attempts.
    set({ conn: 'offline', unreachable: true });
    return;
  }
  openWs(gen);
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
    if (get().authIssue) return; // needs a token: wait for the user, no blind retries
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
  if (applyPairingFromHash()) return; // addHub already started the connection
  void goLive();
}

export function stopPulse(): void {
  generation++;
  clearTimers();
  closeWs();
  stopDemo();
}
