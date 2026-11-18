// Game Boost for the PC itself: /api/boost/device (see crates/rhcsd/src/devboost.rs).

import { useEffect } from 'react';
import { create } from 'zustand';
import { getJson, postJson } from './boost';
import { toast, useStore } from './store';

export interface DevAction {
  id: string;
  title: string;
  description: string;
  needs_admin: boolean;
  available: boolean;
  default_on: boolean;
  active: boolean;
  detail: string;
}

export interface DevCandidate {
  pid: number;
  name: string;
  /** Only present for programs that look like games or emulators. */
  window_title?: string;
  mem_mb: number;
  likely_game: boolean;
  hint?: string;
}

export interface DevChange {
  id: string;
  title: string;
  detail: string;
  reversible: boolean;
}

export interface DevSnap {
  link_id: string;
  link_name: string;
  ping_ms: number | null;
  jitter_ms: number | null;
  loss_pct: number | null;
  at: number;
}

export interface DevVerdict {
  improved: boolean | null;
  text: string;
  /** after - before; negative = lower = better. */
  ping_ms: number | null;
  jitter_ms: number | null;
  loss_pct: number | null;
}

export interface DevResult {
  before: DevSnap | null;
  after: DevSnap | null;
  verdict: DevVerdict;
  finished_at: number;
}

export type DevPhase = 'idle' | 'measuring_before' | 'applying' | 'active' | 'restoring' | 'restore_failed';

export interface DevState {
  active: boolean;
  phase: DevPhase;
  started_at: number | null;
  game: { pid: number; name: string } | null;
  changes: DevChange[];
  before: DevSnap | null;
  live: DevSnap | null;
  message: string | null;
}

export interface DevView {
  supported: boolean;
  unsupported_reason: string | null;
  admin: boolean;
  actions: DevAction[];
  candidates: DevCandidate[];
  state: DevState;
  last_result: DevResult | null;
}

export interface DevPlanItem {
  id: string;
  title: string;
  detail: string;
}

export interface DevStartRequest {
  actions: string[];
  game_pid?: number;
  background_pids?: number[];
  dry_run?: boolean;
}

interface DevStore {
  view: DevView | null;
  /** The backend answered but has no /api/boost/device (older rhcsd). */
  unsupported: boolean;
  at: number | null;
}

export const useDevBoost = create<DevStore>()(() => ({ view: null, unsupported: false, at: null }));

export async function refreshDevBoost(): Promise<void> {
  const a = await getJson<DevView>('/api/boost/device');
  if (a.ok && a.data) useDevBoost.setState({ view: a.data, unsupported: false, at: Date.now() });
  else if (a.status === 404) useDevBoost.setState({ unsupported: true });
}

interface PlanAnswer {
  ok: boolean;
  message: string;
  would_change?: DevPlanItem[];
  skipped?: { id: string; reason: string }[];
}

/** Asks rhcsd what WOULD change. Changes nothing. */
export async function planDevBoost(req: DevStartRequest): Promise<{ items: DevPlanItem[]; skipped: { id: string; reason: string }[]; error?: string }> {
  const a = await postJson<PlanAnswer>('/api/boost/device/start', { ...req, dry_run: true });
  if (!a.ok || !a.data) return { items: [], skipped: a.data?.skipped ?? [], error: a.status === 401 ? 'not allowed' : a.message };
  return { items: a.data.would_change ?? [], skipped: a.data.skipped ?? [] };
}

export async function startDevBoost(req: DevStartRequest): Promise<boolean> {
  const a = await postJson<PlanAnswer>('/api/boost/device/start', { ...req, dry_run: false });
  if (a.ok) toast('ok', 'Boosting this PC', a.message, 6000);
  else if (a.status !== 401) toast('error', 'Could not boost', a.message, 12000);
  void refreshDevBoost();
  return a.ok;
}

export async function stopDevBoost(): Promise<void> {
  const a = await postJson<{ ok: boolean; message: string }>('/api/boost/device/stop', {});
  if (a.ok) toast('ok', 'Restored', a.message, 6000);
  else if (a.status !== 401) toast('error', 'Could not restore everything', a.message, 15000);
  void refreshDevBoost();
}

/** Polls while mounted: quickly while a boost runs, slowly otherwise. */
export function useDevBoostPolling(): void {
  const baseUrl = useStore((s) => s.baseUrl);
  const conn = useStore((s) => s.conn);
  useEffect(() => {
    if (!baseUrl || conn === 'demo') return;
    let alive = true;
    let timer = 0;
    const loop = async () => {
      await refreshDevBoost();
      if (!alive) return;
      const phase = useDevBoost.getState().view?.state.phase ?? 'idle';
      timer = window.setTimeout(loop, phase === 'idle' ? 6000 : 1500);
    };
    void loop();
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [baseUrl, conn]);
}
