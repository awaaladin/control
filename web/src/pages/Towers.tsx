import { TowerMap } from '../components/TowerMap';
import { Badge, Empty, Panel } from '../components/ui';
import { bandLabel, fmt, fmtAgo, fmtDateTime, fmtDuration, grade, isNum } from '../lib/format';
import { load, type MarkedSpot } from '../lib/prefs';
import { useStore } from '../lib/store';
import { useNow } from '../lib/ticker';
import {
  renamePlace,
  shareThisDeviceLocation,
  usePlaces,
  usePlacesPolling,
  useTowers,
  useTowersPolling,
  type AroundCell,
  type Place,
  type SourceStats,
  type TowerCell,
  type TowerHandover,
  type TowersView,
} from '../lib/towers';
import '../styles/extras.css';

const cellName = (enb: number, sector: number) => `eNB ${enb} · ${sector}`;

const COMPASS_WORDS: Record<string, string> = {
  N: 'north', NNE: 'north-north-east', NE: 'north-east', ENE: 'east-north-east',
  E: 'east', ESE: 'east-south-east', SE: 'south-east', SSE: 'south-south-east',
  S: 'south', SSW: 'south-south-west', SW: 'south-west', WSW: 'west-south-west',
  W: 'west', WNW: 'west-north-west', NW: 'north-west', NNW: 'north-north-west',
};

function distanceWords(m: number): string {
  return m < 1000 ? `about ${Math.round(m / 10) * 10} m` : `about ${fmt(m / 1000, 1)} km`;
}

/** Arrow pointing toward the tower (0° = north, up). */
function Arrow({ deg }: { deg: number }) {
  return (
    <svg className="where__arrow" viewBox="0 0 64 64" aria-hidden="true" style={{ transform: `rotate(${deg}deg)` }}>
      <circle cx="32" cy="32" r="29" fill="none" stroke="currentColor" strokeOpacity=".18" strokeWidth="2" />
      <path d="M32 9 L42 34 L32 29 L22 34 Z" fill="var(--accent)" />
      <text x="32" y="58" textAnchor="middle" fontSize="8" fill="currentColor" opacity=".5" transform={`rotate(${-deg} 32 32)`}>
        N↑
      </text>
    </svg>
  );
}

/** A place marked in Aim mode takes that spot's name, unless it was renamed. */
function placeName(p: Place, spots: MarkedSpot[]): string {
  if (p.name) return p.name;
  const spot = spots.find((s) => s.place_id === p.id);
  return spot ? spot.label : p.label;
}

/** Places found automatically from the phone's position + signal, best first. */
function PlacesList({ spots }: { spots: MarkedSpot[] }) {
  usePlacesPolling();
  const pv = usePlaces((s) => s.view);
  const unsupported = usePlaces((s) => s.unsupported);
  if (unsupported) return <p className="muted small">This rhcsd doesn't find places automatically yet. Update and restart it.</p>;
  if (!pv) return <p className="muted small">Loading places…</p>;
  if (!pv.places.length) {
    return (
      <p className="muted small">
        No places yet. They're found automatically from the phone's position every 30 s, with the router's and the phone's signal at that
        moment.{' '}
        {pv.position_error
          ? `Right now: ${pv.position_error}.`
          : pv.position
            ? 'Waiting for a precise enough position (better than 50 m).'
            : ''}
      </p>
    );
  }
  return (
    <>
      <ol className="places">
        {pv.places.slice(0, 6).map((p, i) => {
          const name = placeName(p, spots);
          const sig = p.router ?? p.phone;
          const who = p.router ? 'router' : 'phone';
          return (
            <li key={p.id} className={i === 0 && p.quality_sinr !== null ? 'is-best' : ''}>
              <span className="places__name">
                {name}
                {p.is_current ? <Badge tone="live">phone is here</Badge> : null}
                <button
                  type="button"
                  className="linkbtn small"
                  onClick={() => {
                    const n = window.prompt(`Name for "${name}" (leave empty to clear):`, p.name ?? '');
                    if (n !== null) void renamePlace(p.id, n);
                  }}
                >
                  rename
                </button>
              </span>
              <span className="num">
                {sig && isNum(sig.avg_sinr) ? (
                  <>
                    <span className={`g-${grade('sinr', sig.avg_sinr)}`}>{fmt(sig.avg_sinr, 1)} dB</span>
                    <span className="muted small"> {who}</span>
                  </>
                ) : (
                  <span className="muted small">no signal reading</span>
                )}
              </span>
              <span className="places__where muted small">
                {p.where} · {p.samples} reading{p.samples === 1 ? '' : 's'}
                {p.router && p.phone && isNum(p.phone.avg_sinr) ? ` · phone ${fmt(p.phone.avg_sinr, 1)} dB` : ''}
                {sig && isNum(sig.avg_rsrp) ? ` · RSRP ${fmt(sig.avg_rsrp)} dBm` : ''}
              </span>
            </li>
          );
        })}
      </ol>
      {pv.position && !pv.position.fresh ? (
        <p className="muted small">
          The phone's last position is {Math.round(pv.position.age_s / 60)} min old, so no new readings are added right now.
        </p>
      ) : null}
    </>
  );
}

