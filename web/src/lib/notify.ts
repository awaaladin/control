// Opt-in browser notifications when a device without a friendly name joins a
// network (LAN scan or phone hotspot). Off by default; the choice is stored in
// this browser only. Starts listening as soon as this module is loaded.

import { useStore } from './store';
import type { RhcsEvent } from './types';

const KEY = 'pulse.notifyUnknownDevices';

function read(): boolean {
  try {
    return localStorage.getItem(KEY) === '1';
  } catch {
    return false;
  }
}

let enabled = read();
const loadedAt = Date.now();
let lastTs = 0;

export const notifySupported = (): boolean => typeof window !== 'undefined' && 'Notification' in window && window.isSecureContext;
export const notifyEnabled = (): boolean => enabled && notifySupported() && Notification.permission === 'granted';

/** Turns alerts on (asking the browser for permission) or off. Returns the new state. */
export async function setNotify(on: boolean): Promise<boolean> {
  if (on) {
    if (!notifySupported()) return false;
    const p = Notification.permission === 'default' ? await Notification.requestPermission() : Notification.permission;
    if (p !== 'granted') return false;
  }
  enabled = on;
  try {
    localStorage.setItem(KEY, on ? '1' : '0');
  } catch {
    /* private mode: works until the tab closes */
  }
  return on;
}

/** A join event for a device that has no friendly name from .env. */
export function isUnknownJoin(e: RhcsEvent): boolean {
  if (e.kind !== 'device_join') return false;
  const d = (e.data ?? {}) as { label?: string | null };
  return !d.label;
}

useStore.subscribe((s, prev) => {
  if (s.events === prev.events || !notifyEnabled()) return;
  for (const e of s.events) {
    // Only events that happened after this page was opened; history is not re-announced.
    if (e.ts <= lastTs || e.ts < loadedAt - 5000) continue;
    lastTs = Math.max(lastTs, e.ts);
    if (!isUnknownJoin(e)) continue;
    try {
      new Notification('Unknown device joined', { body: e.message ?? 'A device without a name joined your network.', tag: `join-${e.ts}` });
    } catch {
      /* some mobile browsers only allow notifications from a service worker */
    }
  }
});
