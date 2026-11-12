import { useEffect, useRef } from 'react';
import { FONT_MONO, FONT_UI } from '../lib/chart';
import { prefersReducedMotion } from '../lib/format';
import { useStore } from '../lib/store';
import { useCanvasSize, useTick } from '../lib/ticker';
import type { DeviceView } from '../lib/types';
import { usePointer } from './usePointer';

export type Kind = 'gateway' | 'self' | 'phone' | 'unknown';

const PHONE_VENDORS =
  /samsung|apple|xiaomi|huawei|oppo|vivo|oneplus|google|motorola|tecno|infinix|itel|realme|nokia|honor|sony mobile|lg electronics|htc|meizu|zte|transsion|nothing/i;

export function kindOf(d: DeviceView): Kind {
  if (d.is_gateway) return 'gateway';
  if (d.is_self) return 'self';
  if (PHONE_VENDORS.test(`${d.vendor ?? ''} ${d.hostname ?? ''}`)) return 'phone';
  return 'unknown';
}

export const KIND_LABEL: Record<Kind, string> = {
  gateway: 'Gateway',
  self: 'This PC',
  phone: 'Phone / tablet',
  unknown: 'Other',
};

export const deviceKey = (d: DeviceView) => `${d.network}|${d.mac || d.ip}`;
export const deviceName = (d: DeviceView) => d.label || d.hostname || d.vendor || d.ip;

interface Node {
  d: DeviceView;
  appear: number;
  leave: number | null;
  flash: number;
  x: number;
  y: number;
  r: number;
}

function kindRgb(k: Kind, hue: number): string {
  switch (k) {
    case 'gateway':
      return `hsl(${hue} 85% 62%)`;
    case 'self':
      return 'rgb(225,238,255)';
    case 'phone':
      return 'rgb(186,150,255)';
    default:
      return 'rgb(128,150,170)';
  }
}

interface Props {
  network: string;
  devices: DeviceView[];
  onSelect: (d: DeviceView) => void;
  selected: string | null;
}

