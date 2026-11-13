import { TowerMap } from '../components/TowerMap';
import { Badge, Empty, Panel } from '../components/ui';
import { bandLabel, fmt, fmtAgo, fmtDateTime, fmtDuration, grade, isNum } from '../lib/format';
import { useStore } from '../lib/store';
import { useNow } from '../lib/ticker';
import {
  shareThisDeviceLocation,
  useTowers,
  useTowersPolling,
  type SourceStats,
  type TowerCell,
  type TowerHandover,
  type TowersView,
} from '../lib/towers';
import '../styles/extras.css';

const cellName = (enb: number, sector: number) => `eNB ${enb} · ${sector}`;

function Sig({ st }: { st: SourceStats | null }) {
  if (!st) return <span className="muted">—</span>;
  return (
    <span className="num">
      <span className={`g-${grade('sinr', st.avg_sinr)}`}>{fmt(st.avg_sinr, 1)}</span>
      <span className="muted"> / </span>
      <span className={`g-${grade('sinr', st.best_sinr)}`}>{fmt(st.best_sinr)}</span>
      <span className="muted"> dB · </span>
      <span className={`g-${grade('rsrp', st.avg_rsrp)}`}>{fmt(st.avg_rsrp)}</span>
      <span className="muted"> dBm</span>
    </span>
  );
}

function lookupText(c: TowerCell): { text: string; tone: string } {
  switch (c.lookup.status) {
    case 'found':
      return { text: 'located', tone: 'good' };
    case 'pending':
      return { text: 'looking up…', tone: 'neutral' };
    case 'not_found':
      return { text: 'not in OpenCellID', tone: 'fair' };
    case 'error':
      return { text: 'lookup failed', tone: 'poor' };
    case 'incomplete':
      return { text: 'IDs incomplete', tone: 'fair' };
    default:
      return { text: 'no key', tone: 'neutral' };
  }
}

function NoKey() {
  return (
    <Panel kicker="Map" title="Tower locations need an OpenCellID key">
      <p className="small">
        rhcs knows which towers your router and phone use, but not where they stand. OpenCellID is a free, open database of tower
        positions. Nothing is guessed without it.
      </p>
      <ol className="tips">
        <li>
          Sign up for free at{' '}
          <a href="https://opencellid.org/register.php" target="_blank" rel="noreferrer">
            opencellid.org
          </a>{' '}
          and copy your access token.
        </li>
        <li>
          Add it to <code>.env</code> on the laptop: <code>OPENCELLID_KEY=your-token</code>
        </li>
        <li>
          Optional, for distance and direction: <code>HOME_LAT=…</code> and <code>HOME_LON=…</code> (from any map app: long-press your
          house).
        </li>
        <li>Restart rhcsd. Tower positions appear here within a minute.</li>
      </ol>
      <p className="muted small">
        With a key, rhcsd sends the IDs of the towers it has seen (not your location) to OpenCellID, and caches the answers in{' '}
        <code>data/towers.json</code>.
      </p>
    </Panel>
  );
}

