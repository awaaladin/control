import { useEffect, useRef, type RefObject } from 'react';

export interface PointerPos {
  x: number;
  y: number;
}

/**
 * Tracks the pointer over an element in CSS pixels without re-rendering.
 * Touch keeps the crosshair for a moment after lifting the finger.
 */
export function usePointer(el: RefObject<HTMLElement>) {
  const pos = useRef<PointerPos | null>(null);
  useEffect(() => {
    const node = el.current;
    if (!node) return;
    let clearTimer = 0;
    const move = (e: PointerEvent) => {
      const r = node.getBoundingClientRect();
      pos.current = { x: e.clientX - r.left, y: e.clientY - r.top };
      window.clearTimeout(clearTimer);
    };
    const leave = (e: PointerEvent) => {
      if (e.pointerType === 'touch') {
        window.clearTimeout(clearTimer);
        clearTimer = window.setTimeout(() => (pos.current = null), 1800);
      } else pos.current = null;
    };
    node.addEventListener('pointermove', move);
    node.addEventListener('pointerdown', move);
    node.addEventListener('pointerleave', leave);
    node.addEventListener('pointerup', leave);
    node.addEventListener('pointercancel', leave);
    return () => {
      window.clearTimeout(clearTimer);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerdown', move);
      node.removeEventListener('pointerleave', leave);
      node.removeEventListener('pointerup', leave);
      node.removeEventListener('pointercancel', leave);
    };
  }, [el]);
  return pos;
}
