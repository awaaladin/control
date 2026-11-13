// Every internet connection by name: /api/connections.

import { useEffect } from 'react';
import { create } from 'zustand';
import { getJson } from './boost';
import { useStore } from './store';

export interface ConnRadio {
  source: 'router' | 'phone';
  rsrp: number | null;
  rsrq: number | null;
  sinr: number | null;
  rssi: number | null;
  band: number | null;
  earfcn: number | null;
  pci: number | null;
  cell_id: number | null;
  enb: number | null;
  sector: number | null;
  tower: string | null;
  network_type: string | null;
}

export interface Connection {
  id: string;
  name: string;
  kind: 'router' | 'phone' | 'other';
  link_kind: 'router' | 'phone_usb' | 'phone_hotspot' | 'other' | null;
  alias: string | null;
  ssid: string | null;
  medium: string | null;
  in_use: boolean | null;
  radio: ConnRadio | null;
  radio_note: string | null;
  throughput: { rx_bps: number | null; tx_bps: number | null; source: string } | null;
  ping: { ping_ms: number | null; jitter_ms: number | null; loss_pct: number | null; score: number | null; grade: string; at: number | null } | null;
}

export interface ConnectionsView {
  connections: Connection[];
  best: string | null;
  admin: boolean;
  switch_supported: boolean;
}

interface ConnStore {
  view: ConnectionsView | null;
  unsupported: boolean;
  at: number | null;
}

export const useConnections = create<ConnStore>()(() => ({ view: null, unsupported: false, at: null }));

export async function refreshConnections(): Promise<void> {
  const a = await getJson<ConnectionsView>('/api/connections');
  if (a.ok && a.data) useConnections.setState({ view: a.data, unsupported: false, at: Date.now() });
  else if (a.status === 404) useConnections.setState({ unsupported: true });
}

export function useConnectionsPolling(ms = 5000): void {
  const baseUrl = useStore((s) => s.baseUrl);
  const conn = useStore((s) => s.conn);
  useEffect(() => {
    if (!baseUrl || conn === 'demo') return;
    void refreshConnections();
    const t = window.setInterval(() => void refreshConnections(), ms);
    return () => window.clearInterval(t);
  }, [baseUrl, conn, ms]);
}

export const KIND_WORD: Record<Connection['kind'], string> = { router: 'router', phone: 'phone', other: 'other network' };