/**
 * Side by side, in plain words (no coordinates): where you got the best
 * signal (automatic places, Aim-mode spots, right now) and where the tower is.
 */
function WhereBest({ v }: { v: TowersView }) {
  const routerOn = useStore((s) => s.sources?.router_configured !== false);
  const ph = useStore((s) => s.phone);
  const tel = useStore((s) => s.telemetry);
  // With no router (the app) "right now" is the phone's own reading.
  const t = routerOn ? tel : ph;
  const allSpots = load<MarkedSpot[]>('spots', []);
  const spots = allSpots.filter((s) => isNum(s.sinr)).sort((a, b) => (b.sinr ?? -99) - (a.sinr ?? -99));
  const best = spots[0];
  const here = usePlaces((s) => s.view?.places.find((p) => p.is_current));
  const byKey = new Map(v.cells.map((c) => [c.key, c]));
  const routerCell = v.serving.router ? byKey.get(v.serving.router) : undefined;
  const phoneCell = v.serving.phone ? byKey.get(v.serving.phone) : undefined;
  const towerCell = routerCell ?? phoneCell;
  const located = towerCell && isNum(towerCell.distance_m) && isNum(towerCell.bearing_deg);
  const dir = towerCell?.compass ? COMPASS_WORDS[towerCell.compass] ?? towerCell.compass : null;
  const lookupOff = v.lookup ? !v.lookup.enabled : false;
  let why = '';
  if (towerCell) {
    const l = towerCell.lookup;
    if (lookupOff) why = 'Tower lookups are off (TOWER_LOOKUP=off), so its position is unknown.';
    else if (towerCell.location && !v.home)
      why = "The tower's position is known; for direction and distance rhcs also needs yours: connect the phone (its location is read automatically) or press \"Use this device's location\" below.";
    else if (l.status === 'pending') why = 'Looking up where this tower stands…';
    else if (l.status === 'not_found') why = `No public database knows where this tower stands yet (${l.message ?? 'not found'}); it is asked again weekly.`;
    else if (l.status === 'error') why = `The tower lookup failed (${l.message ?? 'error'}); retried in 10 minutes.`;
    else if (l.status === 'incomplete') why = "The tower's area code is missing, so it can't be looked up.";
  }

  return (
    <section className="where" aria-label="Where you get the best network">
      <Panel kicker="In your home" title="Your spots">
        <h3 className="where__sub">Best places (automatic)</h3>
        <PlacesList spots={allSpots} />

        <h3 className="where__sub">Marked spots</h3>
        {spots.length ? (
          <>
            <p className="where__lead">
              Best so far: <strong>{best.label}</strong> with SINR{' '}
              <span className={`num g-${grade('sinr', best.sinr)}`}>{fmt(best.sinr, 1)} dB</span> ({grade('sinr', best.sinr)})
            </p>
            <ol className="where__spots">
              {spots.slice(0, 8).map((s, i) => (
                <li key={s.id} className={i === 0 ? 'is-best' : ''}>
                  <span className="where__spot">{s.label}</span>
                  <span className={`num g-${grade('sinr', s.sinr)}`}>{fmt(s.sinr, 1)} dB</span>
                  <span className="muted small num">{fmt(s.rsrp)} dBm</span>
                </li>
              ))}
            </ol>
          </>
        ) : (
          <p className="muted small">
            No spots marked yet. Open <strong>Signal → Aim mode</strong>, carry the router to different places (window, shelf, upstairs…)
            and press <strong>Mark spot</strong> at each one. They'll be ranked here.
          </p>
        )}
        <p className="where__now">
          Right now:{' '}
          {isNum(t?.sinr) ? (
            <>
              SINR <span className={`num g-${grade('sinr', t?.sinr)}`}>{fmt(t?.sinr, 1)} dB</span> ({grade('sinr', t?.sinr)})
              {best && isNum(best.sinr) && isNum(t?.sinr) ? (
                <span className="muted">
                  {' '}
                  · {t!.sinr! >= best.sinr! ? 'as good as your best spot' : `${fmt(best.sinr! - t!.sinr!, 1)} dB below "${best.label}"`}
                </span>
              ) : null}
            </>
          ) : (
            <span className="muted">{routerOn ? 'no router reading' : 'no phone reading'}</span>
          )}
          {here ? (
            <span className="muted">
              {' '}
              · the phone is at {placeName(here, allSpots)} ({here.where})
            </span>
          ) : null}
        </p>
      </Panel>

      <Panel kicker="Where the signal comes from" title="Your tower">
        {towerCell ? (
          <div className="where__tower">
            {located ? <Arrow deg={towerCell.bearing_deg!} /> : null}
            <div>
              <p className="where__lead">
                {located ? (
                  <>
                    The tower is <strong>{distanceWords(towerCell.distance_m!)}</strong> away, to the <strong>{dir}</strong>
                    {v.home_manual === false ? ' of where the phone is now' : ''}.
                  </>
                ) : (
                  <>
                    You're on tower <strong>{towerCell.enb}</strong>, antenna (sector) <strong>{towerCell.sector}</strong>.
                  </>
                )}
              </p>
              <p className="muted small">
                {routerCell ? `Router: ${bandLabel(routerCell.bands[0])}` : ''}
                {routerCell && phoneCell ? ' · ' : ''}
                {phoneCell ? `Phone: ${bandLabel(phoneCell.bands[0])}` : ''}
                {routerCell && phoneCell && routerCell.enb === phoneCell.enb ? ' · both on the same tower' : ''}
              </p>
              <p>
                {located ? (
                  <>
                    {routerOn ? (
                      <>
                        Tip: put the router on the <strong>{dir}</strong> side of the house, high up and near a window facing
                        that way. Walls and metal between it and the tower cost the most signal.
                      </>
                    ) : (
                      <>
                        Tip: the signal is usually best on the <strong>{dir}</strong> side of the building, high up and near a
                        window facing that way. Walls and metal between you and the tower cost the most signal.
                      </>
                    )}
                  </>
                ) : (
                  <span className="muted">{why} Until then, your places and marked spots on the left are the best guide.</span>
                )}
              </p>
            </div>
          </div>
        ) : (
          <p className="muted">No tower reported yet. It appears as soon as the router or phone reports its cell.</p>
        )}
      </Panel>
    </section>
  );
}

