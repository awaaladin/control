// Phone Game Boost: talks to the Android app through the `PulseApp` object the app adds to the
// page (see PulseBridge.kt). Outside the app there is no such object and nothing here runs.

import { useEffect } from 'react';
import { create } from 'zustand';
import { insideApp } from './engine';

export interface PhoneNet {
  available: boolean;
  in_use: boolean;
  ms?: number;
}

export type ShizukuState = 'not_installed' | 'not_running' | 'needs_permission' | 'ready';

export interface PhonePriv {
  state: ShizukuState;
  denied_forever: boolean;
  auto: boolean;
  tower_auto?: boolean;
  busy: boolean;
  last: string | null;
  wifi_off_by_us: boolean;
  airplane_pending: boolean;
  repick_blocked: string | null;
}

export interface PhoneBoostStatus {
  available: boolean;
  sdk?: number;
  enabled?: boolean;
  state?: 'off' | 'needs_usage' | 'waiting' | 'boosting';
  usage_access?: boolean;
  dnd_access?: boolean;
  opt_dnd?: boolean;
  opt_lowlat?: boolean;
  dnd_active?: boolean;
  wifi_lock?: boolean;
  wake_lock?: boolean;
  low_latency_api?: boolean;
  service_alive?: boolean;
  game?: { pkg: string; label: string };
  networks?: { wifi: PhoneNet; cellular: PhoneNet };
  measured_age_s?: number;
  recommendation?: { kind: 'none' | 'stay' | 'switch'; text: string | null };
  hint?: string | null;
  priv?: PhonePriv;
  cost?: { probe_bytes: number; probes: number; interval_s: number };
  rsrp?: number;
  games?: string[];
}

export interface PhoneApp {
  pkg: string;
  label: string;
  game: boolean;
  chosen: boolean;
}

interface Bridge {
  getStatus(): string;
  setEnabled(on: boolean): void;
  setOption(name: string, on: boolean): void;
  setGame(pkg: string, on: boolean): void;
  listApps(): string;
  openUsageAccess(): void;
  openDndAccess(): void;
  openInternetPanel(): void;
  privAction(action: string, confirmed: boolean): string;
  openShizukuDownload(): void;
  openShizuku(): void;
  openWifiSettings(): void;
}

declare global {
  interface Window {
    PulseApp?: Bridge;
  }
}

const bridge = (): Bridge | null => (insideApp() && window.PulseApp ? window.PulseApp : null);

interface PhoneStore {
  status: PhoneBoostStatus | null;
}

export const usePhoneBoost = create<PhoneStore>()(() => ({ status: null }));

export function refreshPhoneBoost(): void {
  const b = bridge();
  if (!b) return;
  try {
    usePhoneBoost.setState({ status: JSON.parse(b.getStatus()) as PhoneBoostStatus });
  } catch {
    usePhoneBoost.setState({ status: { available: false } });
  }
}

/** Polls the app every 2 s while the panel is on screen. */
export function usePhoneBoostPolling(): void {
  useEffect(() => {
    refreshPhoneBoost();
    const t = window.setInterval(refreshPhoneBoost, 2000);
    // the user comes back from the Settings screens
    const onVis = () => refreshPhoneBoost();
    document.addEventListener('visibilitychange', onVis);
    return () => {
      window.clearInterval(t);
      document.removeEventListener('visibilitychange', onVis);
    };
  }, []);
}

export function phoneSetEnabled(on: boolean): void {
  bridge()?.setEnabled(on);
  refreshPhoneBoost();
}
export function phoneSetOption(name: 'dnd' | 'lowlat' | 'priv_auto' | 'tower_auto', on: boolean): void {
  bridge()?.setOption(name, on);
  refreshPhoneBoost();
}
export function phoneSetGame(pkg: string, on: boolean): void {
  bridge()?.setGame(pkg, on);
  refreshPhoneBoost();
}
export function phoneListApps(): PhoneApp[] {
  const b = bridge();
  if (!b) return [];
  try {
    return JSON.parse(b.listApps()) as PhoneApp[];
  } catch {
    return [];
  }
}
export const phoneOpenUsageAccess = () => bridge()?.openUsageAccess();
export const phoneOpenDndAccess = () => bridge()?.openDndAccess();
export const phoneOpenInternetPanel = () => bridge()?.openInternetPanel();
export const phoneOpenShizukuDownload = () => bridge()?.openShizukuDownload();
export const phoneOpenShizuku = () => bridge()?.openShizuku();
export const phoneOpenWifiSettings = () => bridge()?.openWifiSettings();

/** Privileged actions: a fixed list of names, never a command. Returns the app's plain-language answer. */
export function phonePrivAction(action: 'request_permission' | 'repick' | 'mobile_only' | 'wifi_back', confirmed = false): { ok: boolean; message: string } {
  const b = bridge();
  if (!b) return { ok: false, message: 'Not running inside the app.' };
  try {
    const r = JSON.parse(b.privAction(action, confirmed)) as { ok?: boolean; message?: string };
    refreshPhoneBoost();
    return { ok: !!r.ok, message: String(r.message ?? '') };
  } catch {
    return { ok: false, message: 'The app did not answer.' };
  }
}
