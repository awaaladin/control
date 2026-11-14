import { useRef } from 'react';
import { useCanvasSize, useTick } from '../lib/ticker';
import {
  GRADE_LABEL,
  GRADE_RGB,
  type Grade,
  isNum,
  makeScorer,
  norm,
  prefersReducedMotion,
  scoreRGB,
} from '../lib/format';

export interface GlowRingProps {
  label: string;
  value: number | null | undefined;
  range: [number, number];
  unit: string;
  digits?: number;
  /** Grade the value; also drives colour. */
  gradeOf: (v: number | null | undefined) => Grade;
  /** Threshold values drawn as ticks on the dial (excellent, good, fair). */
  thresholds?: number[];
  /** Returns true when the source has stopped streaming. */
  isStale: () => boolean;
  size?: 'sm' | 'md' | 'lg' | 'xl';
  sub?: string;
  className?: string;
}

const START = Math.PI * 0.75;
const SWEEP = Math.PI * 1.5;
const NONE = GRADE_RGB.none;

type RGB = [number, number, number];
const mix = (a: RGB, b: RGB, t: number): RGB => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
const rgba = (c: RGB, a: number) => `rgba(${c[0] | 0},${c[1] | 0},${c[2] | 0},${a})`;

/**
 * Circular gauge with bloom. Arc length, colour and glow follow the value;
 * it breathes while data streams and freezes, dimmed, when the source is stale.
 */
