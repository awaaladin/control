// The Android app gives its own page a small bridge (PulseApp). It answers only for the engine built into the phone.
// No imports on purpose: other modules use it.

interface Bridge {
  getToken?: () => string;
  engineState?: () => string;
  startEngine?: () => void;
  stopEngine?: () => void;
}
const bridge = (): Bridge | null => (globalThis as unknown as { PulseApp?: Bridge }).PulseApp ?? null;

/** The built-in engine's token, when this page runs inside the app. */
export function appToken(): string {
  try {
    return bridge()?.getToken?.() ?? '';
  } catch {
    return '';
  }
}

export interface AppEngineState {
  available: boolean;
  state: string;
  stopped_by_user: boolean;
  error: string;
}
export function appEngineState(): AppEngineState | null {
  try {
    const raw = bridge()?.engineState?.();
    if (!raw) return null;
    const o = JSON.parse(raw) as Partial<AppEngineState>;
    return o.available ? { available: true, state: String(o.state ?? ''), stopped_by_user: !!o.stopped_by_user, error: String(o.error ?? '') } : null;
  } catch {
    return null;
  }
}
export const appStartEngine = (): void => {
  try {
    bridge()?.startEngine?.();
  } catch {
    /* not in the app */
  }
};
export const appStopEngine = (): void => {
  try {
    bridge()?.stopEngine?.();
  } catch {
    /* not in the app */
  }
};
