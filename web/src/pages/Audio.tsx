import { useEffect, useRef, useState } from 'react';
import { Badge, Icon, Panel, Switch } from '../components/ui';
import { FONT_MONO } from '../lib/chart';
import { useStore } from '../lib/store';
import { useCanvasSize, useTick } from '../lib/ticker';

type Status = 'idle' | 'asking' | 'running' | 'denied' | 'error';

interface Engine {
  ctx: AudioContext;
  stream: MediaStream;
  analyser: AnalyserNode;
  freq: Float32Array;
  time: Float32Array;
}

const FFT_SIZES = [512, 1024, 2048, 4096, 8192, 16384];
const BAR_COUNTS = [64, 96, 128];

/** Log-spaced bar edges in Hz from 20 Hz to min(20 kHz, Nyquist). */
function edges(n: number, nyquist: number): number[] {
  const lo = 20;
  const hi = Math.min(20000, nyquist);
  const out: number[] = [];
  for (let i = 0; i <= n; i++) out.push(lo * Math.pow(hi / lo, i / n));
  return out;
}

const db = (lin: number) => (lin > 0 ? 20 * Math.log10(lin) : -Infinity);

export function AudioPage() {
  const [status, setStatus] = useState<Status>('idle');
  const [err, setErr] = useState('');
  const [fft, setFft] = useState(4096);
  const [smoothing, setSmoothing] = useState(0.72);
  const [floorDb, setFloorDb] = useState(-90);
  const [bars, setBars] = useState(96);
  const [frozen, setFrozen] = useState(false);
  const [beatSync, setBeatSync] = useState(true);
  const eng = useRef<Engine | null>(null);
  const hue = useStore((s) => s.prefs.accentHue);

  const specRef = useRef<HTMLCanvasElement>(null);
  const scopeRef = useRef<HTMLCanvasElement>(null);
  const fallRef = useRef<HTMLCanvasElement>(null);
  const spec = useCanvasSize(specRef);
  const scope = useCanvasSize(scopeRef);
  const fall = useCanvasSize(fallRef);
  const rmsRef = useRef<HTMLSpanElement>(null);
  const peakRef = useRef<HTMLSpanElement>(null);
  const bpmRef = useRef<HTMLSpanElement>(null);
  const rmsBar = useRef<HTMLDivElement>(null);
  const peakBar = useRef<HTMLDivElement>(null);
  const holdBar = useRef<HTMLDivElement>(null);
  const beatDot = useRef<HTMLSpanElement>(null);

  const st = useRef({
    peaks: new Float32Array(128),
    peakVel: new Float32Array(128),
    levels: new Float32Array(128),
    energyHist: [] as number[],
    lastBeat: 0,
    beat: 0,
    intervals: [] as number[],
    peakHold: -100,
    peakHoldAt: 0,
    fallVer: -1,
  });

  const stop = () => {
    const e = eng.current;
    eng.current = null;
    if (e) {
      e.stream.getTracks().forEach((t) => t.stop());
      void e.ctx.close().catch(() => undefined);
    }
    document.documentElement.style.setProperty('--beat', '0');
    setStatus('idle');
  };

  useEffect(() => () => stop(), []);

  const start = async () => {
    if (!navigator.mediaDevices?.getUserMedia) {
      setErr('This browser does not expose a microphone API here. Microphone access needs HTTPS or localhost.');
      setStatus('error');
      return;
    }
    setStatus('asking');
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false },
      });
      const C = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
      const ctx = new C();
      const src = ctx.createMediaStreamSource(stream);
      const analyser = ctx.createAnalyser();
      analyser.fftSize = fft;
      analyser.smoothingTimeConstant = smoothing;
      analyser.minDecibels = floorDb;
      analyser.maxDecibels = -10;
      src.connect(analyser);
      eng.current = {
        ctx,
        stream,
        analyser,
        freq: new Float32Array(analyser.frequencyBinCount),
        time: new Float32Array(analyser.fftSize),
      };
      setStatus('running');
    } catch (e) {
      const name = e instanceof DOMException ? e.name : '';
      if (name === 'NotAllowedError' || name === 'SecurityError') setStatus('denied');
      else {
        setErr(e instanceof Error ? e.message : String(e));
        setStatus('error');
      }
    }
  };

  // Apply control changes to the live analyser.
  useEffect(() => {
    const e = eng.current;
    if (!e) return;
    e.analyser.fftSize = fft;
    e.analyser.smoothingTimeConstant = smoothing;
    e.analyser.minDecibels = floorDb;
    e.freq = new Float32Array(e.analyser.frequencyBinCount);
    e.time = new Float32Array(e.analyser.fftSize);
  }, [fft, smoothing, floorDb, status]);

  useEffect(() => {
    if (!beatSync) document.documentElement.style.setProperty('--beat', '0');
  }, [beatSync]);

  useTick((now, dt) => {
    const e = eng.current;
    const s = st.current;
    if (!e) return;
    if (!frozen) {
      e.analyser.getFloatFrequencyData(e.freq);
      e.analyser.getFloatTimeDomainData(e.time);
    }
    const sr = e.ctx.sampleRate;
    const nyq = sr / 2;
    const bins = e.freq.length;
    const binHz = nyq / bins;
    const minDb = e.analyser.minDecibels;
    const maxDb = e.analyser.maxDecibels;
    const accent = (l: number, a = 1) => `hsla(${hue} 90% ${l}% / ${a})`;

    // ---- Levels per log-spaced bar.
    const ed = edges(bars, nyq);
    for (let b = 0; b < bars; b++) {
      const i0 = Math.max(0, Math.floor(ed[b] / binHz));
      const i1 = Math.min(bins - 1, Math.max(i0, Math.ceil(ed[b + 1] / binHz)));
      let m = -Infinity;
      for (let i = i0; i <= i1; i++) if (e.freq[i] > m) m = e.freq[i];
      const v = Math.max(0, Math.min(1, (m - minDb) / (maxDb - minDb)));
      s.levels[b] = v;
      if (!frozen) {
        if (v >= s.peaks[b]) {
          s.peaks[b] = v;
          s.peakVel[b] = 0;
        } else {
          s.peakVel[b] += dt * 0.9;
          s.peaks[b] = Math.max(v, s.peaks[b] - s.peakVel[b] * dt);
        }
      }
    }

    // ---- Spectrum.
    {
      const c = specRef.current;
      const ctx = c?.getContext('2d');
      const { w, h, dpr } = spec.size.current;
      if (c && ctx && w) {
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, w, h);
        const padB = 18;
        const H = h - padB;
        const gap = Math.max(1, w / bars / 5);
        const bw = w / bars - gap;
        const grad = ctx.createLinearGradient(0, H, 0, 0);
        grad.addColorStop(0, accent(40, 0.85));
        grad.addColorStop(0.6, accent(62, 1));
        grad.addColorStop(1, accent(85, 1));
        ctx.save();
        ctx.shadowColor = accent(60, 0.9);
        ctx.shadowBlur = 12;
        ctx.fillStyle = grad;
        for (let b = 0; b < bars; b++) {
          const bh = Math.max(1, s.levels[b] * H);
          ctx.fillRect(b * (bw + gap) + gap / 2, H - bh, bw, bh);
        }
        ctx.restore();
        ctx.fillStyle = 'rgba(240,248,255,0.85)';
        for (let b = 0; b < bars; b++) {
          const y = H - s.peaks[b] * H;
          ctx.fillRect(b * (bw + gap) + gap / 2, y - 2, bw, 2);
        }
        ctx.font = `10px ${FONT_MONO}`;
        ctx.fillStyle = 'rgba(170,190,205,0.55)';
        ctx.textBaseline = 'bottom';
        ctx.textAlign = 'center';
        const lo = ed[0];
        const hi = ed[ed.length - 1];
        for (const f of [50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]) {
          if (f < lo || f > hi) continue;
          const x = (Math.log(f / lo) / Math.log(hi / lo)) * w;
          ctx.fillText(f >= 1000 ? `${f / 1000}k` : `${f}`, x, h - 2);
        }
      }
    }

    // ---- Oscilloscope (rising zero-crossing trigger for a steady trace).
    {
      const c = scopeRef.current;
      const ctx = c?.getContext('2d');
      const { w, h, dpr } = scope.size.current;
      if (c && ctx && w) {
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, w, h);
        ctx.strokeStyle = 'rgba(160,200,230,0.08)';
        ctx.beginPath();
        ctx.moveTo(0, h / 2 + 0.5);
        ctx.lineTo(w, h / 2 + 0.5);
        ctx.stroke();
        const t = e.time;
        const span = Math.min(t.length, 2048);
        let trig = 0;
        for (let i = 1; i < t.length - span; i++) {
          if (t[i - 1] < 0 && t[i] >= 0) {
            trig = i;
            break;
          }
        }
        ctx.save();
        ctx.strokeStyle = accent(70, 1);
        ctx.shadowColor = accent(60, 0.9);
        ctx.shadowBlur = 10;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        for (let i = 0; i < span; i++) {
          const x = (i / (span - 1)) * w;
          const y = h / 2 - Math.max(-1, Math.min(1, t[trig + i] * 2.5)) * (h / 2 - 4);
          if (i) ctx.lineTo(x, y);
          else ctx.moveTo(x, y);
        }
        ctx.stroke();
        ctx.restore();
      }
    }

    // ---- Spectrogram waterfall (scrolls left one device pixel per frame).
    if (!frozen) {
      const c = fallRef.current;
      const ctx = c?.getContext('2d');
      const { w, h, dpr } = fall.size.current;
      if (c && ctx && w) {
        const W = c.width;
        const H = c.height;
        ctx.setTransform(1, 0, 0, 1, 0, 0);
        if (fall.version.current !== s.fallVer) {
          s.fallVer = fall.version.current;
          ctx.fillStyle = '#05080c';
          ctx.fillRect(0, 0, W, H);
        }
        const step = Math.max(1, Math.round(dpr));
        ctx.drawImage(c, -step, 0);
        const img = ctx.createImageData(step, H);
        const lo = 20;
        const hi = Math.min(20000, nyq);
        for (let y = 0; y < H; y++) {
          const f = lo * Math.pow(hi / lo, 1 - y / H);
          const v = e.freq[Math.min(bins - 1, Math.round(f / binHz))];
          const n = Math.max(0, Math.min(1, (v - minDb) / (maxDb - minDb)));
          // Dark → accent → white heat map.
          const hRgb = hslToRgb(hue / 360, 0.9, 0.12 + n * 0.55);
          const k = Math.pow(n, 1.6);
          for (let x = 0; x < step; x++) {
            const o = (y * step + x) * 4;
            img.data[o] = hRgb[0] * k + 255 * Math.pow(n, 6) * 0.6;
            img.data[o + 1] = hRgb[1] * k + 255 * Math.pow(n, 6) * 0.6;
            img.data[o + 2] = hRgb[2] * k + 255 * Math.pow(n, 6) * 0.6 + 10;
            img.data[o + 3] = 255;
          }
        }
        ctx.putImageData(img, W - step, 0);
        void h;
      }
    }

    // ---- RMS / peak meter.
    let sum = 0;
    let pk = 0;
    for (let i = 0; i < e.time.length; i++) {
      const v = e.time[i];
      sum += v * v;
      const a = Math.abs(v);
      if (a > pk) pk = a;
    }
    const rmsDb = db(Math.sqrt(sum / e.time.length));
    const pkDb = db(pk);
    if (pkDb > s.peakHold || now - s.peakHoldAt > 1500) {
      s.peakHold = Math.max(-100, pkDb);
      s.peakHoldAt = now;
    }
    const pos = (d: number) => `${Math.max(0, Math.min(1, (d + 60) / 60)) * 100}%`;
    if (rmsBar.current) rmsBar.current.style.width = pos(rmsDb);
    if (peakBar.current) peakBar.current.style.width = pos(pkDb);
    if (holdBar.current) holdBar.current.style.left = pos(s.peakHold);
    if (rmsRef.current) rmsRef.current.textContent = Number.isFinite(rmsDb) ? rmsDb.toFixed(1).replace('-', '−') : '−∞';
    if (peakRef.current) peakRef.current.textContent = Number.isFinite(s.peakHold) ? s.peakHold.toFixed(1).replace('-', '−') : '−∞';

    // ---- Beat detection on 40–150 Hz energy against a ~1 s running mean.
    if (!frozen) {
      let energy = 0;
      const b0 = Math.floor(40 / binHz);
      const b1 = Math.max(b0, Math.ceil(150 / binHz));
      for (let i = b0; i <= b1 && i < bins; i++) energy += Math.pow(10, e.freq[i] / 10);
      energy /= b1 - b0 + 1;
      const hist = s.energyHist;
      const mean = hist.length ? hist.reduce((a, b) => a + b, 0) / hist.length : energy;
      const variance = hist.length ? hist.reduce((a, b) => a + (b - mean) * (b - mean), 0) / hist.length : 0;
      const thresh = mean * 1.35 + Math.sqrt(variance) * 0.5;
      hist.push(energy);
      if (hist.length > 60) hist.shift();
      if (hist.length > 20 && energy > thresh && energy > 1e-7 && now - s.lastBeat > 280) {
        if (s.lastBeat) {
          s.intervals.push(now - s.lastBeat);
          if (s.intervals.length > 12) s.intervals.shift();
        }
        s.lastBeat = now;
        s.beat = 1;
      }
    }
    s.beat *= Math.exp(-dt * 7);
    if (beatSync) document.documentElement.style.setProperty('--beat', s.beat.toFixed(3));
    if (beatDot.current) beatDot.current.style.opacity = String(0.25 + s.beat * 0.75);
    if (bpmRef.current) {
      const iv = s.intervals.filter((x) => x > 280 && x < 1500).sort((a, b) => a - b);
      const med = iv.length >= 4 ? iv[Math.floor(iv.length / 2)] : 0;
      const txt = med && now - s.lastBeat < 3000 ? String(Math.round(60000 / med)) : '—';
      if (bpmRef.current.textContent !== txt) bpmRef.current.textContent = txt;
    }
  });

  const running = status === 'running';

  return (
    <div className="page page--audio">
      <header className="pagehead">
        <div>
          <div className="pagehead__kicker">Browser microphone · nothing leaves this device</div>
          <h1 className="pagehead__title">Audio</h1>
        </div>
        <div className="pagehead__actions">
          <Badge tone="neutral">Phone mic via backend: coming soon</Badge>
          {running ? (
            <button type="button" className="btn" onClick={stop}>
              Stop mic
            </button>
          ) : null}
        </div>
      </header>

      {!running ? (
        <Panel className="mic-ask">
          <div className="mic-ask__inner">
            <span className="mic-ask__icon" aria-hidden="true">
              <Icon name="mic" size={34} />
            </span>
            <div>
              <h2 className="mic-ask__title">
                {status === 'denied' ? 'Microphone access was blocked' : status === 'error' ? 'Could not open the microphone' : 'Listen through this browser'}
              </h2>
              <p className="muted">
                {status === 'denied'
                  ? 'Allow the microphone for this site in the browser’s address bar, then try again.'
                  : status === 'error'
                    ? err
                    : 'PULSE analyses the audio locally (spectrum, scope, waterfall and level meter). Nothing is recorded or sent anywhere. Your browser will ask for permission.'}
              </p>
              <button type="button" className="btn btn--accent" onClick={() => void start()} disabled={status === 'asking'}>
                <Icon name="mic" size={18} /> {status === 'asking' ? 'Waiting for permission…' : status === 'idle' ? 'Enable microphone' : 'Try again'}
              </button>
            </div>
          </div>
        </Panel>
      ) : null}

      <div className={`audio-grid ${running ? '' : 'is-idle'}`} aria-hidden={!running}>
        <Panel
          className="audio-spec"
          kicker={`${bars} log bands · 20 Hz–20 kHz`}
          title="Spectrum"
          actions={
            <span className="beat">
              <span ref={beatDot} className="beat__dot" aria-hidden="true" /> <span ref={bpmRef} className="num">—</span> <small>BPM</small>
            </span>
          }
        >
          <canvas ref={specRef} className="audio-canvas audio-canvas--spec" role="img" aria-label="Live frequency spectrum" />
        </Panel>
        <Panel className="audio-meter" kicker="dBFS" title="Level">
          <div className="meter">
            <div className="meter__row">
              <span className="meter__k">RMS</span>
              <div className="meter__track">
                <div ref={rmsBar} className="meter__fill meter__fill--rms" />
              </div>
              <span ref={rmsRef} className="meter__v num">—</span>
            </div>
            <div className="meter__row">
              <span className="meter__k">Peak</span>
              <div className="meter__track">
                <div ref={peakBar} className="meter__fill meter__fill--peak" />
                <div ref={holdBar} className="meter__hold" />
              </div>
              <span ref={peakRef} className="meter__v num">—</span>
            </div>
            <div className="meter__scale num" aria-hidden="true">
              <span>−60</span>
              <span>−40</span>
              <span>−20</span>
              <span>0</span>
            </div>
          </div>
          <div className="audio-controls">
            <label className="field">
              <span className="field__label">FFT size</span>
              <select className="input" value={fft} onChange={(e) => setFft(Number(e.target.value))}>
                {FFT_SIZES.map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span className="field__label">Bars</span>
              <select className="input" value={bars} onChange={(e) => setBars(Number(e.target.value))}>
                {BAR_COUNTS.map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
            <label className="field field--wide">
              <span className="field__label">
                Smoothing <span className="num">{smoothing.toFixed(2)}</span>
              </span>
              <input type="range" min={0} max={0.95} step={0.01} value={smoothing} onChange={(e) => setSmoothing(Number(e.target.value))} />
            </label>
            <label className="field field--wide">
              <span className="field__label">
                Sensitivity floor <span className="num">{floorDb} dB</span>
              </span>
              <input type="range" min={-120} max={-40} step={1} value={floorDb} onChange={(e) => setFloorDb(Number(e.target.value))} />
            </label>
            <Switch checked={frozen} onChange={setFrozen} label="Freeze" />
            <Switch checked={beatSync} onChange={setBeatSync} label="Beat glow" />
          </div>
        </Panel>
        <Panel className="audio-scope" kicker="Time domain" title="Oscilloscope">
          <canvas ref={scopeRef} className="audio-canvas" role="img" aria-label="Oscilloscope trace" />
        </Panel>
        <Panel className="audio-fall" kicker="Time → , frequency ↑ (log)" title="Spectrogram">
          <canvas ref={fallRef} className="audio-canvas audio-canvas--fall" role="img" aria-label="Spectrogram waterfall" />
        </Panel>
      </div>
    </div>
  );
}

function hslToRgb(h: number, s: number, l: number): [number, number, number] {
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const f = (t: number) => {
    if (t < 0) t += 1;
    if (t > 1) t -= 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  };
  return [f(h + 1 / 3) * 255, f(h) * 255, f(h - 1 / 3) * 255];
}
