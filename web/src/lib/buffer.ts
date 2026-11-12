// Rolling sample buffer kept outside React. Charts read it directly from a
// requestAnimationFrame loop, so new samples never cause component re-renders.

import type { Sample, Telemetry } from './types';
import { isNum } from './format';

export const BUFFER_SPAN_MS = 24 * 60 * 60 * 1000;
const CAP = 100_000; // > 24 h at 1 Hz

export type Field = 'rsrp' | 'rsrq' | 'sinr' | 'rx' | 'tx' | 'pci' | 'cell' | 'band';

const nan = (v: number | null | undefined) => (isNum(v) ? v : NaN);

class SampleBuffer {
  ts = new Float64Array(CAP);
  cols: Record<Field, Float64Array> = {
    rsrp: new Float64Array(CAP),
    rsrq: new Float64Array(CAP),
    sinr: new Float64Array(CAP),
    rx: new Float64Array(CAP),
    tx: new Float64Array(CAP),
    pci: new Float64Array(CAP),
    cell: new Float64Array(CAP),
    band: new Float64Array(CAP),
  };
  len = 0;
  /** Bumps on every mutation; charts use it to decide when to redraw. */
  version = 0;
  /** Oldest timestamp for which history has been fetched from the backend. */
  coveredSince = Infinity;

  clear() {
    this.len = 0;
    this.version++;
    this.coveredSince = Infinity;
  }

  private write(i: number, s: Sample) {
    this.ts[i] = s.ts;
    this.cols.rsrp[i] = nan(s.rsrp);
    this.cols.rsrq[i] = nan(s.rsrq);
    this.cols.sinr[i] = nan(s.sinr);
    this.cols.rx[i] = nan(s.rx_bytes_per_s);
    this.cols.tx[i] = nan(s.tx_bytes_per_s);
    this.cols.pci[i] = nan(s.pci);
    this.cols.cell[i] = nan(s.cell_id);
    this.cols.band[i] = nan(s.band);
  }

  private dropOldest(n: number) {
    if (n <= 0) return;
    n = Math.min(n, this.len);
    this.ts.copyWithin(0, n, this.len);
    for (const k of Object.keys(this.cols) as Field[]) this.cols[k].copyWithin(0, n, this.len);
    this.len -= n;
  }

  private trim() {
    if (this.len === 0) return;
    const cutoff = this.ts[this.len - 1] - BUFFER_SPAN_MS;
    const i = this.lowerBound(cutoff);
    if (i > 0) this.dropOldest(i);
  }

  push(s: Sample) {
    if (!isNum(s.ts)) return;
    if (this.len > 0 && s.ts <= this.ts[this.len - 1]) {
      // Out-of-order or duplicate: fold it in through merge.
      if (s.ts === this.ts[this.len - 1]) {
        this.write(this.len - 1, s);
        this.version++;
        return;
      }
      this.merge([s]);
      return;
    }
    if (this.len >= CAP) this.dropOldest(Math.ceil(CAP * 0.05));
    this.write(this.len, s);
    this.len++;
    this.trim();
    this.version++;
  }

  /** Merge a batch (e.g. history) into the buffer, sorted and de-duplicated by ts. */
  merge(batch: Sample[]) {
    const all: Sample[] = [];
    for (let i = 0; i < this.len; i++) all.push(this.get(i));
    for (const s of batch) if (isNum(s.ts)) all.push(s);
    all.sort((a, b) => a.ts - b.ts);
    const out: Sample[] = [];
    for (const s of all) {
      const last = out[out.length - 1];
      if (last && Math.abs(last.ts - s.ts) < 1) out[out.length - 1] = s;
      else out.push(s);
    }
    const start = Math.max(0, out.length - CAP);
    this.len = 0;
    for (let i = start; i < out.length; i++) this.write(this.len++, out[i]);
    this.trim();
    this.version++;
  }

  get(i: number): Sample {
    const n = (v: number) => (Number.isNaN(v) ? null : v);
    return {
      ts: this.ts[i],
      rsrp: n(this.cols.rsrp[i]),
      rsrq: n(this.cols.rsrq[i]),
      sinr: n(this.cols.sinr[i]),
      rx_bytes_per_s: n(this.cols.rx[i]),
      tx_bytes_per_s: n(this.cols.tx[i]),
      pci: n(this.cols.pci[i]),
      cell_id: n(this.cols.cell[i]),
      band: n(this.cols.band[i]),
    };
  }

  /** First index with ts >= t. */
  lowerBound(t: number): number {
    let lo = 0;
    let hi = this.len;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (this.ts[mid] < t) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  /** Index of the sample closest to t, or -1. */
  nearest(t: number): number {
    if (this.len === 0) return -1;
    const i = this.lowerBound(t);
    if (i <= 0) return 0;
    if (i >= this.len) return this.len - 1;
    return t - this.ts[i - 1] <= this.ts[i] - t ? i - 1 : i;
  }

  first(): number | null {
    return this.len ? this.ts[0] : null;
  }
  last(): number | null {
    return this.len ? this.ts[this.len - 1] : null;
  }
}

export const samples = new SampleBuffer();

export function sampleFromTelemetry(ts: number, t: Telemetry): Sample {
  return {
    ts,
    rsrp: t.rsrp,
    rsrq: t.rsrq,
    sinr: t.sinr,
    rx_bytes_per_s: t.rx_bytes_per_s,
    tx_bytes_per_s: t.tx_bytes_per_s,
    pci: t.pci,
    cell_id: t.cell_id,
    band: t.band,
  };
}

export function samplesToCsv(): string {
  const head = 'timestamp_ms,iso_time,rsrp_dbm,rsrq_db,sinr_db,rx_bytes_per_s,tx_bytes_per_s,pci,cell_id,band';
  const rows: string[] = [head];
  const c = (v: number) => (Number.isNaN(v) ? '' : String(v));
  for (let i = 0; i < samples.len; i++) {
    const t = samples.ts[i];
    rows.push(
      [
        t,
        new Date(t).toISOString(),
        c(samples.cols.rsrp[i]),
        c(samples.cols.rsrq[i]),
        c(samples.cols.sinr[i]),
        c(samples.cols.rx[i]),
        c(samples.cols.tx[i]),
        c(samples.cols.pci[i]),
        c(samples.cols.cell[i]),
        c(samples.cols.band[i]),
      ].join(','),
    );
  }
  return rows.join('\n');
}
