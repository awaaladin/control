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
  niceStep,
  type Plot,
  tracePath,
  traceArea,
  tzOffset,
  type TipLine,
} from '../lib/chart';
import {
  bandLabel,
  fmt,
  fmtTime,
  grade,
  GRADE_LABEL,
  gradeColor,
  LABELS,
  type Metric,
  prefersReducedMotion,
  RANGES,
  THRESHOLDS,
  UNITS,
} from '../lib/format';
import { serverNow } from '../lib/rhcs';
import { isStale, STALE_MS, useStore, WINDOW_MS } from '../lib/store';
import { useCanvasSize, useTick } from '../lib/ticker';
import { usePointer } from './usePointer';

interface Props {
  metric: Metric;
  height?: number;
  /** Which readings to draw (default: the router's). */
  buffer?: SampleBuffer;
  /** Whose freshness decides the "stale" look. */
  source?: 'router' | 'phone';
}

/** Live scrolling line chart for one signal metric, drawn on canvas each frame. */
export function SeriesChart({ metric, height = 170, buffer, source = 'router' }: Props) {
  const samples = buffer ?? routerSamples;
  const canvas = useRef<HTMLCanvasElement>(null);
  const { size, version } = useCanvasSize(canvas);
  const pointer = usePointer(canvas);
  const st = useRef({ lo: RANGES[metric][0], hi: RANGES[metric][1], lastDraw: 0, lastVer: -1, lastSize: -1, lastAria: '', lastPtr: '' });
  const digits = metric === 'rsrp' ? 0 : 1;
  const unit = UNITS[metric];
  const [e, g, f] = THRESHOLDS[metric];

  useTick((now, dt) => {
    const c = canvas.current;
    if (!c) return;
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
    if (prefersReducedMotion() && !dirty && now - m.lastDraw < 1000) return;
    m.lastDraw = now;
    m.lastVer = samples.version;
    m.lastSize = version.current;
    m.lastPtr = ptrKey;

    const ctx = c.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const t1 = serverNow();
    const t0 = t1 - windowMs;
    const p: Plot = { x: 40, y: 12, w: w - 48, h: h - 34 };
    const gapMs = Math.max(6000, msPerPx * 3);
    const ser = collect(`s-${source}-${metric}`, metric, t0, t1, p, gapMs, false, samples);

    // Domain: the metric's nominal range, widened to fit the data, tweened.
    const [r0, r1] = RANGES[metric];
    const wantLo = Number.isFinite(ser.min) ? Math.min(r0, Math.floor(ser.min - 2)) : r0;
    const wantHi = Number.isFinite(ser.max) ? Math.max(r1, Math.ceil(ser.max + 2)) : r1;
    const k = smooth ? 1 - Math.exp(-dt * 5) : 1;
    m.lo += (wantLo - m.lo) * k;
    m.hi += (wantHi - m.hi) * k;
    const lo = m.lo;
    const hi = m.hi;
    const yOf = (v: number) => p.y + p.h - ((v - lo) / (hi - lo)) * p.h;
    const clampY = (v: number) => Math.max(p.y, Math.min(p.y + p.h, yOf(v)));

    // Grade bands.
    const bands: [number, number, ReturnType<typeof grade>][] = [
      [e, hi, 'excellent'],
      [g, e, 'good'],
      [f, g, 'fair'],
      [lo, f, 'poor'],
    ];
    for (const [a, b, gr] of bands) {
      const ya = clampY(Math.max(a, lo));
      const yb = clampY(Math.min(b, hi));
      if (ya - yb <= 0) continue;
      ctx.fillStyle = gradeColor(gr, 0.045);
      ctx.fillRect(p.x, yb, p.w, ya - yb);
    }
    ctx.save();
    ctx.setLineDash([2, 5]);
    for (const th of [e, g, f]) {
      if (th <= lo || th >= hi) continue;
      const y = Math.round(yOf(th)) + 0.5;
      ctx.strokeStyle = gradeColor(grade(metric, th), 0.3);
      ctx.beginPath();
      ctx.moveTo(p.x, y);
      ctx.lineTo(p.x + p.w, y);
      ctx.stroke();
    }
    ctx.restore();

    // Y labels.
    const step = niceStep(hi - lo, Math.max(2, Math.floor(p.h / 34)));
    ctx.font = `10px ${FONT_MONO}`;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    for (let v = Math.ceil(lo / step) * step; v <= hi; v += step) {
      const y = yOf(v);
      ctx.fillStyle = 'rgba(170,190,205,0.5)';
      ctx.fillText(fmt(v, step < 1 ? 1 : 0), p.x - 7, y);
      ctx.strokeStyle = 'rgba(160,200,230,0.045)';
      ctx.beginPath();
      ctx.moveTo(p.x, Math.round(y) + 0.5);
      ctx.lineTo(p.x + p.w, Math.round(y) + 0.5);
      ctx.stroke();
    }
    drawTimeAxis(ctx, p, t0, t1, tzOffset());

    // Series (clipped to plot).
    ctx.save();
    ctx.beginPath();
    ctx.rect(p.x, p.y - 4, p.w, p.h + 8);
    ctx.clip();

    const stroke = ctx.createLinearGradient(0, p.y, 0, p.y + p.h);
    const stop = (v: number) => Math.max(0, Math.min(1, (yOf(v) - p.y) / p.h));
    const addStops = (gr: CanvasGradient, a: number) => {
      const pts: [number, string][] = [
        [0, gradeColor('excellent', a)],
        [stop(e), gradeColor('excellent', a)],
        [stop(e), gradeColor('good', a)],
        [stop(g), gradeColor('good', a)],
        [stop(g), gradeColor('fair', a)],
        [stop(f), gradeColor('fair', a)],
        [stop(f), gradeColor('poor', a)],
        [1, gradeColor('poor', a)],
      ];
      for (const [o, col] of pts) gr.addColorStop(o, col);
    };
    addStops(stroke, 1);
    const fill = ctx.createLinearGradient(0, p.y, 0, p.y + p.h);
    addStops(fill, 0.1);

    if (ser.bucketed) {
      ctx.globalAlpha = 0.25;
      ctx.strokeStyle = stroke;
      ctx.lineWidth = 1;
      for (let i = 0; i < ser.n; i++) {
        if (Number.isNaN(ser.lo[i])) continue;
        ctx.beginPath();
        ctx.moveTo(ser.xs[i], yOf(ser.lo[i]));
        ctx.lineTo(ser.xs[i], yOf(ser.hi[i]) - 0.5);
        ctx.stroke();
      }
      ctx.globalAlpha = 1;
    }
    ctx.fillStyle = fill;
    traceArea(ctx, ser, yOf, p.y + p.h);
    ctx.fill();

    ctx.lineJoin = 'round';
    ctx.lineCap = 'round';
    ctx.strokeStyle = stroke;
    ctx.globalAlpha = 0.18;
    ctx.lineWidth = 6;
    tracePath(ctx, ser, yOf);
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.lineWidth = 1.6;
    ctx.stroke();

    // Live head.
    const last = samples.len - 1;
    const stale = s.conn !== 'demo' && (source === 'phone' ? isStale(s.phoneAt, STALE_MS.phone) : isStale(s.telemetryAt, STALE_MS.telemetry));
    if (last >= 0 && samples.ts[last] >= t0) {
      const v = samples.cols[metric][last];
      if (!Number.isNaN(v)) {
        const x = p.x + ((samples.ts[last] - t0) / windowMs) * p.w;
        const y = yOf(v);
        const pulse = stale || !smooth ? 0.5 : 0.5 + 0.5 * Math.sin(now / 320);
        ctx.fillStyle = gradeColor(stale ? 'none' : grade(metric, v), 1);
        ctx.shadowColor = ctx.fillStyle;
        ctx.shadowBlur = 6 + 8 * pulse;
        ctx.beginPath();
        ctx.arc(x, y, 3, 0, Math.PI * 2);
        ctx.fill();
        ctx.shadowBlur = 0;
      }
    }
    ctx.restore();

    const hos = handoverEvents(s.events, t0, t1);
    const hit = drawHandovers(ctx, p, hos, t0, t1, ptr ? ptr.x : null);

    // Crosshair + tooltip.
    if (ptr && ptr.x >= p.x && ptr.x <= p.x + p.w) {
      const t = t0 + ((ptr.x - p.x) / p.w) * windowMs;
      const i = samples.nearest(t);
      drawCrosshair(ctx, p, ptr.x);
      const lines: TipLine[] = [];
      if (i >= 0 && Math.abs(samples.ts[i] - t) < Math.max(msPerPx * 2, 3000)) {
        const v = samples.cols[metric][i];
        const gr = grade(metric, Number.isNaN(v) ? null : v);
        lines.push({ text: fmtTime(samples.ts[i]), color: 'rgba(170,190,205,0.8)' });
        lines.push({ text: `${LABELS[metric]} ${fmt(Number.isNaN(v) ? null : v, digits)} ${unit}`, color: gradeColor(gr), bold: true });
        lines.push({ text: GRADE_LABEL[gr], color: gradeColor(gr, 0.8) });
        const pci = samples.cols.pci[i];
        const band = samples.cols.band[i];
        lines.push({ text: `PCI ${fmt(Number.isNaN(pci) ? null : pci)} · ${bandLabel(Number.isNaN(band) ? null : band)}`, color: 'rgba(170,190,205,0.8)' });
        if (!Number.isNaN(v)) {
          ctx.fillStyle = gradeColor(gr);
          ctx.beginPath();
          ctx.arc(p.x + ((samples.ts[i] - t0) / windowMs) * p.w, yOf(v), 4, 0, Math.PI * 2);
          ctx.fill();
        }
      } else lines.push({ text: `${fmtTime(t)} · no sample`, color: 'rgba(170,190,205,0.8)' });
      if (hit) lines.push({ text: `⇄ ${hit.message ?? 'handover'}`, color: 'rgb(214,190,255)' });
      drawTooltip(ctx, ptr.x, ptr.y, lines, { x: 0, y: 0, w, h });
    }

    // Accessible summary, refreshed only when it changes.
    const lv = last >= 0 ? samples.cols[metric][last] : NaN;
    const aria = `${LABELS[metric]} chart, last ${s.window}. Latest ${fmt(Number.isNaN(lv) ? null : lv, digits)} ${unit}; range ${fmt(Number.isFinite(ser.min) ? ser.min : null, digits)} to ${fmt(Number.isFinite(ser.max) ? ser.max : null, digits)}; ${hos.length} handovers.`;
    if (aria !== m.lastAria) {
      c.setAttribute('aria-label', aria);
      m.lastAria = aria;
    }
  });

  return <canvas ref={canvas} className="chart" style={{ height }} role="img" aria-label={`${LABELS[metric]} chart`} />;
}
