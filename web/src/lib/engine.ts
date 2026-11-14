// The "engine" is rhcsd, the small program that talks to the router and devices.
// This file holds what the website needs to offer it, ask it about itself and stop it.
// Starting it again is not an API (the engine is not running): on Windows the installer
// registers a `pulse:` link that launches it.

import { authHeaders } from './rhcs';
import { useStore } from './store';

/** Fixed download link, served by the "Build Windows installer" GitHub Actions workflow. */
export const WINDOWS_INSTALLER_URL = 'https://github.com/awaaladin/control/releases/download/windows-latest/PULSE-Setup.exe';
/** Approximate size shown before downloading (the real file differs a little per build). */
export const WINDOWS_INSTALLER_SIZE = 'about 10 MB';
/** The link the installer registers; it starts the engine and nothing else. */
export const START_LINK = 'pulse://start';

export type OsKind = 'windows' | 'android' | 'other';

export function detectOs(): OsKind {
  const ua = navigator.userAgent;
  if (/Android/i.test(ua)) return 'android';
  if (/Windows/i.test(ua)) return 'windows';
  return 'other';
}

/** True inside the Android app (its user agent carries "PulseApp"); the engine is bundled there. */
export const insideApp = (): boolean => /PulseApp/.test(navigator.userAgent);

export interface EngineInfo {
  running: boolean;
  mode: 'app' | 'windows' | 'termux' | 'other';
  version: string;
  uptime_s: number;
  can_stop: boolean;
}

export type EngineResult =
  | { kind: 'ok'; info: EngineInfo }
  | { kind: 'old' } // 404: this engine is older than the engine API
  | { kind: 'auth' } // needs (or rejected) the access token
  | { kind: 'down' }; // nothing answered

async function call(path: string, method: 'GET' | 'POST'): Promise<{ status: number; data: unknown }> {
  const base = useStore.getState().baseUrl;
  if (!base) throw new Error('no backend');
  const ctl = new AbortController();
  const t = window.setTimeout(() => ctl.abort(), 6000);
  try {
    const r = await fetch(base + path, {
      method,
      cache: 'no-store',
      signal: ctl.signal,
      headers: { ...authHeaders(base), ...(method === 'POST' ? { 'content-type': 'application/json' } : {}) },
      body: method === 'POST' ? '{}' : undefined,
    });
    let data: unknown = null;
    try {
      data = await r.json();
    } catch {
      /* not JSON */
    }
    return { status: r.status, data };
  } finally {
    window.clearTimeout(t);
  }
}

export async function getEngine(): Promise<EngineResult> {
  try {
    const { status, data } = await call('/api/engine', 'GET');
    if (status === 404) return { kind: 'old' };
    if (status === 401 || status === 403 || status === 429) return { kind: 'auth' };
    const o = data as Partial<EngineInfo> | null;
    if (status === 200 && o && typeof o === 'object' && typeof o.running === 'boolean') {
      return {
        kind: 'ok',
        info: {
          running: o.running,
          mode: (o.mode as EngineInfo['mode']) ?? 'other',
          version: String(o.version ?? ''),
          uptime_s: Number(o.uptime_s ?? 0),
          can_stop: o.can_stop === true,
        },
      };
    }
    return { kind: 'down' };
  } catch {
    return { kind: 'down' };
  }
}

export type StopOutcome = 'stopped' | 'auth' | 'old' | 'refused' | 'failed';

export async function stopEngine(): Promise<{ outcome: StopOutcome; message: string }> {
  try {
    const { status, data } = await call('/api/engine/stop', 'POST');
    const message = (data as { message?: string } | null)?.message ?? '';
    if (status === 404) return { outcome: 'old', message };
    if (status === 401 || status === 403 || status === 429) return { outcome: 'auth', message };
    if (status === 200 && (data as { ok?: boolean } | null)?.ok !== false) return { outcome: 'stopped', message };
    return { outcome: 'refused', message };
  } catch {
    // The engine may close the connection while exiting; the caller re-checks with getEngine().
    return { outcome: 'failed', message: '' };
  }
}

/** Opens the `pulse:` link (the installer's handler starts the engine). */
export function startEngineLink(): void {
  window.location.href = START_LINK;
}

// "Not now" is remembered for 7 days.
const SNOOZE_KEY = 'pulse.engineSnooze';
export function snoozed(): boolean {
  try {
    return Number(localStorage.getItem(SNOOZE_KEY) ?? 0) > Date.now();
  } catch {
    return false;
  }
}
export function snooze(days = 7): void {
  try {
    localStorage.setItem(SNOOZE_KEY, String(Date.now() + days * 86_400_000));
  } catch {
    /* storage unavailable: dismissed for this visit only */
  }
}
