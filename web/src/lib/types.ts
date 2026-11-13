// Wire types for rhcsd. Every field may be null or missing: render "—", never crash.

export interface Telemetry {
  logged_in?: boolean | null;
  modem_state?: string | null;
  network_type?: string | null;
  provider?: string | null;
  ppp_status?: string | null;
  signal_bars?: number | null;
  rsrp?: number | null;
  rsrq?: number | null;
  sinr?: number | null;
  rssi?: number | null;
  pci?: number | null;
  cell_id?: number | null;
  band?: number | null;
  earfcn?: number | null;
  mcc?: string | null;
  mnc?: string | null;
  tac?: string | null;
  rx_bytes_per_s?: number | null;
  tx_bytes_per_s?: number | null;
  session_rx_bytes?: number | null;
  session_tx_bytes?: number | null;
  session_secs?: number | null;
  month_rx_bytes?: number | null;
  month_tx_bytes?: number | null;
}

export interface Sample {
  ts: number;
  rsrp?: number | null;
  rsrq?: number | null;
  sinr?: number | null;
  rx_bytes_per_s?: number | null;
  tx_bytes_per_s?: number | null;
  pci?: number | null;
  cell_id?: number | null;
  band?: number | null;
}

export interface RouterSource {
  online?: boolean;
  logged_in?: boolean;
  error?: string | null;
  last_ok?: number | null;
  host?: string;
  model?: string;
}
export interface PhoneSource {
  online?: boolean;
  error?: string | null;
  last_ok?: number | null;
}
export interface ScanSource {
  running?: boolean;
  last_run?: number | null;
  networks?: string[];
}
export interface Sources {
  router?: RouterSource;
  phone?: PhoneSource;
  scan?: ScanSource;
  /** The phone's position (Termux:API or ADB). */
  location?: PhoneSource;
}

/** The phone's position. Only drawn inside the map, never as text. */
export interface PhoneFix {
  lat: number;
  lon: number;
  accuracy_m: number | null;
  provider: string;
  via: 'termux' | 'adb' | string;
  fix_ts: number;
  read_ts: number;
}

export interface Port {
  port: number;
  label?: string | null;
  banner?: string | null;
}

export interface Upnp {
  location?: string | null;
  server?: string | null;
  friendly_name?: string | null;
  manufacturer?: string | null;
  model_name?: string | null;
  model_number?: string | null;
  model_description?: string | null;
  device_type?: string | null;
}

export interface DeviceView {
  network: string;
  label?: string | null;
  first_seen?: number | null;
  last_seen?: number | null;
  ip: string;
  mac?: string | null;
  vendor?: string | null;
  hostname?: string | null;
  rtt_ms?: number | null;
  is_gateway?: boolean;
  is_self?: boolean;
  ports?: Port[] | null;
  upnp?: Upnp[] | null;
}

export interface HotspotClient {
  ip?: string | null;
  mac?: string | null;
  iface?: string | null;
  state?: string | null;
  vendor?: string | null;
  label?: string | null;
}

export interface PhoneStatus {
  model?: string | null;
  android?: string | null;
  transport?: 'usb' | 'wifi' | 'local' | null;
  battery_level?: number | null;
  battery_temp_c?: number | null;
  charging?: boolean | null;
  usb_powered?: boolean | null;
  rsrp?: number | null;
  rsrq?: number | null;
  sinr?: number | null;
  rssi?: number | null;
  pci?: number | null;
  cell_id?: number | null;
  tac?: number | null;
  earfcn?: number | null;
  band?: number | null;
  mcc?: string | null;
  mnc?: string | null;
  hotspot_clients?: HotspotClient[] | null;
}

export type EventKind =
  | 'handover'
  | 'login'
  | 'router_online'
  | 'disconnect'
  | 'error'
  | 'phone_online'
  | 'phone_offline'
  | 'device_join'
  | 'device_leave'
  | string;

export interface RhcsEvent {
  type?: 'event';
  ts: number;
  kind: EventKind;
  message?: string | null;
  data?: unknown;
}

export interface Health {
  ok?: boolean;
  version?: string;
  uptime_s?: number;
  capabilities?: string[];
  auth_required?: boolean;
  /** rhcsd runs as administrator (link switching possible). */
  admin?: boolean;
  switch_supported?: boolean;
  sources?: Sources;
}

export interface Settings {
  interval_s?: number | null;
  scan_interval_s?: number | null;
}

export interface ActionResult {
  ok?: boolean;
  message?: string;
  interval_s?: number;
}

export type WsMessage =
  | { type: 'status'; ts: number; data: Sources }
  | { type: 'telemetry'; ts: number; data: Telemetry }
  | { type: 'phone'; ts: number; data: PhoneStatus }
  | { type: 'devices'; ts: number; data: DeviceView[] }
  | { type: 'location'; ts: number; data: PhoneFix }
  | ({ type: 'event' } & RhcsEvent);

export type RouterAction = 'relogin' | 'reconnect' | 'disconnect' | 'set_network_mode';