function CellTable({ cells, now }: { cells: TowerCell[]; now: number }) {
  const sorted = [...cells].sort((a, b) => b.last_seen - a.last_seen);
  return (
    <div className="table-wrap">
      <table className="table">
        <thead>
          <tr>
            <th scope="col">Cell</th>
            <th scope="col">PCI</th>
            <th scope="col">Band</th>
            <th scope="col">Router: time · SINR avg / best · RSRP</th>
            <th scope="col">Phone: time · SINR avg / best · RSRP</th>
            <th scope="col">Where</th>
            <th scope="col">Last seen</th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((c) => {
            const l = lookupText(c);
            return (
              <tr key={c.key}>
                <th scope="row" className="num">
                  {cellName(c.enb, c.sector)}{' '}
                  {c.serving.map((s) => (
                    <Badge key={s} tone="live">
                      {s} now
                    </Badge>
                  ))}
                </th>
                <td className="num">{fmt(c.pci)}</td>
                <td className="num">{c.bands.length ? c.bands.map((b) => bandLabel(b)).join(', ') : '—'}</td>
                <td>
                  {c.router ? <span className="num">{fmtDuration(c.router.secs)} · </span> : null}
                  <Sig st={c.router} />
                </td>
                <td>
                  {c.phone ? <span className="num">{fmtDuration(c.phone.secs)} · </span> : null}
                  <Sig st={c.phone} />
                </td>
                <td>
                  {isNum(c.distance_m) ? (
                    <span className="num">
                      {fmt(c.distance_m / 1000, 2)} km {c.compass} ({fmt(c.bearing_deg)}°)
                    </span>
                  ) : (
                    <span title={c.lookup.message ?? undefined}>
                      <Badge tone={l.tone}>{l.text}</Badge>
                    </span>
                  )}
                </td>
                <td className="num">{fmtAgo(c.last_seen, now)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function Handovers({ list, cells }: { list: TowerHandover[]; cells: Map<string, TowerCell> }) {
  if (!list.length) return <p className="muted small">No handovers recorded yet.</p>;
  const name = (key: string, cellId: number) => {
    const c = cells.get(key);
    return c ? cellName(c.enb, c.sector) : cellName(Math.floor(cellId / 256), cellId % 256);
  };
  return (
    <ul className="hos towerhos">
      {list.slice(0, 80).map((h) => (
        <li className="ho" key={`${h.ts}-${h.source}-${h.to.key}`}>
          <span className="num muted">{fmtDateTime(h.ts)}</span>{' '}
          <Badge tone={h.source === 'router' ? 'accent' : 'violet'}>{h.source}</Badge>{' '}
          <span className="num">
            {name(h.from.key, h.from.cell_id)} → {name(h.to.key, h.to.cell_id)}
          </span>
          <span className="muted small num">
            {' '}
            · PCI {fmt(h.from.pci)}→{fmt(h.to.pci)} · B{fmt(h.from.band)}→B{fmt(h.to.band)} · stayed {fmtDuration(h.stayed_s)}
          </span>
        </li>
      ))}
    </ul>
  );
}

function MapPanel({ v }: { v: TowersView }) {
  const located = v.cells.filter((c) => c.location);
  const pending = v.cells.filter((c) => c.lookup.status === 'pending').length;
  return (
    <Panel
      kicker="OpenStreetMap"
      title="Where your signal comes from"
      actions={
        <button type="button" className="btn btn--small" onClick={shareThisDeviceLocation}>
          Use this device's location
        </button>
      }
    >
      {v.message ? <p className="small g-fair">{v.message}</p> : null}
      {located.length || v.home ? (
        <TowerMap home={v.home} cells={v.cells} bestKey={v.best?.key ?? null} />
      ) : (
        <Empty title="No tower positions yet">
          {pending
            ? `Looking up ${pending} tower${pending === 1 ? '' : 's'} on OpenCellID…`
            : 'None of the towers seen so far are in the OpenCellID database. They will be retried weekly.'}
        </Empty>
      )}
      <p className="muted small">
        Lines run from home to each tower; solid = serving now, colour = average SINR seen on it. Tower positions are OpenCellID estimates
        and can be off by hundreds of metres.
      </p>
    </Panel>
  );
}

export function TowersPage() {
  useTowersPolling();
  const { view: v, unsupported, at } = useTowers();
  const conn = useStore((s) => s.conn);
  const now = useNow(5000);

  const head = (
    <header className="pagehead">
      <div>
        <div className="pagehead__kicker">Router and phone cells</div>
        <h1 className="pagehead__title">Towers</h1>
      </div>
      <div className="pagehead__actions">{v ? <Badge>{`${v.cells.length} cell${v.cells.length === 1 ? '' : 's'} seen`}</Badge> : null}</div>
    </header>
  );

  if (unsupported) {
    return (
      <div className="page">
        {head}
        <Panel>
          <Empty title="This rhcsd has no tower map">The backend answered but doesn't know /api/towers. Update and restart rhcsd.</Empty>
        </Panel>
      </div>
    );
  }
  if (!v) {
    return (
      <div className="page">
        {head}
        <Panel>
          <Empty title={conn === 'live' || at ? 'Loading…' : 'Not connected'}>
            Tower data comes from the home system and will appear once PULSE is connected.
          </Empty>
        </Panel>
      </div>
    );
  }

  const byKey = new Map(v.cells.map((c) => [c.key, c]));
  return (
    <div className="page page--towers">
      {head}
      {v.key_configured ? <MapPanel v={v} /> : <NoKey />}

      {v.best ? (
        <Panel kicker="Best direction" title="Cleanest tower so far" className="towerbest">
          <p>{v.best.message}</p>
          {!v.key_configured || !v.home ? (
            <p className="muted small">Direction needs both an OpenCellID key and a home location.</p>
          ) : null}
        </Panel>
      ) : null}

      <Panel kicker="Everything seen since rhcsd started keeping records" title="Cells">
        {v.cells.length ? (
          <CellTable cells={v.cells} now={now} />
        ) : (
          <Empty title="No cells recorded yet">They appear as soon as the router or phone reports its serving cell.</Empty>
        )}
        <p className="muted small">
          A tower (eNB) usually has three sectors facing different directions; each sector is its own cell. SINR shown as average / best.
        </p>
      </Panel>

      <Panel kicker="Switching between cells" title="Handovers">
        <Handovers list={v.handovers} cells={byKey} />
      </Panel>
    </div>
  );
}
