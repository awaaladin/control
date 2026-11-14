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
  /** Last signal the phone measured here without being served by it. */
  nearby?: { ts: number; rsrp: number | null; rsrq: number | null; sinr: number | null } | null;
  serving: ('router' | 'phone')[];
  router: SourceStats | null;
  phone: SourceStats | null;
  location: CellLocation | null;
  lookup: { status: 'off' | 'incomplete' | 'pending' | 'found' | 'not_found' | 'error'; at?: number; message: string | null };
  distance_m: number | null;
  bearing_deg: number | null;
  compass: string | null;
}

/** One line of "Cells around you": the serving cell first, then what else the phone hears. */
export interface AroundCell {
  serving: boolean;
  key: string | null;
  ci: number | null;
  enb: number | null;
  sector: number | null;
  pci: number | null;
  tac: number | null;
  earfcn: number | null;
  band: number | null;
  mhz: string | null;
  rsrp: number | null;
  rsrq: number | null;
  sinr: number | null;
  rat: string | null;
  location: CellLocation | null;
  lookup: TowerCell['lookup'] | null;
  distance_m: number | null;
  bearing_deg: number | null;
  compass: string | null;
}

export interface Recommendation {
  status: 'better' | 'best' | 'unknown' | 'stale';
  message: string;
  key?: string | null;
  enb?: number | null;
  sector?: number | null;
  band?: number | null;
  delta_db?: number;
  distance_m?: number | null;
  compass?: string | null;
  can_force?: boolean;
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
  /** An OpenCellID key is set (beaconDB needs none). */
  key_configured: boolean;
  lookup?: { enabled: boolean; sources: string[]; external?: boolean };
  message: string | null;
  /** Where distances are measured from: the manual home, else the phone's position. */
  home: HomeLocation | null;
  /** True when `home` is the manual home (.env or browser), false when it is the phone's position. */
  home_manual?: boolean;
  /** The phone's latest position (map only). */
  current?: HomeLocation | null;
  serving: { router: string | null; phone: string | null };
  cells: TowerCell[];
  handovers: TowerHandover[];
  best: { key: string; avg_sinr: number; message: string } | null;
  /** Cells the phone hears right now (app only); null when none reported. */
  around?: { ts: number; age_s: number; cells: AroundCell[] } | null;
  recommendation?: Recommendation | null;
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

// ---------------------------------------------------------------- automatic places

export interface PlaceSig {
  samples: number;
  avg_sinr: number | null;
  best_sinr: number | null;
  avg_rsrp: number | null;
  best_rsrp: number | null;
}

export interface Place {
  id: number;
  rank: number;
  label: string;
  name: string | null;
  /** In words, e.g. "about 20 m north-east of home". */
  where: string;
  distance_m: number | null;
  compass: string | null;
  samples: number;
  quality_sinr: number | null;
  router: PlaceSig | null;
  phone: PlaceSig | null;
  accuracy_m: number;
  first_seen: number;
  last_seen: number;
  is_current: boolean;
}

export interface PlacesView {
  home: 'manual' | 'most_visited' | 'none';
  current_place: number | null;
  places: Place[];
  position: { provider: string; via: string; accuracy_m: number | null; age_s: number; fresh: boolean } | null;
  position_error: string | null;
}

interface PlacesStore {
  view: PlacesView | null;
  unsupported: boolean;
}

export const usePlaces = create<PlacesStore>()(() => ({ view: null, unsupported: false }));

export async function refreshPlaces(): Promise<void> {
  const a = await getJson<PlacesView>('/api/places');
  if (a.ok && a.data) usePlaces.setState({ view: a.data, unsupported: false });
  else if (a.status === 404) usePlaces.setState({ unsupported: true });
}

export function usePlacesPolling(): void {
  const baseUrl = useStore((s) => s.baseUrl);
  const conn = useStore((s) => s.conn);
  useEffect(() => {
    if (!baseUrl || conn === 'demo') return;
    void refreshPlaces();
    const t = window.setInterval(() => void refreshPlaces(), 15_000);
    return () => window.clearInterval(t);
  }, [baseUrl, conn]);
}

export async function renamePlace(id: number, name: string): Promise<void> {
  const a = await postJson(`/api/places/${id}/name`, { name });
  if (a.ok) toast('ok', 'Place renamed', a.message);
  else if (a.status !== 401) toast('error', 'Could not rename', a.message);
  void refreshPlaces();
}