/** 0-5 bars from RSRP (dBm). */
function rsrpBars(r: number | null | undefined): number {
  if (!isNum(r)) return 0;
  return r >= -80 ? 5 : r >= -90 ? 4 : r >= -100 ? 3 : r >= -110 ? 2 : r >= -120 ? 1 : 0;
}

function SignalBars({ rsrp }: { rsrp: number | null | undefined }) {
  const n = rsrpBars(rsrp);
  return (
    <span className="bars" role="img" aria-label={isNum(rsrp) ? `${n} of 5 bars` : 'no signal reading'}>
      {[1, 2, 3, 4, 5].map((i) => (
        <span key={i} className={`bars__b ${i <= n ? 'is-on' : ''}`} style={{ height: `${30 + i * 14}%` }} />
      ))}
    </span>
  );
}

function aroundName(c: AroundCell): string {
  if (isNum(c.enb) && isNum(c.sector)) return cellName(c.enb, c.sector);
  return isNum(c.pci) ? `PCI ${c.pci}` : 'cell';
}

/** What the phone hears right now: its own cell and the others around it, strongest first. */
function AroundPanel({ v }: { v: TowersView }) {
  const around = v.around;
  const rec = v.recommendation;
  if (!around) return null;
  return (
    <Panel kicker="Heard by this phone, live" title="Cells around you" className="around">
      {rec ? (
        <p className={`small ${rec.status === 'better' ? 'g-good' : 'muted'}`} role="status">
          {rec.message}
        </p>
      ) : null}
      <div className="table-wrap">
        <table className="table">
          <thead>
            <tr>
              <th scope="col">Cell</th>
              <th scope="col">Signal</th>
              <th scope="col">RSRP</th>
              <th scope="col">SINR</th>
              <th scope="col">Band</th>
              <th scope="col">Where the tower is</th>
            </tr>
          </thead>
          <tbody>
            {around.cells.map((c, i) => (
              <tr key={`${c.key ?? 'x'}-${c.pci ?? i}-${i}`} className={c.serving ? 'is-best' : ''}>
                <th scope="row" className="num">
                  {aroundName(c)} {c.serving ? <Badge tone="live">you are here</Badge> : null}
                  {c.rat && c.rat !== 'LTE' ? <span className="muted small"> {c.rat}</span> : null}
                  {isNum(c.pci) && isNum(c.enb) ? <span className="muted small"> · PCI {c.pci}</span> : null}
                </th>
                <td>
                  <SignalBars rsrp={c.rsrp} />
                </td>
                <td className={`num g-${grade('rsrp', c.rsrp)}`}>{isNum(c.rsrp) ? `${fmt(c.rsrp)} dBm` : '—'}</td>
                <td className={`num g-${grade('sinr', c.sinr)}`}>{isNum(c.sinr) ? `${fmt(c.sinr, 1)} dB` : '—'}</td>
                <td className="num">
                  {isNum(c.band) ? bandLabel(c.band) : '—'}
                  {!isNum(c.band) && isNum(c.earfcn) ? `EARFCN ${c.earfcn}` : ''}
                </td>
                <td>
                  {isNum(c.distance_m) ? (
                    <span className="num">
                      {distanceWords(c.distance_m)} {COMPASS_WORDS[c.compass ?? ''] ?? c.compass}
                    </span>
                  ) : c.lookup ? (
                    <Badge tone={lookupTone(c.lookup.status)}>{lookupWord(c.lookup.status)}</Badge>
                  ) : (
                    <span className="muted small">identity hidden by the phone</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="muted small">
        Updated {around.age_s < 5 ? 'just now' : `${around.age_s} s ago`}. Your phone chooses its cell by itself; PULSE can show you
        the stronger ones but cannot move the phone onto them.
      </p>
    </Panel>
  );
}

function lookupWord(s: string): string {
  return s === 'found' ? 'located' : s === 'pending' ? 'looking up…' : s === 'not_found' ? 'position unknown' : s === 'error' ? 'lookup failed' : s === 'incomplete' ? 'IDs incomplete' : 'lookups off';
}
function lookupTone(s: string): string {
  return s === 'found' ? 'good' : s === 'not_found' || s === 'incomplete' ? 'fair' : s === 'error' ? 'poor' : 'neutral';
}

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
      return { text: 'position unknown', tone: 'fair' };
    case 'error':
      return { text: 'lookup failed', tone: 'poor' };
    case 'incomplete':
      return { text: 'IDs incomplete', tone: 'fair' };
    default:
      return { text: 'lookups off', tone: 'neutral' };
  }
}

function LookupOff() {
  return (
    <Panel kicker="Map" title="Tower lookups are off">
      <p className="small">
        rhcs knows which towers your router and phone use, but <code>TOWER_LOOKUP=off</code> in <code>.env</code> stops it asking public
        databases where they stand, so no positions are shown. Nothing is guessed.
      </p>
      <p className="muted small">
        To turn lookups back on, remove that line (or set <code>TOWER_LOOKUP=on</code>) and restart rhcsd. Only tower IDs are sent, never
        your position.
      </p>
    </Panel>
  );
}

function sourcesText(v: TowersView): string {
  const names = (v.lookup?.sources ?? ['beacondb']).map((x) => (x === 'beacondb' ? 'beaconDB' : x === 'opencellid' ? 'OpenCellID' : x));
  return names.join(' and ') || 'no database';
}

function CellTable({ cells, now, routerOn }: { cells: TowerCell[]; now: number; routerOn: boolean }) {
  const sorted = [...cells].sort((a, b) => b.last_seen - a.last_seen);
  return (
    <div className="table-wrap">
      <table className="table">
        <thead>
          <tr>
            <th scope="col">Cell</th>
            <th scope="col">PCI</th>
            <th scope="col">Band</th>
            {routerOn ? <th scope="col">Router: time · SINR avg / best · RSRP</th> : null}
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
                {routerOn ? (
                  <td>
                    {c.router ? <span className="num">{fmtDuration(c.router.secs)} · </span> : null}
                    <Sig st={c.router} />
                  </td>
                ) : null}
                <td>
                  {c.phone ? <span className="num">{fmtDuration(c.phone.secs)} · </span> : null}
                  <Sig st={c.phone} />
                  {!c.phone && c.nearby ? (
                    <span className="muted small num">
                      {' '}
                      heard, not served: {isNum(c.nearby.rsrp) ? `${fmt(c.nearby.rsrp)} dBm` : '—'}
                      {isNum(c.nearby.sinr) ? ` · ${fmt(c.nearby.sinr, 1)} dB` : ''}
                    </span>
                  ) : null}
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
        <TowerMap home={v.home} homeManual={v.home_manual ?? true} current={v.current ?? null} cells={v.cells} bestKey={v.best?.key ?? null} />
      ) : (
        <Empty title="No tower positions yet">
          {pending
            ? `Looking up ${pending} tower${pending === 1 ? '' : 's'} (${sourcesText(v)})…`
            : `None of the towers seen so far are in ${sourcesText(v)} yet. They are asked again weekly.`}
        </Empty>
      )}
      <p className="muted small">
        Lines run from {v.home_manual === false ? "the phone's position" : 'home'} to each tower; solid = serving now, colour = average SINR
        seen on it. Tower positions are estimates from {sourcesText(v)} and can be off by hundreds of metres. Only tower IDs are sent to
        them, never your position.
      </p>
    </Panel>
  );
}

export function TowersPage() {
  useTowersPolling();
  const { view: v, unsupported, at } = useTowers();
  const conn = useStore((s) => s.conn);
  const routerOn = useStore((s) => s.sources?.router_configured !== false);
  const now = useNow(5000);

  const head = (
    <header className="pagehead">
      <div>
        <div className="pagehead__kicker">{routerOn ? 'Router and phone cells' : 'Cells around this phone'}</div>
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
      <AroundPanel v={v} />
      <WhereBest v={v} />
      {v.lookup && !v.lookup.enabled ? <LookupOff /> : <MapPanel v={v} />}

      {v.best ? (
        <Panel kicker="Best direction" title="Cleanest tower so far" className="towerbest">
          <p>{v.best.message}</p>
          {!v.home ? <p className="muted small">Direction needs the tower's position and yours (the phone's, or a home location).</p> : null}
        </Panel>
      ) : null}

      <Panel kicker="Everything seen since rhcsd started keeping records" title="Cells">
        {v.cells.length ? (
          <CellTable cells={v.cells} now={now} routerOn={routerOn} />
        ) : (
          <Empty title="No cells recorded yet">They appear as soon as {routerOn ? 'the router or phone reports' : 'the phone reports'} its serving cell.</Empty>
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
