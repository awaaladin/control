// One shared requestAnimationFrame loop for every canvas on screen.

import { useEffect, useRef, useState, type RefObject } from 'react';

type Tick = (now: number, dt: number) => void;

const subs = new Set<Tick>();
let raf = 0;
let last = 0;

function frame(now: number) {
  const dt = last ? Math.min(0.1, (now - last) / 1000) : 1 / 60;
  last = now;
  for (const f of subs) {
    try {
      f(now, dt);
    } catch (e) {
      console.error(e);
    }
  }
  raf = subs.size ? requestAnimationFrame(frame) : 0;
  if (!raf) last = 0;
}

export function subscribeTick(f: Tick): () => void {
  subs.add(f);
  if (!raf) raf = requestAnimationFrame(frame);
  return () => {
    subs.delete(f);
  };
}

/** Run `f` every animation frame while mounted. `f` may change between renders. */
export function useTick(f: Tick): void {
  const ref = useRef(f);
  ref.current = f;
  useEffect(() => subscribeTick((n, dt) => ref.current(n, dt)), []);
}

/** Re-render every `ms` (for clocks and "x seconds ago" labels). */
export function useNow(ms = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(id);
  }, [ms]);
  return now;
}

export interface CanvasSize {
  w: number;
  h: number;
  dpr: number;
}

/**
 * Keeps a canvas' backing store matched to its CSS size and device pixel
 * ratio. Returns a ref with the current size (read it inside the tick).
 */
export function useCanvasSize(canvas: RefObject<HTMLCanvasElement>) {
  const size = useRef<CanvasSize>({ w: 0, h: 0, dpr: 1 });
  const version = useRef(0);
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const apply = () => {
      const r = c.getBoundingClientRect();
      const dpr = Math.min(2.5, window.devicePixelRatio || 1);
      const w = Math.max(1, Math.round(r.width));
      const h = Math.max(1, Math.round(r.height));
      if (w !== size.current.w || h !== size.current.h || dpr !== size.current.dpr) {
        size.current = { w, h, dpr };
        c.width = Math.round(w * dpr);
        c.height = Math.round(h * dpr);
        version.current++;
      }
    };
    apply();
    const ro = new ResizeObserver(apply);
    ro.observe(c);
    return () => ro.disconnect();
  }, [canvas]);
  return { size, version };
}
