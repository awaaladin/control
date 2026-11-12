// Demo simulator. Every identifier here is deliberately fake: IPs come from
// the documentation range 192.0.2.0/24, MACs from 00:00:5E:00:53:xx
// (RFC 7042 documentation block), MCC/MNC 001/01 is the ITU test network.

import { samples } from './buffer';
import type { PostResult } from './rhcs';
import type { DeviceView, HotspotClient, PhoneStatus, Sample, Sources, Telemetry, WsMessage } from './types';

type Emit = (m: WsMessage) => void;

interface Cell {
  pci: number;
  cell_id: number;
  band: number;
  earfcn: number;
  rsrp: number;
  sinr: number;
  rsrq: number;
}

const CELLS: Cell[] = [
  { pci: 101, cell_id: 0x01e240 * 256 + 1, band: 7, earfcn: 3150, rsrp: -83, sinr: 26, rsrq: -7 },
  { pci: 102, cell_id: 0x01e240 * 256 + 2, band: 7, earfcn: 3150, rsrp: -88, sinr: 19, rsrq: -9 },
  { pci: 207, cell_id: 0x01e2a4 * 256 + 1, band: 3, earfcn: 1300, rsrp: -91, sinr: 14, rsrq: -11 },
];

const gauss = () => {
  let u = 0;
  let v = 0;
  while (u === 0) u = Math.random();
  while (v === 0) v = Math.random();
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
};

const mac = (n: number) => `00:00:5e:00:53:${n.toString(16).padStart(2, '0')}`;
const NET = 'demo-lan 192.0.2.0/24';

function baseDevices(now: number): DeviceView[] {
  const d = (
    n: number,
    over: Partial<DeviceView>,
  ): DeviceView => ({
    network: NET,
    label: null,
    first_seen: now - 3_600_000 * (n + 1),
    last_seen: now,
    ip: `192.0.2.${n}`,
    mac: mac(n),
    vendor: null,
    hostname: null,
    rtt_ms: 2 + Math.random() * 8,
    is_gateway: false,
    is_self: false,
    ports: [],
    upnp: [],
    ...over,
  });
  return [
    d(1, {
      is_gateway: true,
      label: 'Demo router',
      vendor: 'Demo Networks',
      rtt_ms: 1.2,
      ports: [
        { port: 80, label: 'http', banner: 'demo-httpd' },
        { port: 443, label: 'https', banner: null },
        { port: 53, label: 'dns', banner: null },
      ],
      upnp: [
        {
          location: 'http://192.0.2.1:52869/desc.xml',
          server: 'Demo/1.0 UPnP/1.0',
          friendly_name: 'Demo LTE Router',
          manufacturer: 'Demo Networks',
          model_name: 'DEMO-4G',
          model_number: '1',
          model_description: 'Simulated LTE CPE',
          device_type: 'urn:schemas-upnp-org:device:InternetGatewayDevice:1',
        },
      ],
    }),
    d(10, { is_self: true, hostname: 'this-pc', vendor: 'Demo Computers', rtt_ms: 0.1, ports: [{ port: 7878, label: 'rhcsd', banner: null }] }),
    d(23, { vendor: 'Samsung Electronics', hostname: 'android-demo', label: 'Phone (demo)' }),
    d(31, { vendor: 'Demo Printers', hostname: 'printer', ports: [{ port: 631, label: 'ipp', banner: null }, { port: 9100, label: 'jetdirect', banner: null }] }),
    d(44, { vendor: null, hostname: null }),
    d(57, { vendor: 'Demo TV Co', hostname: 'living-room-tv', ports: [{ port: 8008, label: 'http', banner: null }] }),
  ];
}

export class DemoSim {
  private timers: number[] = [];
  private emit: Emit;
  private cellIdx = 0;
  private rsrp = -83;
  private sinr = 26;
  private rsrq = -7;
  private burst = 0;
  private burstLevel = 0;
  private sessionRx = 1.8e9;
  private sessionTx = 2.1e8;
  private monthRx = 48.2e9;
  private monthTx = 5.1e9;
  private sessionSecs = 3 * 3600 + 1234;
  private battery = 72;
  private charging = true;
  private devices: DeviceView[] = [];
  private extra: DeviceView | null = null;
  private sources: Sources;
  private interval = 1;
  private nextHandover = 0;
  private tickTimer = 0;

