// Tower map data: /api/towers and /api/location.

import { useEffect } from 'react';
import { create } from 'zustand';
import { getJson, postJson } from './boost';
import { toast, useStore } from './store';

export interface SourceStats {
  first_seen: number;
  last_seen: number;
  secs: number;
  samples: number;
  best_rsrp: number | null;
  avg_rsrp: number | null;
  best_sinr: number | null;
  avg_sinr: number | null;
}

export interface CellLocation {
  lat: number;
  lon: number;
  range_m: number | null;
  samples: number | null;
  source: string;
}

export interface TowerCell {
  key: string;
  mcc: string | null;
  mnc: string | null;
  tac: number | null;
  cell_id: number;
  enb: number;
  sector: number;
  pci: number | null;
  bands: number[];
  earfcn: number | null;
  first_seen: number;
  last_seen: number;
  seen_by: ('router' | 'phone')[];
  serving: ('router' | 'phone')[];
  router: SourceStats | null;
  phone: SourceStats | null;
  location: CellLocation | null;
  lookup: { status: 'no_key' | 'incomplete' | 'pending' | 'found' | 'not_found' | 'error'; at?: number; message: string | null };
  distance_m: number | null;
  bearing_deg: number | null;
  compass: string | null;
}

export interface HandoverEnd {
  key: string;
  cell_id: number;
  pci: number | null;
  band: number | null;
}

export interface TowerHandover {
  ts: number;
  source: 'router' | 'phone';
  from: HandoverEnd;
  to: HandoverEnd;
  stayed_s: number;
}

export interface HomeLocation {
  lat: number;
  lon: number;
  accuracy_m: number | null;
  source: string;
  ts: number;
}

export interface TowersView {
  key_configured: boolean;
  message: string | null;
  home: HomeLocation | null;
  serving: { router: string | null; phone: string | null };
  cells: TowerCell[];
  handovers: TowerHandover[];
  best: { key: string; avg_sinr: number; message: string } | null;
}

interface TowersStore {
  view: TowersView | null;
  unsupported: boolean;
  at: number | null;
}

export const useTowers = create<TowersStore>()(() => ({ view: null, unsupported: false, at: null }));

export async function refreshTowers(): Promise<void> {
  const a = await getJson<TowersView>('/api/towers');
  if (a.ok && a.data) useTowers.setState({ view: a.data, unsupported: false, at: Date.now() });
  else if (a.status === 404) useTowers.setState({ unsupported: true });
}

export function useTowersPolling(): void {
  const baseUrl = useStore((s) => s.baseUrl);
  const conn = useStore((s) => s.conn);
  useEffect(() => {
    if (!baseUrl || conn === 'demo') return;
    void refreshTowers();
    const t = window.setInterval(() => void refreshTowers(), 10_000);
    return () => window.clearInterval(t);
  }, [baseUrl, conn]);
}

/** Sends this device's position (browser geolocation) as the home location. */
export function shareThisDeviceLocation(): void {
  if (!('geolocation' in navigator)) {
    toast('error', 'Location not available', 'This browser has no location support.');
    return;
  }
  if (!window.isSecureContext) {
    toast(
      'warn',
      'Location blocked by the browser',
      'Browsers only share location with https pages or localhost. Open PULSE on the laptop itself (http://localhost:7878), or set HOME_LAT and HOME_LON in .env.',
      9000,
    );
    return;
  }
  navigator.geolocation.getCurrentPosition(
    async (pos) => {
      const a = await postJson('/api/location', {
        lat: pos.coords.latitude,
        lon: pos.coords.longitude,
        accuracy_m: pos.coords.accuracy,
        source: 'browser',
      });
      if (a.ok) {
        toast('ok', 'Home location saved', `Accuracy about ${Math.round(pos.coords.accuracy)} m.`);
        void refreshTowers();
      } else if (a.status !== 401) toast('error', 'Could not save location', a.message);
    },
    (err) => toast('error', 'Location not shared', err.message || 'The browser refused to share the location.'),
    { enableHighAccuracy: true, timeout: 20_000, maximumAge: 60_000 },
  );
}
