import { useEffect, useRef, useState } from 'react';
import { GlowRing } from '../components/GlowRing';
import { Icon, Switch } from '../components/ui';
import { bandLabel, enb, fmt, fmtSigned, fmtTime, grade, GRADE_LABEL, isNum, norm, RANGES, THRESHOLDS } from '../lib/format';
import { load, save, type MarkedSpot } from '../lib/prefs';
import { PitchTone } from '../lib/sound';
import { isStale, STALE_MS, toast, useStore } from '../lib/store';
import { useTick } from '../lib/ticker';
import { refreshPlaces, usePlaces } from '../lib/towers';

/** Aim mode follows the router's reading if there is one, otherwise the phone's own radio. */
const stale = () => {
  const st = useStore.getState();
  return st.telemetry ? isStale(st.telemetryAt, STALE_MS.telemetry) : isStale(st.phoneAt, STALE_MS.phone);
};

type Sample = { ts: number; v: number };
const spread = (xs: Sample[]) => (xs.length ? Math.max(...xs.map((x) => x.v)) - Math.min(...xs.map((x) => x.v)) : 0);
const mean = (xs: Sample[]) => xs.reduce((a, x) => a + x.v, 0) / (xs.length || 1);

const MOVE_DB = 4; // a swing this big within MOVE_MS means the router/phone is being moved
const MOVE_MS = 8_000;
const SETTLE_DB = 1.2; // steady within this for SETTLE_MS = a position worth recording
const SETTLE_MS = 6_000;
const IDLE_MS = 90_000; // no movement for this long: an automatic session finishes by itself
const COOLDOWN_MS = 5 * 60_000; // after you close it yourself, it stays out of the way for a while