/** Animated radial map of one network: gateway at the centre, devices in orbit. */
export function Constellation({ network, devices, onSelect, selected }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const { size } = useCanvasSize(canvas);
  const pointer = usePointer(canvas);
  const nodes = useRef(new Map<string, Node>());
  const first = useRef(true);
  const focusKey = useRef<string | null>(null);
  const hoverKey = useRef<string | null>(null);
  const events = useStore((s) => s.events);
  const seenEvents = useRef(0);

  // Diff the device list into animated nodes.
  useEffect(() => {
    const now = performance.now();
    const map = nodes.current;
    const keys = new Set<string>();
    devices.forEach((d, i) => {
      const k = deviceKey(d);
      keys.add(k);
      const n = map.get(k);
      if (n) {
        n.d = d;
        if (n.leave !== null) {
          n.leave = null;
          n.appear = now;
        }
      } else {
        map.set(k, { d, appear: first.current ? now + i * 60 : now, leave: null, flash: first.current ? 0 : now, x: 0, y: 0, r: 0 });
      }
    });
    for (const [k, n] of map) if (!keys.has(k) && n.leave === null) n.leave = now;
    first.current = false;
  }, [devices]);

  // Flash on join/leave events for this network.
  useEffect(() => {
    const fresh = events.slice(seenEvents.current === 0 ? events.length : seenEvents.current);
    seenEvents.current = events.length;
    const now = performance.now();
    for (const e of fresh) {
      if (e.kind !== 'device_join' && e.kind !== 'device_leave') continue;
      const d = e.data as Partial<DeviceView> | null;
      if (!d || d.network !== network) continue;
      for (const n of nodes.current.values()) {
        if ((d.mac && n.d.mac === d.mac) || (d.ip && n.d.ip === d.ip)) {
          n.flash = now;
          if (e.kind === 'device_leave') n.leave = n.leave ?? now;
        }
      }
    }
  }, [events, network]);

  useTick((now) => {
    const c = canvas.current;
    if (!c) return;
    const ctx = c.getContext('2d');
    if (!ctx) return;
    const { w, h, dpr } = size.current;
    if (!w) return;
    const reduce = prefersReducedMotion();
    const hue = useStore.getState().prefs.accentHue;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const cx = w / 2;
    const cy = h / 2;
    const R = Math.min(w, h) / 2 - 34;
    const time = reduce ? 0 : now / 1000;

    // Drop finished leave animations.
    for (const [k, n] of nodes.current) if (n.leave !== null && now - n.leave > 1400) nodes.current.delete(k);
    const all = [...nodes.current.values()];
    const hub = all.find((n) => n.d.is_gateway) ?? null;
    const orbiters = all.filter((n) => n !== hub).sort((a, b) => a.d.ip.localeCompare(b.d.ip, undefined, { numeric: true }));

    // Orbit rings.
    const rings: Node[][] = [];
    let idx = 0;
    for (let ring = 0; idx < orbiters.length; ring++) {
      const cap = 6 + ring * 6;
      rings.push(orbiters.slice(idx, idx + cap));
      idx += cap;
    }
    const ringCount = Math.max(1, rings.length);
    for (let i = 0; i < ringCount; i++) {
      const rr = R * (0.42 + (0.58 * (i + 1)) / ringCount);
      ctx.strokeStyle = 'rgba(160,200,230,0.07)';
      ctx.setLineDash([2, 6]);
      ctx.beginPath();
      ctx.arc(cx, cy, rr, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);
    }

    const life = (n: Node) => {
      const a = Math.max(0, Math.min(1, (now - n.appear) / 700));
      const easeA = 1 - Math.pow(1 - a, 3);
      const l = n.leave === null ? 1 : Math.max(0, 1 - (now - n.leave) / 1200);
      return reduce ? (n.leave === null ? 1 : 0) : easeA * l;
    };
    const radius = (n: Node) => 5 + Math.min(9, (n.d.ports?.length ?? 0) * 1.8);

    // Positions.
    if (hub) {
      hub.x = cx;
      hub.y = cy;
      hub.r = radius(hub) + 6;
    }
    rings.forEach((ring, ri) => {
      const rr = R * (0.42 + (0.58 * (ri + 1)) / ringCount);
      const dir = ri % 2 ? -1 : 1;
      ring.forEach((n, i) => {
        const ang = (i / ring.length) * Math.PI * 2 + dir * time * (0.035 / (ri + 1)) + ri * 0.4;
        const wob = reduce ? 0 : Math.sin(time * 0.7 + i * 1.7) * 3;
        n.x = cx + Math.cos(ang) * (rr + wob);
        n.y = cy + Math.sin(ang) * (rr + wob);
        n.r = radius(n);
      });
    });

    // Hit testing.
    const ptr = pointer.current;
    let hover: Node | null = null;
    if (ptr) {
      let best = Infinity;
      for (const n of all) {
        const d = Math.hypot(n.x - ptr.x, n.y - ptr.y);
        if (d < n.r + 10 && d < best) {
          best = d;
          hover = n;
        }
      }
    }
    hoverKey.current = hover ? deviceKey(hover.d) : null;
    c.style.cursor = hover ? 'pointer' : 'default';

    // Links + packets.
    const ox = hub ? hub.x : cx;
    const oy = hub ? hub.y : cy;
    for (const n of orbiters) {
      const a = life(n);
      if (a <= 0) continue;
      const col = kindRgb(kindOf(n.d), hue);
      ctx.globalAlpha = 0.16 * a;
      ctx.strokeStyle = col;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(ox, oy);
      ctx.lineTo(n.x, n.y);
      ctx.stroke();
      if (!reduce && n.leave === null) {
        const rtt = n.d.rtt_ms ?? 20;
        const period = 1.6 + Math.min(4, rtt / 15);
        const ph = ((time + n.d.ip.length * 0.37 + (n.d.ip.charCodeAt(n.d.ip.length - 1) % 7) * 0.3) % period) / period;
        const px = ox + (n.x - ox) * ph;
        const py = oy + (n.y - oy) * ph;
        ctx.globalAlpha = 0.7 * a * Math.sin(ph * Math.PI);
        ctx.fillStyle = col;
        ctx.beginPath();
        ctx.arc(px, py, 1.6, 0, Math.PI * 2);
        ctx.fill();
      }
    }
    ctx.globalAlpha = 1;

    // Hub placeholder when no gateway was found.
    if (!hub) {
      ctx.strokeStyle = 'rgba(160,200,230,0.25)';
      ctx.beginPath();
      ctx.arc(cx, cy, 10, 0, Math.PI * 2);
      ctx.stroke();
    }

    // Nodes.
    const sel = selected;
    for (const n of all) {
      const a = life(n);
      if (a <= 0) continue;
      const k = kindOf(n.d);
      const col = kindRgb(k, hue);
      const key = deviceKey(n.d);
      const hl = key === hoverKey.current || key === sel || key === focusKey.current;
      const r = n.r * (0.4 + 0.6 * a) * (hl ? 1.15 : 1);
      // Join flash ring.
      const fl = n.flash ? (now - n.flash) / 1400 : 2;
      if (fl < 1 && !reduce) {
        ctx.strokeStyle = col;
        ctx.globalAlpha = (1 - fl) * 0.8;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(n.x, n.y, r + fl * 36, 0, Math.PI * 2);
        ctx.stroke();
      }
      ctx.globalAlpha = a;
      ctx.save();
      ctx.shadowColor = col;
      ctx.shadowBlur = hl ? 22 : k === 'gateway' ? 18 : 10;
      const g = ctx.createRadialGradient(n.x - r * 0.3, n.y - r * 0.3, r * 0.1, n.x, n.y, r);
      g.addColorStop(0, 'rgba(255,255,255,0.95)');
      g.addColorStop(0.35, col);
      g.addColorStop(1, col);
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(n.x, n.y, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
      if (hl) {
        ctx.strokeStyle = col;
        ctx.lineWidth = 1.2;
        ctx.beginPath();
        ctx.arc(n.x, n.y, r + 5, 0, Math.PI * 2);
        ctx.stroke();
      }
      // Label.
      ctx.font = `${hl ? 600 : 500} 11px ${FONT_UI}`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      ctx.fillStyle = hl ? 'rgba(235,244,250,0.95)' : 'rgba(200,215,228,0.7)';
      const name = deviceName(n.d);
      ctx.fillText(name.length > 22 ? `${name.slice(0, 21)}…` : name, n.x, n.y + r + 5);
      if (hl || k === 'gateway') {
        ctx.font = `10px ${FONT_MONO}`;
        ctx.fillStyle = 'rgba(170,190,205,0.6)';
        ctx.fillText(n.d.ip, n.x, n.y + r + 19);
      }
      ctx.globalAlpha = 1;
    }
  });

  return (
    <div className="constellation">
      <canvas
        ref={canvas}
        className="constellation__canvas"
        role="img"
        aria-label={`Map of ${devices.length} devices on ${network}`}
        onClick={() => {
          const k = hoverKey.current;
          const n = k ? nodes.current.get(k) : null;
          if (n && n.leave === null) onSelect(n.d);
        }}
      />
      <ul className="sr-only-focusable" aria-label={`Devices on ${network}`}>
        {devices.map((d) => (
          <li key={deviceKey(d)}>
            <button
              type="button"
              onFocus={() => (focusKey.current = deviceKey(d))}
              onBlur={() => (focusKey.current = null)}
              onClick={() => onSelect(d)}
            >
              {deviceName(d)} ({KIND_LABEL[kindOf(d)]}, {d.ip})
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
