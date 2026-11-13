import { useRef } from 'react';
import { samples as routerSamples, type SampleBuffer } from '../lib/buffer';
import {
  collect,
  drawCrosshair,
  drawHandovers,
  drawTimeAxis,
  drawTooltip,
  FONT_MONO,
  handoverEvents,
  type Plot,
  tracePath,
  traceArea,
  tzOffset,
  type TipLine,
} from '../lib/chart';
import { fmtRate, fmtTime, prefersReducedMotion } from '../lib/format';
import { serverNow } from '../lib/rhcs';
import { useStore, WINDOW_MS } from '../lib/store';
import { useCanvasSize, useTick } from '../lib/ticker';
import { usePointer } from './usePointer';

const DOWN = [64, 214, 240];
const UP = [186, 150, 255];
const c = (rgb: number[], a: number) => `rgba(${rgb[0]},${rgb[1]},${rgb[2]},${a})`;

function niceMbps(v: number): number {
  const steps = [0.1, 0.25, 0.5, 1, 2, 5, 10, 20, 50, 100, 200, 500, 1000];
  return steps.find((s) => s >= v) ?? Math.ceil(v / 1000) * 1000;
}

/** Mirrored area chart: download above the axis, upload below. */
export function ThroughputChart({ height = 190, buffer }: { height?: number; buffer?: SampleBuffer }) {
  const samples = buffer ?? routerSamples;
  const canvas = useRef<HTMLCanvasElement>(null);
  const { size, version } = useCanvasSize(canvas);
  const pointer = usePointer(canvas);
  const st = useRef({ scale: 1, lastDraw: 0, lastVer: -1, lastSize: -1, lastPtr: '' });

  useTick((now, dt) => {
    const cv = canvas.current;
    if (!cv) return;
    const { w, h, dpr } = size.current;
    if (!w) return;
    const s = useStore.getState();
    const windowMs = WINDOW_MS[s.window];
    const m = st.current;
    const ptr = pointer.current;
    const ptrKey = ptr ? `${ptr.x | 0},${ptr.y | 0}` : '';
    const msPerPx = windowMs / w;
    const smooth = windowMs <= 600_000 && !prefersReducedMotion();
    const dirty = samples.version !== m.lastVer || version.current !== m.lastSize || ptrKey !== m.lastPtr;
    if (!smooth && !dirty && now - m.lastDraw < Math.max(250, msPerPx)) return;
    m.lastDraw = now;
    m.lastVer = samples.version;
    m.lastSize = version.current;
    m.lastPtr = ptrKey;

    const ctx = cv.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const t1 = serverNow();
    const t0 = t1 - windowMs;
    const p: Plot = { x: 56, y: 12, w: w - 64, h: h - 34 };
    const gapMs = Math.max(6000, msPerPx * 3);
    const rx = collect('tp-rx', 'rx', t0, t1, p, gapMs, true, samples);
    const tx = collect('tp-tx', 'tx', t0, t1, p, gapMs, true, samples);
    const peak = Math.max(Number.isFinite(rx.max) ? rx.max : 0, Number.isFinite(tx.max) ? tx.max : 0);
    const want = niceMbps(Math.max(0.1, (peak * 8) / 1e6) * 1.1);
    m.scale += (want - m.scale) * (smooth ? 1 - Math.exp(-dt * 4) : 1);
    const scaleBps = (m.scale * 1e6) / 8;
    const mid = p.y + p.h / 2;
    const half = p.h / 2 - 2;
    const yUp = (v: number) => mid - Math.min(1, v / scaleBps) * half;
    const yDown = (v: number) => mid + Math.min(1, v / scaleBps) * half;

    // Axis + labels.
    ctx.font = `10px ${FONT_MONO}`;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    const lab = (v: number) => (v >= 1 ? `${+v.toFixed(1)}M` : `${Math.round(v * 1000)}k`);
    for (const f of [1, 0.5]) {
      for (const sign of [-1, 1]) {
        const y = mid + sign * half * f;
        ctx.fillStyle = 'rgba(170,190,205,0.5)';
        ctx.fillText(lab(m.scale * f), p.x - 7, y);
        ctx.strokeStyle = 'rgba(160,200,230,0.045)';
        ctx.beginPath();
        ctx.moveTo(p.x, Math.round(y) + 0.5);
        ctx.lineTo(p.x + p.w, Math.round(y) + 0.5);
        ctx.stroke();
      }
    }
    ctx.fillStyle = 'rgba(170,190,205,0.6)';
    ctx.fillText('0', p.x - 7, mid);
    drawTimeAxis(ctx, p, t0, t1, tzOffset());
    ctx.strokeStyle = 'rgba(200,225,240,0.18)';
    ctx.beginPath();
    ctx.moveTo(p.x, Math.round(mid) + 0.5);
    ctx.lineTo(p.x + p.w, Math.round(mid) + 0.5);
    ctx.stroke();

    ctx.save();
    ctx.beginPath();
    ctx.rect(p.x, p.y - 2, p.w, p.h + 4);
    ctx.clip();
    const draw = (ser: typeof rx, rgb: number[], yOf: (v: number) => number, top: boolean) => {
      const g = ctx.createLinearGradient(0, top ? p.y : mid, 0, top ? mid : p.y + p.h);
      g.addColorStop(top ? 0 : 1, c(rgb, 0.45));
      g.addColorStop(top ? 1 : 0, c(rgb, 0.02));
      ctx.fillStyle = g;
      traceArea(ctx, ser, yOf, mid);
      ctx.fill();
      ctx.strokeStyle = c(rgb, 0.95);
      ctx.lineWidth = 1.3;
      ctx.shadowColor = c(rgb, 0.8);
      ctx.shadowBlur = 6;
      tracePath(ctx, ser, yOf);
      ctx.stroke();
      ctx.shadowBlur = 0;
    };
    draw(rx, DOWN, yUp, true);
    draw(tx, UP, yDown, false);
    ctx.restore();

    ctx.font = `600 10px ${FONT_MONO}`;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'top';
    ctx.fillStyle = c(DOWN, 0.9);
    ctx.fillText('▲ DOWN', p.x + 8, p.y + 4);
    ctx.textBaseline = 'bottom';
    ctx.fillStyle = c(UP, 0.9);
    ctx.fillText('▼ UP', p.x + 8, p.y + p.h - 4);

    const hit = drawHandovers(ctx, p, handoverEvents(s.events, t0, t1), t0, t1, ptr ? ptr.x : null);

    if (ptr && ptr.x >= p.x && ptr.x <= p.x + p.w) {
      const t = t0 + ((ptr.x - p.x) / p.w) * windowMs;
      drawCrosshair(ctx, p, ptr.x);
      const i = samples.nearest(t);
      const lines: TipLine[] = [];
      if (i >= 0 && Math.abs(samples.ts[i] - t) < Math.max(msPerPx * 2, 3000)) {
        const r = samples.cols.rx[i];
        const u = samples.cols.tx[i];
        lines.push({ text: fmtTime(samples.ts[i]), color: 'rgba(170,190,205,0.8)' });
        lines.push({ text: `▲ ${fmtRate(Number.isNaN(r) ? null : r)}`, color: c(DOWN, 1), bold: true });
        lines.push({ text: `▼ ${fmtRate(Number.isNaN(u) ? null : u)}`, color: c(UP, 1), bold: true });
      } else lines.push({ text: `${fmtTime(t)} · no sample`, color: 'rgba(170,190,205,0.8)' });
      if (hit) lines.push({ text: `⇄ ${hit.message ?? 'handover'}`, color: 'rgb(214,190,255)' });
      drawTooltip(ctx, ptr.x, ptr.y, lines, { x: 0, y: 0, w, h });
    }
  });

  return (
    <canvas
      ref={canvas}
      className="chart"
      style={{ height }}
      role="img"
      aria-label="Throughput chart: download above the axis, upload below, in megabits per second"
    />
  );
}
