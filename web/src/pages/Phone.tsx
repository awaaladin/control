import { GlowRing } from '../components/GlowRing';
import { Badge, Empty, ErrorExplain, KV, Panel } from '../components/ui';
import { explainError } from '../lib/explain';
import { bandLabel, enb, fmt, fmtAgo, fmtSigned, grade, type Grade, isNum, sector, str } from '../lib/format';
import { isStale, STALE_MS, useStore } from '../lib/store';
import { useNow } from '../lib/ticker';
import { MetricRing } from './Signal';

const phoneStale = () => isStale(useStore.getState().phoneAt, STALE_MS.phone);

const batteryGrade = (v: number | null | undefined): Grade =>
  !isNum(v) ? 'none' : v >= 60 ? 'excellent' : v >= 35 ? 'good' : v >= 15 ? 'fair' : 'poor';

function Delta({ a, b, digits = 0, unit }: { a: number | null | undefined; b: number | null | undefined; digits?: number; unit: string }) {
  if (!isNum(a) || !isNum(b)) return <span className="num">—</span>;
  const d = a - b;
  const tone = Math.abs(d) < (digits ? 0.5 : 1) ? 'neutral' : d > 0 ? 'better' : 'worse';
  return (
    <span className={`delta delta--${tone} num`}>
      {fmtSigned(d, digits)} {unit}
    </span>
  );
}

