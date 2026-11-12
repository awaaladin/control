// Web Audio helpers: short event chimes and the Aim-mode pitch tone.

let ctx: AudioContext | null = null;

function audio(): AudioContext | null {
  try {
    if (!ctx) {
      const C = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!C) return null;
      ctx = new C();
    }
    if (ctx.state === 'suspended') void ctx.resume();
    return ctx;
  } catch {
    return null;
  }
}

const CHIMES: Record<string, number[]> = {
  handover: [660, 880],
  disconnect: [440, 294],
  error: [392, 262],
  router_online: [523, 784],
  phone_online: [587, 880],
  phone_offline: [587, 392],
  device_join: [784],
  device_leave: [523],
};

export function chime(kind: string): void {
  const notes = CHIMES[kind];
  if (!notes) return;
  const ac = audio();
  if (!ac) return;
  const t0 = ac.currentTime + 0.01;
  notes.forEach((f, i) => {
    const o = ac.createOscillator();
    const g = ac.createGain();
    o.type = 'sine';
    o.frequency.value = f;
    const t = t0 + i * 0.12;
    g.gain.setValueAtTime(0, t);
    g.gain.linearRampToValueAtTime(0.08, t + 0.015);
    g.gain.exponentialRampToValueAtTime(0.0001, t + 0.35);
    o.connect(g).connect(ac.destination);
    o.start(t);
    o.stop(t + 0.4);
  });
}

/** Continuous tone whose pitch follows a value (Aim mode). */
export class PitchTone {
  private osc: OscillatorNode | null = null;
  private gain: GainNode | null = null;

  start(): boolean {
    const ac = audio();
    if (!ac) return false;
    if (this.osc) return true;
    this.osc = ac.createOscillator();
    this.gain = ac.createGain();
    this.osc.type = 'triangle';
    this.osc.frequency.value = 440;
    this.gain.gain.value = 0;
    this.gain.gain.linearRampToValueAtTime(0.06, ac.currentTime + 0.2);
    this.osc.connect(this.gain).connect(ac.destination);
    this.osc.start();
    return true;
  }

  /** t in 0..1 maps to 220..1320 Hz on an exponential scale. */
  set(t: number, stale: boolean): void {
    if (!this.osc || !this.gain || !ctx) return;
    const f = 220 * Math.pow(6, Math.min(1, Math.max(0, t)));
    this.osc.frequency.setTargetAtTime(f, ctx.currentTime, 0.08);
    this.gain.gain.setTargetAtTime(stale ? 0 : 0.06, ctx.currentTime, 0.1);
  }

  stop(): void {
    if (!this.osc || !this.gain || !ctx) return;
    const o = this.osc;
    this.gain.gain.setTargetAtTime(0, ctx.currentTime, 0.05);
    window.setTimeout(() => {
      try {
        o.stop();
      } catch {
        /* already stopped */
      }
    }, 300);
    this.osc = null;
    this.gain = null;
  }
}
