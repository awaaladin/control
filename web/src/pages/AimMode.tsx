import { useEffect, useRef, useState } from 'react';
import { GlowRing } from '../components/GlowRing';
import { Icon, Switch } from '../components/ui';
import { bandLabel, enb, fmt, fmtSigned, fmtTime, grade, GRADE_LABEL, isNum, norm, RANGES, THRESHOLDS } from '../lib/format';
import { load, save, type MarkedSpot } from '../lib/prefs';
import { PitchTone } from '../lib/sound';
import { isStale, STALE_MS, toast, useStore } from '../lib/store';
import { useTick } from '../lib/ticker';

const stale = () => isStale(useStore.getState().telemetryAt, STALE_MS.telemetry);

/** Fullscreen antenna-aiming view built around a giant SINR ring. */
export function AimMode() {
  const aim = useStore((s) => s.aim);
  const setAim = useStore((s) => s.setAim);
  const t = useStore((s) => s.telemetry);
  const [tone, setTone] = useState(false);
  const [best, setBest] = useState<number | null>(null);
  const [spots, setSpots] = useState<MarkedSpot[]>(() => load<MarkedSpot[]>('spots', []));
  const [label, setLabel] = useState('');
  const toneRef = useRef<PitchTone | null>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  // Best so far for this aiming session.
  useEffect(() => {
    if (!aim) return;
    if (isNum(t?.sinr) && (best === null || t!.sinr! > best)) setBest(t!.sinr!);
  }, [aim, t, best]);

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
      if (e.key === 'Escape') setAim(false);
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      prev?.focus?.();
    };
  }, [aim, setAim]);

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
    const v = useStore.getState().telemetry?.sinr;
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
    };
    const next = [spot, ...spots].slice(0, 40);
    setSpots(next);
    save('spots', next);
    setLabel('');
    toast('ok', 'Spot marked', `${spot.label}: SINR ${fmt(spot.sinr, 1)} dB, RSRP ${fmt(spot.rsrp)} dBm`);
  };
  const remove = (id: string) => {
    const next = spots.filter((s) => s.id !== id);
    setSpots(next);
    save('spots', next);
  };

  return (
    <div className="aim" ref={rootRef} role="dialog" aria-modal="true" aria-label="Aim mode">
      <div className="aim__top">
        <div>
          <div className="pagehead__kicker">Aim mode · PCI {fmt(t?.pci)} · eNB {fmt(enb(t?.cell_id))} · {bandLabel(t?.band)}</div>
          <h1 className="aim__title">Point for the best SINR</h1>
        </div>
        <div className="aim__tools">
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
          <button ref={closeRef} type="button" className="btn" onClick={() => setAim(false)} aria-label="Close aim mode">
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
              <span className="field__label">Label this position</span>
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
                        <span className="spot__label">{s.label}</span>
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