export function PhonePage() {
  const p = useStore((s) => s.phone);
  const t = useStore((s) => s.telemetry);
  const src = useStore((s) => s.sources?.phone);
  const phoneAt = useStore((s) => s.phoneAt);
  const now = useNow(2000);
  const stale = isStale(phoneAt, STALE_MS.phone, now);

  if (!p) {
    return (
      <div className="page">
        <header className="pagehead">
          <div>
            <div className="pagehead__kicker">Android over ADB</div>
            <h1 className="pagehead__title">Phone</h1>
          </div>
        </header>
        <Panel>
          <Empty title="No phone connected">
            {src?.error ? <ErrorExplain e={explainError('phone', src.error)} /> : 'rhcsd has not reported a phone yet. Connect it over USB with debugging enabled, or pair ADB over Wi-Fi.'}
            {src?.last_ok ? <div className="muted small">Last seen {fmtAgo(src.last_ok, now)}</div> : null}
          </Empty>
        </Panel>
      </div>
    );
  }

  const sameTower = isNum(p.cell_id) && isNum(t?.cell_id) ? enb(p.cell_id) === enb(t?.cell_id) : null;
  const sameCell = isNum(p.cell_id) && isNum(t?.cell_id) ? p.cell_id === t?.cell_id : null;
  const clients = p.hotspot_clients ?? [];
  const temp = p.battery_temp_c;

  return (
    <div className="page page--phone">
      <header className="pagehead">
        <div>
          <div className="pagehead__kicker">
            Android {str(p.android)} · {bandLabel(p.band)}
          </div>
          <h1 className="pagehead__title">{p.model || 'Phone'}</h1>
        </div>
        <div className="pagehead__actions">
          {p.transport ? <Badge tone={p.transport === 'usb' ? 'good' : 'accent'}>{p.transport === 'usb' ? 'USB' : 'Wi-Fi'}</Badge> : <Badge>transport —</Badge>}
          {stale ? <Badge tone="stale">stale · {fmtAgo(phoneAt, now)}</Badge> : <Badge tone="live" pulse>streaming</Badge>}
        </div>
      </header>

      <div className="phone-grid">
        <Panel title="Battery" kicker="Power" className="battery">
          <div className="battery__row">
            <GlowRing
              label="Battery"
              value={p.battery_level}
              range={[0, 100]}
              unit="%"
              gradeOf={batteryGrade}
              thresholds={[60, 35, 15]}
              isStale={phoneStale}
              size="md"
              sub={p.charging ? '⚡ charging' : p.charging === false ? 'discharging' : undefined}
            />
            <dl className="kvs">
              <KV k="Level" v={isNum(p.battery_level) ? `${fmt(p.battery_level)} %` : '—'} />
              <KV k="Temperature" v={isNum(temp) ? <span className={temp >= 42 ? 'g-poor' : temp >= 38 ? 'g-fair' : ''}>{fmt(temp, 1)} °C</span> : '—'} />
              <KV k="Charging" v={p.charging === null || p.charging === undefined ? '—' : p.charging ? 'yes' : 'no'} />
              <KV k="USB power" v={p.usb_powered === null || p.usb_powered === undefined ? '—' : p.usb_powered ? 'yes' : 'no'} />
              <KV k="Transport" v={p.transport ? p.transport.toUpperCase() : '—'} />
            </dl>
          </div>
        </Panel>

        <Panel title="Phone LTE" kicker="Its own radio" className="phone-lte">
          <div className="rings rings--sm">
            <MetricRing metric="rsrp" value={p.rsrp} size="sm" isStaleFn={phoneStale} />
            <MetricRing metric="sinr" value={p.sinr} size="sm" isStaleFn={phoneStale} />
            <MetricRing metric="rsrq" value={p.rsrq} size="sm" isStaleFn={phoneStale} />
          </div>
          <dl className="kvs kvs--cols">
            <KV k="eNB" v={fmt(enb(p.cell_id))} />
            <KV k="Sector" v={fmt(sector(p.cell_id))} />
            <KV k="PCI" v={fmt(p.pci)} />
            <KV k="Band" v={bandLabel(p.band)} />
            <KV k="EARFCN" v={fmt(p.earfcn)} />
            <KV k="TAC" v={fmt(p.tac)} />
            <KV k="MCC / MNC" v={p.mcc || p.mnc ? `${str(p.mcc)} / ${str(p.mnc)}` : '—'} />
            <KV k="RSSI" v={isNum(p.rssi) ? `${fmt(p.rssi)} dBm` : '—'} />
          </dl>
        </Panel>

        <Panel title="Phone vs router" kicker="Side by side" className="compare">
          <div className="compare__verdict">
            {sameTower === null ? (
              <Badge>tower unknown</Badge>
            ) : sameTower ? (
              <Badge tone="good">same tower{sameCell ? ' · same cell' : ' · different sector'}</Badge>
            ) : (
              <Badge tone="fair">different towers</Badge>
            )}
          </div>
          <table className="table compare__table">
            <thead>
              <tr>
                <th scope="col" />
                <th scope="col">Phone</th>
                <th scope="col">Router</th>
                <th scope="col">Phone − router</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <th scope="row">eNB</th>
                <td className="num">{fmt(enb(p.cell_id))}</td>
                <td className="num">{fmt(enb(t?.cell_id))}</td>
                <td>{sameTower === null ? '—' : sameTower ? 'same' : 'differs'}</td>
              </tr>
              <tr>
                <th scope="row">PCI</th>
                <td className="num">{fmt(p.pci)}</td>
                <td className="num">{fmt(t?.pci)}</td>
                <td>{isNum(p.pci) && isNum(t?.pci) ? (p.pci === t?.pci ? 'same' : 'differs') : '—'}</td>
              </tr>
              <tr>
                <th scope="row">Band</th>
                <td className="num">{bandLabel(p.band)}</td>
                <td className="num">{bandLabel(t?.band)}</td>
                <td>{isNum(p.band) && isNum(t?.band) ? (p.band === t?.band ? 'same' : 'differs') : '—'}</td>
              </tr>
              <tr>
                <th scope="row">RSRP</th>
                <td className={`num g-${grade('rsrp', p.rsrp)}`}>{fmt(p.rsrp)}</td>
                <td className={`num g-${grade('rsrp', t?.rsrp)}`}>{fmt(t?.rsrp)}</td>
                <td>
                  <Delta a={p.rsrp} b={t?.rsrp} unit="dB" />
                </td>
              </tr>
              <tr>
                <th scope="row">SINR</th>
                <td className={`num g-${grade('sinr', p.sinr)}`}>{fmt(p.sinr, 1)}</td>
                <td className={`num g-${grade('sinr', t?.sinr)}`}>{fmt(t?.sinr, 1)}</td>
                <td>
                  <Delta a={p.sinr} b={t?.sinr} digits={1} unit="dB" />
                </td>
              </tr>
              <tr>
                <th scope="row">RSRQ</th>
                <td className={`num g-${grade('rsrq', p.rsrq)}`}>{fmt(p.rsrq, 1)}</td>
                <td className={`num g-${grade('rsrq', t?.rsrq)}`}>{fmt(t?.rsrq, 1)}</td>
                <td>
                  <Delta a={p.rsrq} b={t?.rsrq} digits={1} unit="dB" />
                </td>
              </tr>
            </tbody>
          </table>
        </Panel>

        <Panel title="Hotspot clients" kicker={`${clients.length} connected`} className="clients">
          {clients.length === 0 ? (
            <p className="muted small">No hotspot clients reported.</p>
          ) : (
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th scope="col">Name</th>
                    <th scope="col">Vendor</th>
                    <th scope="col">IP</th>
                    <th scope="col">MAC</th>
                    <th scope="col">Iface</th>
                    <th scope="col">State</th>
                  </tr>
                </thead>
                <tbody>
                  {clients.map((c, i) => (
                    <tr key={`${c.mac ?? ''}-${c.ip ?? i}`}>
                      <td>{c.label || <span className="muted">unlabelled</span>}</td>
                      <td>{str(c.vendor)}</td>
                      <td className="num">{str(c.ip)}</td>
                      <td className="num">{str(c.mac)}</td>
                      <td className="num">{str(c.iface)}</td>
                      <td>
                        <Badge tone={/reach/i.test(c.state ?? '') ? 'good' : 'neutral'}>{str(c.state)}</Badge>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Panel>
      </div>
    </div>
  );
}