export function GlowRing({
  label,
  value,
  range,
  unit,
  digits = 0,
  gradeOf,
  thresholds = [],
  isStale,
  size = 'md',
  sub,
  className,
}: GlowRingProps) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const numRef = useRef<HTMLSpanElement>(null);
  const gradeRef = useRef<HTMLSpanElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const { size: dims } = useCanvasSize(canvas);
  const anim = useRef({ shown: NaN, color: NONE as RGB, dim: 0, phase: 0, text: '', grade: '' as string, frozenBreath: 1, css: '' });

  const target = isNum(value) ? value : NaN;
  const grade = gradeOf(value);
  // Continuous colour: smooth green → yellow → orange → red by how good the reading is.
  // No reading, no colour (neutral grey).
  const scorer = makeScorer(gradeOf, range);
  const score = isNum(value) ? scorer(value) : null;

  useTick((_now, dt) => {
    const c = canvas.current;
    if (!c) return;
    const ctx = c.getContext('2d');
    if (!ctx) return;
    const { w, h, dpr } = dims.current;
    if (!w) return;
    const a = anim.current;
    const reduce = prefersReducedMotion();
    const stale = isStale();

    // Tween value, colour and dimming.
    const k = reduce ? 1 : 1 - Math.exp(-dt * 6);
    if (Number.isNaN(target)) a.shown = NaN;
    else if (Number.isNaN(a.shown)) a.shown = target;
    else a.shown += (target - a.shown) * k;
    const gc = (score === null ? NONE : scoreRGB(score)) as RGB;
    a.color = mix(a.color, gc, reduce ? 1 : 1 - Math.exp(-dt * 4));
    a.dim += ((stale ? 1 : 0) - a.dim) * (reduce ? 1 : 1 - Math.exp(-dt * 3));
    if (!stale && !reduce) a.phase += dt;
    const breath = reduce ? 1 : 0.8 + 0.2 * Math.sin((a.phase / 3.2) * Math.PI * 2);

    // DOM text (only touched when the visible string changes).
    const text = Number.isNaN(a.shown) ? '—' : a.shown.toFixed(digits).replace('-', '−');
    if (text !== a.text && numRef.current) {
      numRef.current.textContent = text;
      a.text = text;
    }
    const gText = stale && !Number.isNaN(a.shown) ? 'Stale' : GRADE_LABEL[grade];
    if (gText !== a.grade && gradeRef.current) {
      gradeRef.current.textContent = gText;
      a.grade = gText;
    }
    if (wrapRef.current) {
      // "Stale" only makes sense when there was a reading; with none the dial just says No data, in grey.
      wrapRef.current.dataset.grade = stale && !Number.isNaN(a.shown) ? 'stale' : grade;
      // The number's glow and the grade word follow the live colour (inline value beats the grade rule).
      const css = Number.isNaN(a.shown) ? '' : `rgb(${a.color[0] | 0},${a.color[1] | 0},${a.color[2] | 0})`;
      if (css !== a.css) {
        if (css) wrapRef.current.style.setProperty('--gc', css);
        else wrapRef.current.style.removeProperty('--gc');
        a.css = css;
      }
    }

    // Draw.
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const S = Math.min(w, h);
    const cx = w / 2;
    const cy = h / 2;
    const r = S * 0.38;
    const lw = Math.max(3, S * 0.045);
    const col = mix(a.color, NONE, a.dim * 0.75);
    const alive = 1 - a.dim * 0.55;
    const t = Number.isNaN(a.shown) ? 0 : norm(a.shown, range);
    const glow = (0.35 + 0.65 * t) * breath * alive;

    ctx.lineCap = 'round';

    // Ambient bloom disc.
    const halo = ctx.createRadialGradient(cx, cy, r * 0.2, cx, cy, r * 1.35);
    halo.addColorStop(0, rgba(col, 0.1 * glow));
    halo.addColorStop(0.65, rgba(col, 0.05 * glow));
    halo.addColorStop(1, rgba(col, 0));
    ctx.fillStyle = halo;
    ctx.fillRect(0, 0, w, h);

    // Tick ring.
    const ticks = size === 'sm' ? 27 : 54;
    for (let i = 0; i <= ticks; i++) {
      const ang = START + (SWEEP * i) / ticks;
      const major = i % 9 === 0;
      const r1 = r + lw * 1.25;
      const r2 = r1 + (major ? lw * 0.9 : lw * 0.45);
      ctx.strokeStyle = `rgba(190,215,235,${major ? 0.28 : 0.12})`;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(cx + Math.cos(ang) * r1, cy + Math.sin(ang) * r1);
      ctx.lineTo(cx + Math.cos(ang) * r2, cy + Math.sin(ang) * r2);
      ctx.stroke();
    }

    // Threshold markers.
    for (const th of thresholds) {
      const ang = START + SWEEP * norm(th, range);
      const g = scoreRGB(scorer(th)) as RGB;
      ctx.strokeStyle = rgba(g, 0.7 * alive);
      ctx.lineWidth = 2;
      const r1 = r - lw * 1.3;
      const r2 = r - lw * 0.8;
      ctx.beginPath();
      ctx.moveTo(cx + Math.cos(ang) * r1, cy + Math.sin(ang) * r1);
      ctx.lineTo(cx + Math.cos(ang) * r2, cy + Math.sin(ang) * r2);
      ctx.stroke();
    }

    // Track.
    ctx.strokeStyle = 'rgba(160,200,230,0.07)';
    ctx.lineWidth = lw;
    ctx.beginPath();
    ctx.arc(cx, cy, r, START, START + SWEEP);
    ctx.stroke();

    if (!Number.isNaN(a.shown)) {
      const end = START + SWEEP * Math.max(0.002, t);
      // Bloom passes.
      ctx.save();
      ctx.shadowColor = rgba(col, 0.9 * glow);
      ctx.shadowBlur = lw * (2.2 + 2.5 * glow);
      ctx.strokeStyle = rgba(col, 0.55 + 0.35 * alive);
      ctx.lineWidth = lw;
      ctx.beginPath();
      ctx.arc(cx, cy, r, START, end);
      ctx.stroke();
      ctx.shadowBlur = lw * 0.8;
      ctx.stroke();
      ctx.restore();

      // Bright inner core.
      const grad = ctx.createLinearGradient(cx - r, cy + r, cx + r, cy - r);
      grad.addColorStop(0, rgba(col, 0.25 * alive));
      grad.addColorStop(1, rgba(mix(col, [255, 255, 255], 0.55), 0.95 * alive));
      ctx.strokeStyle = grad;
      ctx.lineWidth = lw * 0.38;
      ctx.beginPath();
      ctx.arc(cx, cy, r, START, end);
      ctx.stroke();

      // Head.
      const hx = cx + Math.cos(end) * r;
      const hy = cy + Math.sin(end) * r;
      ctx.save();
      ctx.shadowColor = rgba(col, 1 * alive);
      ctx.shadowBlur = lw * 3 * breath;
      ctx.fillStyle = rgba(mix(col, [255, 255, 255], 0.7), alive);
      ctx.beginPath();
      ctx.arc(hx, hy, lw * 0.55, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
    }

    // Inner hairline.
    ctx.strokeStyle = rgba(col, 0.12 * alive);
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(cx, cy, r - lw * 1.9, 0, Math.PI * 2);
    ctx.stroke();
  });

  const aria = `${label}: ${isNum(value) ? `${value.toFixed(digits)} ${unit}, ${GRADE_LABEL[grade]}` : 'no data'}`;

  return (
    <div ref={wrapRef} className={`ring ring--${size} ${className ?? ''}`} role="meter" aria-label={aria}
      aria-valuemin={range[0]} aria-valuemax={range[1]} aria-valuenow={isNum(value) ? value : undefined}>
      <canvas ref={canvas} className="ring__canvas" aria-hidden="true" />
      <div className="ring__center">
        <span className="ring__label">{label}</span>
        <span className="ring__value num" ref={numRef}>—</span>
        <span className="ring__unit">{unit}</span>
        <span className="ring__grade" ref={gradeRef}>—</span>
        {sub ? <span className="ring__sub">{sub}</span> : null}
      </div>
    </div>
  );
}