  constructor(emit: Emit) {
    this.emit = emit;
    this.sources = {
      router: { online: true, logged_in: true, error: null, last_ok: Date.now(), host: '192.0.2.1', model: 'DEMO-4G (simulated)' },
      phone: { online: true, error: null, last_ok: Date.now() },
      scan: { running: false, last_run: Date.now() - 120_000, networks: [NET] },
    };
  }

  private cell() {
    return CELLS[this.cellIdx];
  }

  private step(dt: number) {
    const c = this.cell();
    const k = 0.04 * dt;
    this.rsrp += (c.rsrp - this.rsrp) * k + gauss() * 0.7 * Math.sqrt(dt);
    this.sinr += (c.sinr - this.sinr) * k + gauss() * 0.9 * Math.sqrt(dt);
    this.rsrq += (c.rsrq - this.rsrq) * k + gauss() * 0.3 * Math.sqrt(dt);
    this.rsrq = Math.min(-3, Math.max(-20, this.rsrq));
    if (this.burst > 0) this.burst -= dt;
    else if (Math.random() < 0.012 * dt) {
      this.burst = 4 + Math.random() * 25;
      this.burstLevel = 1.5e6 + Math.random() * 5e6;
    }
  }

  private telemetry(): Telemetry {
    const c = this.cell();
    const bursting = this.burst > 0;
    const quality = Math.max(0.15, Math.min(1, (this.sinr + 5) / 30));
    const rx = Math.max(200, (bursting ? this.burstLevel * quality : 18_000) * (0.75 + Math.random() * 0.5));
    const tx = Math.max(100, rx * (bursting ? 0.06 : 0.35) * (0.6 + Math.random() * 0.8));
    this.sessionRx += rx * this.interval;
    this.sessionTx += tx * this.interval;
    this.monthRx += rx * this.interval;
    this.monthTx += tx * this.interval;
    this.sessionSecs += this.interval;
    const rsrp = Math.round(this.rsrp);
    return {
      logged_in: true,
      modem_state: 'modem_init_complete',
      network_type: 'LTE',
      provider: 'DEMO NETWORK',
      ppp_status: 'ppp_connected',
      signal_bars: rsrp >= -85 ? 5 : rsrp >= -95 ? 4 : rsrp >= -105 ? 3 : 2,
      rsrp,
      rsrq: Math.round(this.rsrq * 10) / 10,
      sinr: Math.round(this.sinr * 10) / 10,
      rssi: rsrp + 22,
      pci: c.pci,
      cell_id: c.cell_id,
      band: c.band,
      earfcn: c.earfcn,
      mcc: '001',
      mnc: '01',
      tac: '00FF',
      rx_bytes_per_s: Math.round(rx),
      tx_bytes_per_s: Math.round(tx),
      session_rx_bytes: Math.round(this.sessionRx),
      session_tx_bytes: Math.round(this.sessionTx),
      session_secs: Math.round(this.sessionSecs),
      month_rx_bytes: Math.round(this.monthRx),
      month_tx_bytes: Math.round(this.monthTx),
    };
  }