/** Fullscreen antenna-aiming view built around a giant SINR ring. */
export function AimMode() {
  const aim = useStore((s) => s.aim);
  const setAim = useStore((s) => s.setAim);
  const t = useStore((s) => s.telemetry ?? s.phone);
  const [tone, setTone] = useState(false);
  const [auto, setAuto] = useState<boolean>(() => load<boolean>('autoAim', true));
  const win = useRef<Sample[]>([]);
  const autoOpened = useRef(false);
  const cooldownUntil = useRef(0);
  const lastMove = useRef(0);
  const lastAuto = useRef<{ ts: number; sinr: number } | null>(null);
  const [best, setBest] = useState<number | null>(null);
  const [spots, setSpots] = useState<MarkedSpot[]>(() => load<MarkedSpot[]>('spots', []));
  const [label, setLabel] = useState('');
  const spotsRef = useRef<MarkedSpot[]>(spots);
  spotsRef.current = spots;
  const toneRef = useRef<PitchTone | null>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  // Best so far for this aiming session.
  useEffect(() => {
    if (!aim) return;
    if (isNum(t?.sinr) && (best === null || t!.sinr! > best)) setBest(t!.sinr!);
  }, [aim, t, best]);

  // Know which automatic place the phone is at, so a marked spot can name it.
  useEffect(() => {
    if (aim) void refreshPlaces();
  }, [aim]);

  useEffect(() => {
    if (!aim) {
      toneRef.current?.stop();
      toneRef.current = null;
      setTone(false);
      setBest(null);
      if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined);
      return;
    }
    const prev = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') closeByHand();
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      prev?.focus?.();
    };
  }, [aim, setAim]);

  const closeByHand = () => {
    cooldownUntil.current = Date.now() + COOLDOWN_MS;
    autoOpened.current = false;
    setAim(false);
  };

  const saveSpots = (next: MarkedSpot[]) => {
    setSpots(next);
    save('spots', next);
  };

  const bestOf = (list: MarkedSpot[]) =>
    list.reduce<MarkedSpot | null>((b, x) => (isNum(x.sinr) && (!b || !isNum(b.sinr) || x.sinr > b.sinr) ? x : b), null);

  // ---- automatic aiming: start when the signal is being moved, record steady positions, finish when it stops.
  useEffect(() => {
    const v = t?.sinr;
    if (!isNum(v)) return;
    const now = Date.now();
    const w = win.current;
    w.push({ ts: now, v });
    while (w.length && now - w[0].ts > IDLE_MS + 5_000) w.shift();
    if (!auto) return;
    const within = (ms: number) => w.filter((x) => now - x.ts <= ms);

    if (!aim) {
      const recent = within(MOVE_MS);
      if (recent.length >= 4 && spread(recent) >= MOVE_DB && now > cooldownUntil.current) {
        autoOpened.current = true;
        lastMove.current = now;
        setAim(true);
        toast('info', 'Aim mode started by itself', 'The signal is changing. Move slowly: PULSE records the steady spots and tells you the best one.', 7000);
      }
      return;
    }

    if (spread(within(MOVE_MS)) >= MOVE_DB / 2) lastMove.current = now;

    // A steady reading = a position. Record it once, unless it is the same place again.
    const settled = within(SETTLE_MS);
    if (settled.length >= 5 && now - settled[0].ts >= SETTLE_MS - 1_000 && spread(settled) <= SETTLE_DB) {
      const avg = mean(settled);
      const la = lastAuto.current;
      if (!la || Math.abs(avg - la.sinr) >= 1.5 || now - la.ts > 90_000) {
        lastAuto.current = { ts: now, sinr: avg };
        const list = spotsRef.current;
        const spot: MarkedSpot = {
          id: `${now}`,
          label: `Spot ${list.length + 1}`,
          ts: now,
          sinr: Math.round(avg * 10) / 10,
          rsrp: t?.rsrp ?? null,
          rsrq: t?.rsrq ?? null,
          pci: t?.pci ?? null,
          cell_id: t?.cell_id ?? null,
          band: t?.band ?? null,
          place_id: usePlaces.getState().view?.current_place ?? null,
          auto: true,
        };
        saveSpots([spot, ...list].slice(0, 40));
        toast('ok', `Recorded ${spot.label}`, `SINR ${fmt(spot.sinr, 1)} dB, RSRP ${fmt(spot.rsrp)} dBm`, 3500);
      }
    }

    // An automatic session ends by itself once nothing has moved for a while.
    if (autoOpened.current && now - lastMove.current > IDLE_MS) {
      autoOpened.current = false;
      const best = bestOf(spotsRef.current);
      setAim(false);
      const advice =
        best && isNum(best.sinr)
          ? v < best.sinr - 2
            ? `You are ${fmt(best.sinr - v, 1)} dB below it: move back there.`
            : 'You are at (or close to) the best position.'
          : '';
      toast(
        'ok',
        'Aim finished',
        best ? `Best position: ${best.label} (SINR ${fmt(best.sinr, 1)} dB). ${advice}` : 'No steady position was found. Try moving more slowly.',
        10000,
      );
    }
  }, [t, aim, auto]);

  useEffect(() => {
    if (tone) {
      const p = new PitchTone();
      if (!p.start()) {
        toast('warn', 'Audio unavailable', 'This browser blocked Web Audio.');
        setTone(false);
        return;
      }
      toneRef.current = p;
      return () => {
        p.stop();
        toneRef.current = null;
      };
    }
  }, [tone]);

  useTick(() => {
    const p = toneRef.current;
    if (!p) return;
    const st = useStore.getState();
    const v = (st.telemetry ?? st.phone)?.sinr;
    p.set(isNum(v) ? norm(v, RANGES.sinr) : 0, stale() || !isNum(v));
  });

  if (!aim) return null;

  const sinr = t?.sinr;
  const g = grade('sinr', sinr);
  const bestSpot = spots.reduce<MarkedSpot | null>((b, s) => (isNum(s.sinr) && (!b || !isNum(b.sinr) || s.sinr > b.sinr) ? s : b), null);

  const mark = () => {
    const spot: MarkedSpot = {
      id: `${Date.now()}`,
      label: label.trim() || `Spot ${spots.length + 1}`,
      ts: Date.now(),
      sinr: t?.sinr ?? null,
      rsrp: t?.rsrp ?? null,
      rsrq: t?.rsrq ?? null,
      pci: t?.pci ?? null,
      cell_id: t?.cell_id ?? null,
      band: t?.band ?? null,
      place_id: usePlaces.getState().view?.current_place ?? null,
    };
    saveSpots([spot, ...spots].slice(0, 40));
    setLabel('');
    toast('ok', 'Spot marked', `${spot.label}: SINR ${fmt(spot.sinr, 1)} dB, RSRP ${fmt(spot.rsrp)} dBm`);
  };
  const remove = (id: string) => {
    saveSpots(spots.filter((s) => s.id !== id));
  };

  // Live coaching: is it getting better or worse, and how far is the best spot?
  const nowMs = Date.now();
  const recentW = win.current.filter((x) => nowMs - x.ts <= 3_000);
  const beforeW = win.current.filter((x) => nowMs - x.ts > 3_000 && nowMs - x.ts <= 6_000);
  const delta = recentW.length && beforeW.length ? mean(recentW) - mean(beforeW) : 0;
  const trend =
    delta > 0.8
      ? { arrow: '▲', text: 'Getting better', cls: 'is-up' }
      : delta < -0.8
        ? { arrow: '▼', text: 'Getting worse', cls: 'is-down' }
        : { arrow: '●', text: 'Holding steady', cls: 'is-flat' };
  const bestNow = bestOf(spots);
  const coach =
    isNum(sinr) && bestNow && isNum(bestNow.sinr)
      ? sinr >= bestNow.sinr - 0.5
        ? `You are at your best position so far (${bestNow.label}).`
        : `Best so far: ${bestNow.label} at ${fmt(bestNow.sinr, 1)} dB: ${fmt(bestNow.sinr - sinr, 1)} dB better than here.`
      : 'Move slowly and pause for a few seconds at each position. PULSE records the steady ones for you.';

  return (
    <div className="aim" ref={rootRef} role="dialog" aria-modal="true" aria-label="Aim mode">
      <div className="aim__top">
        <div>
          <div className="pagehead__kicker">Aim mode · PCI {fmt(t?.pci)} · eNB {fmt(enb(t?.cell_id))} · {bandLabel(t?.band)}</div>
          <h1 className="aim__title">Point for the best SINR</h1>
        </div>
        <div className="aim__tools">
          <Switch
            checked={auto}
            onChange={(v) => {
              setAuto(v);
              save('autoAim', v);
              if (!v) autoOpened.current = false;
              toast('info', v ? 'Automatic aiming on' : 'Automatic aiming off', v ? 'It starts by itself when the signal changes and records steady spots.' : 'Aim mode will only open when you open it.');
            }}
            label="Auto"
          />
          <Switch checked={tone} onChange={setTone} label="Tone" />
          <button
            type="button"
            className="btn btn--ghost"
            onClick={() => {
              if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined);
              else void rootRef.current?.requestFullscreen?.().catch(() => undefined);
            }}
          >
            Fullscreen
          </button>
          <button ref={closeRef} type="button" className="btn" onClick={closeByHand} aria-label="Close aim mode">
            <Icon name="close" size={18} /> Close
          </button>
        </div>
      </div>

      <div className="aim__body">
        <div className="aim__ring">
          <GlowRing
            label="SINR"
            value={sinr}
            range={RANGES.sinr}
            unit="dB"
            digits={1}
            gradeOf={(v) => grade('sinr', v)}
            thresholds={THRESHOLDS.sinr}
            isStale={stale}
            size="xl"
          />
        </div>
        <div className="aim__side">
          <div className={`aim__coach ${trend.cls}`} role="status" aria-live="polite">
            <span className="aim__trend">
              {trend.arrow} {trend.text}
            </span>
            <span className="aim__advice">{coach}</span>
          </div>
          <div className="aim__stats">
            <div className="aim__stat">
              <span className="aim__k">Grade</span>
              <span className={`aim__v g-${g}`}>{GRADE_LABEL[g]}</span>
            </div>
            <div className="aim__stat">
              <span className="aim__k">Best so far</span>
              <span className="aim__v num">{fmt(best, 1)} <small>dB</small></span>
              <button type="button" className="linkbtn" onClick={() => setBest(null)}>
                reset
              </button>
            </div>
            <div className="aim__stat">
              <span className="aim__k">vs best</span>
              <span className="aim__v num">{isNum(sinr) && isNum(best) ? fmtSigned(sinr - best, 1) : '—'}</span>
            </div>
            <div className="aim__stat">
              <span className="aim__k">RSRP</span>
              <span className={`aim__v num g-${grade('rsrp', t?.rsrp)}`}>{fmt(t?.rsrp)} <small>dBm</small></span>
            </div>
            <div className="aim__stat">
              <span className="aim__k">RSRQ</span>
              <span className={`aim__v num g-${grade('rsrq', t?.rsrq)}`}>{fmt(t?.rsrq, 1)} <small>dB</small></span>
            </div>
          </div>

          <form
            className="aim__mark"
            onSubmit={(e) => {
              e.preventDefault();
              mark();
            }}
          >
            <label className="field">
              <span className="field__label">Name this position (optional: spots are recorded automatically)</span>
              <input className="input" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="e.g. window, north wall, 2 m high" maxLength={60} />
            </label>
            <button type="submit" className="btn btn--accent" disabled={!isNum(sinr)}>
              <Icon name="pin" size={18} /> Mark spot
            </button>
          </form>

          <div className="spots">
            {spots.length === 0 ? (
              <p className="muted small">Marked spots are saved in this browser so you can compare positions.</p>
            ) : (
              <ol className="spots__list">
                {[...spots]
                  .sort((a, b) => (b.sinr ?? -999) - (a.sinr ?? -999))
                  .map((s) => (
                    <li key={s.id} className={`spot ${bestSpot?.id === s.id ? 'is-best' : ''}`}>
                      <div className="spot__main">
                        <span className="spot__label">
                          {s.label}
                          {s.auto ? <span className="spot__auto"> auto</span> : null}
                        </span>
                        <span className="spot__meta muted small num">
                          {fmtTime(s.ts, false)} · PCI {fmt(s.pci)} · {bandLabel(s.band)}
                        </span>
                      </div>
                      <span className={`spot__sinr num g-${grade('sinr', s.sinr)}`}>{fmt(s.sinr, 1)}</span>
                      <span className="spot__delta num muted small">
                        {isNum(sinr) && isNum(s.sinr) ? `now ${fmtSigned(sinr - s.sinr, 1)}` : ''}
                      </span>
                      <button type="button" className="iconbtn" aria-label={`Delete ${s.label}`} onClick={() => remove(s.id)}>
                        <Icon name="trash" size={16} />
                      </button>
                    </li>
                  ))}
              </ol>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
