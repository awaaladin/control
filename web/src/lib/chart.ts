// Shared canvas chart helpers.

import { samples as routerSamples, type Field, type SampleBuffer } from './buffer';
import { fmtTime } from './format';
import type { RhcsEvent } from './types';

export const FONT_MONO = '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
export const FONT_UI = '"Space Grotesk", system-ui, sans-serif';

export interface Plot {
  x: number;
  y: number;
  w: number;
  h: number;
}

export function niceStep(span: number, target: number): number {
  const raw = span / Math.max(1, target);
  const p = Math.pow(10, Math.floor(Math.log10(raw)));
  const m = raw / p;
  const n = m < 1.5 ? 1 : m < 3 ? 2 : m < 7 ? 5 : 10;
  return n * p;
}

const TIME_STEPS = [5e3, 10e3, 15e3, 30e3, 60e3, 120e3, 300e3, 600e3, 900e3, 1800e3, 3600e3, 7200e3, 10800e3, 21600e3];

export function drawTimeAxis(ctx: CanvasRenderingContext2D, p: Plot, t0: number, t1: number, tzOffsetMs: number) {
  const span = t1 - t0;
  const maxLabels = Math.max(2, Math.floor(p.w / 90));
  const step = TIME_STEPS.find((s) => span / s <= maxLabels) ?? 21600e3;
  const withSec = step < 60e3;
  ctx.font = `10px ${FONT_MONO}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  // Align ticks to local wall-clock boundaries.
  let t = Math.ceil((t0 - tzOffsetMs) / step) * step + tzOffsetMs;
  for (; t <= t1; t += step) {
    const x = p.x + ((t - t0) / span) * p.w;
    ctx.strokeStyle = 'rgba(160,200,230,0.06)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(Math.round(x) + 0.5, p.y);
    ctx.lineTo(Math.round(x) + 0.5, p.y + p.h);
    ctx.stroke();
    if (x > p.x + 18 && x < p.x + p.w - 18) {
      ctx.fillStyle = 'rgba(170,190,205,0.55)';
      ctx.fillText(fmtTime(t, withSec), x, p.y + p.h + 6);
    }
  }
}

export const tzOffset = () => new Date().getTimezoneOffset() * 60_000;

export interface Series {
  xs: Float32Array;
  ys: Float32Array;
  lo: Float32Array;
  hi: Float32Array;
  n: number;
  /** True when several samples were folded into each point. */
  bucketed: boolean;
  min: number;
  max: number;
}

const pool = new Map<string, Series>();
function alloc(key: string, n: number): Series {
  let s = pool.get(key);
  if (!s || s.xs.length < n) {
    s = {
      xs: new Float32Array(n),
      ys: new Float32Array(n),
      lo: new Float32Array(n),
      hi: new Float32Array(n),
      n: 0,
      bucketed: false,
      min: Infinity,
      max: -Infinity,
    };
    pool.set(key, s);
  }
  s.n = 0;
  s.min = Infinity;
  s.max = -Infinity;
  return s;
}

/**
 * Collects the samples of `field` between t0 and t1 as plot-space x positions.
 * Gaps (missing values or time gaps) are encoded as NaN y. When there are
 * more samples than pixels, values are folded into per-pixel buckets
 * (mean for the line, min/max for the envelope, or max with `peak`).
 */
export function collect(key: string, field: Field, t0: number, t1: number, p: Plot, gapMs: number, peak = false, samples: SampleBuffer = routerSamples): Series {
  const span = t1 - t0;
  const i0 = Math.max(0, samples.lowerBound(t0) - 1);
  const i1 = samples.len;
  const count = i1 - i0;
  const cols = Math.max(1, Math.floor(p.w));
  const col = samples.cols[field];
  const ts = samples.ts;
  if (count <= cols * 1.5) {
    const s = alloc(key, count * 2 + 2);
    let prevT = -Infinity;
    for (let i = i0; i < i1; i++) {
      const v = col[i];
      const t = ts[i];
      if (t - prevT > gapMs && s.n > 0) {
        s.xs[s.n] = NaN;
        s.ys[s.n] = NaN;
        s.n++;
      }
      prevT = t;
      s.xs[s.n] = p.x + ((t - t0) / span) * p.w;
      s.ys[s.n] = v;
      s.lo[s.n] = v;
      s.hi[s.n] = v;
      s.n++;
      if (!Number.isNaN(v) && t >= t0) {
        if (v < s.min) s.min = v;
        if (v > s.max) s.max = v;
      }
    }
    s.bucketed = false;
    return s;
  }
  const s = alloc(key, cols + 2);
  let b = -1;
  let sum = 0;
  let cnt = 0;
  let lo = Infinity;
  let hi = -Infinity;
  let lastT = -Infinity;
  const flush = () => {
    if (b < 0) return;
    s.xs[s.n] = p.x + b + 0.5;
    if (cnt > 0) {
      s.ys[s.n] = peak ? hi : sum / cnt;
      s.lo[s.n] = lo;
      s.hi[s.n] = hi;
      if (lo < s.min) s.min = lo;
      if (hi > s.max) s.max = hi;
    } else {
      s.ys[s.n] = NaN;
      s.lo[s.n] = NaN;
      s.hi[s.n] = NaN;
    }
    s.n++;
  };
  for (let i = i0; i < i1; i++) {
    const t = ts[i];
    const nb = Math.floor(((t - t0) / span) * p.w);
    if (nb !== b) {
      flush();
      if (b >= 0 && t - lastT > gapMs) {
        s.xs[s.n] = NaN;
        s.ys[s.n] = NaN;
        s.n++;
      }
      b = nb;
      sum = 0;
      cnt = 0;
      lo = Infinity;
      hi = -Infinity;
    }
    lastT = t;
    const v = col[i];
    if (!Number.isNaN(v)) {
      sum += v;
      cnt++;
      if (v < lo) lo = v;
      if (v > hi) hi = v;
    }
  }
  flush();
  s.bucketed = true;
  return s;
}

/** Traces the series as a path, breaking on NaN. Returns the number of segments. */
export function tracePath(ctx: CanvasRenderingContext2D, s: Series, yOf: (v: number) => number, which: 'ys' | 'lo' | 'hi' = 'ys') {
  const arr = s[which];
  let pen = false;
  ctx.beginPath();
  for (let i = 0; i < s.n; i++) {
    const v = arr[i];
    const x = s.xs[i];
    if (Number.isNaN(v) || Number.isNaN(x)) {
      pen = false;
      continue;
    }
    const y = yOf(v);
    if (!pen) {
      ctx.moveTo(x, y);
      pen = true;
    } else ctx.lineTo(x, y);
  }
}

/** Closed area between the series and a baseline, per contiguous run. */
export function traceArea(ctx: CanvasRenderingContext2D, s: Series, yOf: (v: number) => number, baseY: number) {
  ctx.beginPath();
  let runStart = -1;
  let lastX = 0;
  for (let i = 0; i <= s.n; i++) {
    const v = i < s.n ? s.ys[i] : NaN;
    const x = i < s.n ? s.xs[i] : NaN;
    const ok = !Number.isNaN(v) && !Number.isNaN(x);
    if (ok) {
      const y = yOf(v);
      if (runStart < 0) {
        ctx.moveTo(x, baseY);
        ctx.lineTo(x, y);
        runStart = i;
      } else ctx.lineTo(x, y);
      lastX = x;
    } else if (runStart >= 0) {
      ctx.lineTo(lastX, baseY);
      ctx.closePath();
      runStart = -1;
    }
  }
}

export function handoverEvents(events: RhcsEvent[], t0: number, t1: number): RhcsEvent[] {
  const out: RhcsEvent[] = [];
  for (const e of events) if (e.kind === 'handover' && e.ts >= t0 && e.ts <= t1) out.push(e);
  return out;
}

export function drawHandovers(ctx: CanvasRenderingContext2D, p: Plot, evs: RhcsEvent[], t0: number, t1: number, hoverX: number | null): RhcsEvent | null {
  const span = t1 - t0;
  let hit: RhcsEvent | null = null;
  for (const e of evs) {
    const x = Math.round(p.x + ((e.ts - t0) / span) * p.w) + 0.5;
    const near = hoverX !== null && Math.abs(hoverX - x) < 7;
    if (near) hit = e;
    ctx.save();
    ctx.strokeStyle = near ? 'rgba(200,170,255,0.95)' : 'rgba(186,150,255,0.55)';
    ctx.setLineDash([3, 4]);
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x, p.y);
    ctx.lineTo(x, p.y + p.h);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = near ? 'rgb(214,190,255)' : 'rgba(186,150,255,0.9)';
    ctx.shadowColor = 'rgba(186,150,255,0.9)';
    ctx.shadowBlur = 8;
    ctx.beginPath();
    ctx.moveTo(x, p.y + 1);
    ctx.lineTo(x + 4, p.y + 5);
    ctx.lineTo(x, p.y + 9);
    ctx.lineTo(x - 4, p.y + 5);
    ctx.closePath();
    ctx.fill();
    ctx.restore();
  }
  return hit;
}

export interface TipLine {
  text: string;
  color?: string;
  bold?: boolean;
}

export function drawTooltip(ctx: CanvasRenderingContext2D, x: number, y: number, lines: TipLine[], bounds: Plot) {
  ctx.font = `11px ${FONT_MONO}`;
  const lh = 16;
  const pad = 8;
  let w = 0;
  for (const l of lines) w = Math.max(w, ctx.measureText(l.text).width);
  w += pad * 2;
  const h = lines.length * lh + pad * 2 - 4;
  let bx = x + 14;
  if (bx + w > bounds.x + bounds.w) bx = x - 14 - w;
  let by = y - h / 2;
  by = Math.max(bounds.y, Math.min(bounds.y + bounds.h - h, by));
  ctx.save();
  ctx.fillStyle = 'rgba(8,12,18,0.92)';
  ctx.strokeStyle = 'rgba(150,200,230,0.22)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(bx + 0.5, by + 0.5, w, h, 6);
  ctx.fill();
  ctx.stroke();
  ctx.textAlign = 'left';
  ctx.textBaseline = 'top';
  lines.forEach((l, i) => {
    ctx.font = `${l.bold ? '600 ' : ''}11px ${FONT_MONO}`;
    ctx.fillStyle = l.color ?? 'rgba(220,232,242,0.9)';
    ctx.fillText(l.text, bx + pad, by + pad + i * lh);
  });
  ctx.restore();
}

export function drawCrosshair(ctx: CanvasRenderingContext2D, p: Plot, x: number) {
  ctx.save();
  ctx.strokeStyle = 'rgba(220,235,245,0.35)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(Math.round(x) + 0.5, p.y);
  ctx.lineTo(Math.round(x) + 0.5, p.y + p.h);
  ctx.stroke();
  ctx.restore();
}