  private phone(): PhoneStatus {
    const c = CELLS[2];
    this.battery += this.charging ? 0.05 : -0.03;
    if (this.battery >= 100) this.charging = false;
    if (this.battery <= 25) this.charging = true;
    this.battery = Math.min(100, Math.max(1, this.battery));
    const clients: HotspotClient[] = [
      { ip: '192.0.2.130', mac: mac(130), iface: 'wlan1', state: 'REACHABLE', vendor: 'Demo Computers', label: 'Laptop (demo)' },
      { ip: '192.0.2.131', mac: mac(131), iface: 'wlan1', state: 'STALE', vendor: null, label: null },
    ];
    return {
      model: 'Demo Phone X',
      android: '14',
      transport: 'usb',
      battery_level: Math.round(this.battery),
      battery_temp_c: Math.round((31 + Math.sin(Date.now() / 60000) * 2 + (this.charging ? 2 : 0)) * 10) / 10,
      charging: this.charging,
      usb_powered: true,
      rsrp: Math.round(c.rsrp + gauss() * 2),
      rsrq: Math.round((c.rsrq + gauss() * 0.6) * 10) / 10,
      sinr: Math.round((c.sinr + gauss() * 1.5) * 10) / 10,
      rssi: Math.round(c.rsrp + 22 + gauss() * 2),
      pci: c.pci,
      cell_id: c.cell_id,
      tac: 255,
      earfcn: c.earfcn,
      band: c.band,
      mcc: '001',
      mnc: '01',
      hotspot_clients: clients,
    };
  }

  private event(kind: string, message: string, data: unknown = {}, ts = Date.now()) {
    this.emit({ type: 'event', ts, kind, message, data });
  }

  private handover(ts = Date.now(), emit = true) {
    const prev = this.cell();
    let next = this.cellIdx;
    while (next === this.cellIdx) next = Math.random() < 0.75 ? Math.floor(Math.random() * 2) : 2;
    this.cellIdx = next;
    const c = this.cell();
    this.rsrp += (c.rsrp - this.rsrp) * 0.5;
    this.sinr += (c.sinr - this.sinr) * 0.5;
    if (emit)
      this.event(
        'handover',
        `PCI ${prev.pci}→${c.pci} · eNB ${Math.floor(prev.cell_id / 256)}→${Math.floor(c.cell_id / 256)} · B${prev.band}→B${c.band}`,
        { from: { pci: prev.pci, cell_id: prev.cell_id, band: prev.band }, to: { pci: c.pci, cell_id: c.cell_id, band: c.band } },
        ts,
      );
  }

  /** Synthesize a few hours of history so charts are populated immediately. */
  private prefill(now: number) {
    const hours = 6;
    const start = now - hours * 3_600_000;
    const rows: Sample[] = [];
    let nextHo = start + (180 + Math.random() * 600) * 1000;
    this.cellIdx = 0;
    for (let t = start; t < now - 1000; t += 1000) {
      if (t >= nextHo) {
        this.handover(t, t > now - 50 * 60_000);
        nextHo = t + (180 + Math.random() * 900) * 1000;
      }
      this.step(1);
      const tel = this.telemetry();
      rows.push({
        ts: t,
        rsrp: tel.rsrp,
        rsrq: tel.rsrq,
        sinr: tel.sinr,
        rx_bytes_per_s: tel.rx_bytes_per_s,
        tx_bytes_per_s: tel.tx_bytes_per_s,
        pci: tel.pci,
        cell_id: tel.cell_id,
        band: tel.band,
      });
    }
    // Land back on the home cell so the live view starts on band 7.
    if (this.cellIdx !== 0) {
      const prev = this.cell();
      this.cellIdx = 0;
      const c = this.cell();
      const t = now - 2000;
      this.event(
        'handover',
        `PCI ${prev.pci}→${c.pci} · eNB ${Math.floor(prev.cell_id / 256)}→${Math.floor(c.cell_id / 256)} · B${prev.band}→B${c.band}`,
        { from: { pci: prev.pci, cell_id: prev.cell_id, band: prev.band }, to: { pci: c.pci, cell_id: c.cell_id, band: c.band } },
        t,
      );
    }
    this.sessionSecs = 3 * 3600 + 1234;
    samples.merge(rows);
    samples.coveredSince = start;
  }

  start() {
    const now = Date.now();
    this.prefill(now);
    this.devices = baseDevices(now);
    this.emit({ type: 'status', ts: now, data: this.sources });
    this.emit({ type: 'devices', ts: now, data: this.devices });
    this.event('router_online', 'router 192.0.2.1 reachable (demo)');
    this.event('phone_online', 'phone connected over usb: Demo Phone X (demo)', { transport: 'usb' });
    this.emit({ type: 'phone', ts: now, data: this.phone() });
    this.nextHandover = now + (90 + Math.random() * 150) * 1000;

    this.loopTelemetry();
    this.timers.push(window.setInterval(() => this.emit({ type: 'phone', ts: Date.now(), data: this.phone() }), 5000));
    this.timers.push(window.setInterval(() => this.churnDevices(), 45_000));
    this.timers.push(
      window.setInterval(() => {
        const t = Date.now();
        this.sources.router!.last_ok = t;
        this.sources.phone!.last_ok = t;
        this.emit({ type: 'status', ts: t, data: this.sources });
      }, 10_000),
    );
  }

  private loopTelemetry() {
    const tick = () => {
      const t = Date.now();
      if (t >= this.nextHandover) {
        this.handover(t);
        this.nextHandover = t + (120 + Math.random() * 240) * 1000;
      }
      this.step(this.interval);
      this.emit({ type: 'telemetry', ts: t, data: this.telemetry() });
      this.tickTimer = window.setTimeout(tick, this.interval * 1000);
    };
    tick();
  }

  private churnDevices() {
    const now = Date.now();
    if (this.extra) {
      const gone = this.extra;
      this.extra = null;
      this.devices = this.devices.filter((d) => d.ip !== gone.ip);
      this.event('device_leave', `${gone.hostname ?? gone.ip} left ${NET} (${gone.ip})`, gone);
    } else {
      const n = 60 + Math.floor(Math.random() * 30);
      this.extra = {
        network: NET,
        label: null,
        first_seen: now,
        last_seen: now,
        ip: `192.0.2.${n}`,
        mac: mac(n),
        vendor: Math.random() < 0.5 ? 'Apple' : null,
        hostname: `guest-${n}`,
        rtt_ms: 4 + Math.random() * 20,
        is_gateway: false,
        is_self: false,
        ports: [],
        upnp: [],
      };
      this.devices = [...this.devices, this.extra];
      this.event('device_join', `guest-${n} joined ${NET} (${this.extra.ip})`, this.extra);
    }
    this.emit({ type: 'devices', ts: now, data: this.devices });
  }

  async post(path: string, body: unknown): Promise<PostResult> {
    await new Promise((r) => setTimeout(r, 350));
    const ok = (message: string, extra: Record<string, unknown> = {}): PostResult => ({
      ok: true,
      status: 200,
      message,
      data: { ok: true, message, ...extra },
    });
    const b = (body ?? {}) as { action?: string; interval_s?: number };
    if (path === '/api/scan') {
      this.sources.scan = { ...this.sources.scan, running: true };
      this.emit({ type: 'status', ts: Date.now(), data: this.sources });
      this.timers.push(
        window.setTimeout(() => {
          this.sources.scan = { ...this.sources.scan, running: false, last_run: Date.now() };
          this.devices = this.devices.map((d) => ({ ...d, last_seen: Date.now() }));
          this.emit({ type: 'status', ts: Date.now(), data: this.sources });
          this.emit({ type: 'devices', ts: Date.now(), data: this.devices });
        }, 4000),
      );
      return ok('scan started (demo)');
    }
    if (path === '/api/router/action') {
      if (b.action === 'relogin') {
        this.event('login', 'router session reset; logging in again (demo)');
        return ok('router session reset; logging in on the next reading (demo)');
      }
      const message = `'${b.action}' is not implemented yet: it changes router settings and hasn't been tested on this router`;
      return { ok: false, status: 501, message, data: { ok: false, message } };
    }
    if (path === '/api/settings') {
      const v = Math.min(60, Math.max(0.5, Number(b.interval_s) || 1));
      this.interval = v;
      return ok(`interval set to ${v}s (demo)`, { interval_s: v });
    }
    return { ok: false, status: 404, message: 'unknown endpoint (demo)', data: null };
  }

  stop() {
    for (const t of this.timers) {
      window.clearTimeout(t);
      window.clearInterval(t);
    }
    this.timers = [];
    window.clearTimeout(this.tickTimer);
  }
}
