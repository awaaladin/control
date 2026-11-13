import { useState } from 'react';
import { GlowRing } from '../components/GlowRing';
import { Badge, Empty, Icon, KV, Panel, Switch } from '../components/ui';
import {
  restoreLinks,
  runBoost,
  setAutoSwitch,
  stopBoostWatch,
  switchLink,
  useBoost,
  useBoostPolling,
  type BoostLink,
  type BoostState,
  type Hourly,
} from '../lib/boost';
import { fmt, fmtAgo, fmtRate, grade, type Grade, isNum } from '../lib/format';
import { confirmAction, toast, useStore } from '../lib/store';
import { useNow } from '../lib/ticker';
import '../styles/extras.css';

/** Ping grade for gaming: lower is better. */
const pingGrade = (v: number | null | undefined): Grade =>
  !isNum(v) ? 'none' : v <= 50 ? 'excellent' : v <= 90 ? 'good' : v <= 150 ? 'fair' : 'poor';
const jitterGrade = (v: number | null | undefined): Grade =>
  !isNum(v) ? 'none' : v <= 8 ? 'excellent' : v <= 15 ? 'good' : v <= 30 ? 'fair' : 'poor';
const lossGrade = (v: number | null | undefined): Grade =>
  !isNum(v) ? 'none' : v < 1 ? 'excellent' : v < 2 ? 'good' : v < 5 ? 'fair' : 'poor';

const KIND_LABEL: Record<BoostLink['kind'], string> = {
  router: '4G router',
  phone_usb: 'Phone, USB tethering',
  phone_hotspot: 'Phone, Wi-Fi hotspot',
  other: 'Other network',
};

function Sparkline({ values, g }: { values: (number | null)[]; g: Grade }) {
  const w = 220;
  const h = 44;
  const nums = values.filter(isNum);
  if (nums.length < 2) return <div className="spark spark--empty muted small">waiting for pings…</div>;
  const max = Math.max(...nums) * 1.1;
  const min = Math.min(0, ...nums);
  const x = (i: number) => (values.length <= 1 ? 0 : (i / (values.length - 1)) * w);
  const y = (v: number) => h - ((v - min) / (max - min || 1)) * h;
  let d = '';
  let pen = false;
  values.forEach((v, i) => {
    if (!isNum(v)) {
      pen = false;
      return;
    }
    d += `${pen ? 'L' : 'M'}${x(i).toFixed(1)},${y(v).toFixed(1)}`;
    pen = true;
  });
  return (
    <svg className={`spark g-${g}`} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" role="img" aria-label="Ping over the measurement">
      {values.map((v, i) => (!isNum(v) ? <line key={i} x1={x(i)} x2={x(i)} y1={0} y2={h} className="spark__loss" /> : null))}
      <path d={d} className="spark__line" />
    </svg>
  );
}

function RunButton({ st, onRun }: { st: BoostState | null; onRun: () => void }) {
  const busy = !!st && (st.running || st.phase === 'measuring');
  const pct = busy && st && st.total > 0 ? Math.min(1, st.done / st.total) : busy ? 0 : 1;
  const r = 86;
  const c = 2 * Math.PI * r;
  return (
    <button type="button" className={`boostbtn ${busy ? 'is-busy' : ''}`} onClick={onRun} disabled={busy} aria-live="polite">
      <svg viewBox="0 0 200 200" className="boostbtn__ring" aria-hidden="true">
        <circle cx="100" cy="100" r={r} className="boostbtn__track" />
        <circle
          cx="100"
          cy="100"
          r={r}
          className="boostbtn__arc"
          strokeDasharray={`${c * pct} ${c}`}
          transform="rotate(-90 100 100)"
        />
      </svg>
      <span className="boostbtn__inner">
        <Icon name="boost" size={34} />
        <span className="boostbtn__label">{busy ? `Measuring ${Math.round(pct * 100)} %` : 'Run Game Boost'}</span>
        <span className="boostbtn__sub">{busy ? 'about 5 seconds' : st?.finished ? 'run again' : 'tests every link'}</span>
      </span>
    </button>
  );
}

function LinkCard({ l, best, busy, admin, supported }: { l: BoostLink; best: boolean; busy: boolean; admin: boolean; supported: boolean }) {
  const g = pingGrade(l.ping_ms);
  return (
    <Panel
      className={`linkcard ${best ? 'is-best' : ''}`}
      kicker={KIND_LABEL[l.kind]}
      title={l.name}
      actions={
        <>
          {best ? <Badge tone="excellent">best</Badge> : null}
          {l.is_default ? <Badge tone="accent">in use now</Badge> : null}
        </>
      }
    >
      <div className="linkcard__body">
        <GlowRing
          label="Ping"
          value={l.ping_ms}
          range={[200, 10]}
          unit="ms"
          gradeOf={pingGrade}
          thresholds={[50, 90, 150]}
          isStale={() => false}
          size="sm"
          sub={busy && l.sent === 0 ? 'starting…' : undefined}
        />
        <dl className="kvs">
          <KV k="Jitter" v={<span className={`g-${jitterGrade(l.jitter_ms)}`}>{isNum(l.jitter_ms) ? `${fmt(l.jitter_ms, 1)} ms` : '—'}</span>} hint="How much the ping jumps between pings. Under 8 ms feels smooth." />
          <KV k="Loss" v={<span className={`g-${lossGrade(l.loss_pct)}`}>{isNum(l.loss_pct) ? `${fmt(l.loss_pct, 1)} %` : '—'}</span>} hint="Probes with no answer. Any loss causes stutter in games." />
          <KV k="Best / slow (p90)" v={isNum(l.best_ms) ? `${fmt(l.best_ms)} / ${fmt(l.p90_ms)} ms` : '—'} />
          <KV
            k={l.radio ? `Radio (${l.radio.source})` : 'Radio'}
            v={
              l.radio ? (
                <>
                  <span className={`g-${grade('sinr', l.radio.sinr)}`}>SINR {fmt(l.radio.sinr)} dB</span> ·{' '}
                  <span className={`g-${grade('rsrp', l.radio.rsrp)}`}>RSRP {fmt(l.radio.rsrp)}</span>
                </>
              ) : (
                <span className="muted">not available</span>
              )
            }
          />
          <KV k="Adapter" v={`${l.adapter} · ${l.local_ip}`} />
        </dl>
      </div>
      <Sparkline values={l.recent} g={g} />
      <div className={`linkcard__verdict g-${l.grade}`}>{l.error ?? (l.sent > 0 ? l.verdict : busy ? 'Measuring…' : 'Not measured yet')}</div>
      {l.targets.length ? (
        <details className="linkcard__targets">
          <summary>Per target</summary>
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Target</th>
                <th scope="col">Ping</th>
                <th scope="col">Jitter</th>
                <th scope="col">Answered</th>
              </tr>
            </thead>
            <tbody>
              {l.targets.map((t) => (
                <tr key={t.target}>
                  <td className="num">{t.target}</td>
                  <td className="num">{isNum(t.median_ms) ? `${fmt(t.median_ms)} ms` : '—'}</td>
                  <td className="num">{isNum(t.jitter_ms) ? `${fmt(t.jitter_ms, 1)} ms` : '—'}</td>
                  <td className="num">
                    {t.received}/{t.sent}
                    {t.refused ? <span className="muted"> · {t.refused} refused</span> : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      ) : null}
      {supported && !l.is_default ? (
        <div className="linkcard__switch">
          <button
            type="button"
            className="btn btn--small"
            disabled={!admin}
            title={admin ? undefined : 'Start rhcsd as administrator to switch links (see above).'}
            onClick={async () => {
              const ok = await confirmAction({
                title: `Switch to ${l.name}?`,
                body: `New connections will go through ${l.name} (Windows interface "${l.adapter}" gets the lowest metric, the other links a high one). Your current game keeps its connection; new connections use the new link. Undo any time with "Restore automatic".`,
                confirmLabel: 'Switch now',
              });
              if (ok) await switchLink(l.adapter);
            }}
          >
            Switch now
          </button>
        </div>
      ) : null}
    </Panel>
  );
}

/** Auto-switch toggle, restore button and admin status. */
function SwitchPanel({ st }: { st: BoostState }) {
  const now = useNow(5000);
  const sw = st.switch;
  const admin = st.admin ?? sw?.admin ?? false;
  const supported = sw?.supported ?? true;
  const auto = st.auto;
  if (!supported) {
    return (
      <Panel kicker="Switching" title="Switching links isn't available here">
        <p className="muted small">rhcsd can only change which link carries the internet on Windows. It still measures and recommends.</p>
      </Panel>
    );
  }
  return (
    <Panel
      kicker="Switching"
      title="Use the best link automatically"
      className="switchbar"
      actions={admin ? <Badge tone="excellent">administrator</Badge> : <Badge tone="fair">not administrator</Badge>}
    >
      <div className="switchbar__row">
        <Switch
          checked={!!auto?.enabled}
          disabled={!admin}
          onChange={(on) => void setAutoSwitch(on)}
          label="Auto-switch"
        />
        <button
          type="button"
          className="btn btn--small"
          disabled={!admin || !sw?.switched}
          onClick={async () => {
            const ok = await confirmAction({
              title: 'Restore automatic?',
              body: 'Puts every link rhcs changed back exactly as it was, so Windows chooses again. Auto-switch turns off. Running connections are not cut.',
              confirmLabel: 'Restore',
            });
            if (ok) await restoreLinks();
          }}
        >
          Restore automatic
        </button>
        {sw?.switched && sw.preferred ? <span className="small">rhcs has set <strong>{sw.preferred}</strong> as the preferred link.</span> : null}
      </div>
      <p className="small">
        Auto-switch keeps measuring gently (a few tiny connection tests per link every 12 s) and switches only when another link is clearly
        better (at least 20 % and 15 ms) three times in a row, and never more than once every 2 minutes.{' '}
        <strong>Your current game keeps its connection; new connections use the new link.</strong>
      </p>
      {auto?.last ? (
        <p className={`switchbar__last small ${auto.last.action === 'switch' ? 'g-excellent' : auto.last.action === 'error' ? 'g-poor' : 'muted'}`}>
          Last check {fmtAgo(auto.last.ts, now)}: {auto.last.message}
        </p>
      ) : null}
      {sw?.last_message ? <p className="switchbar__last muted small">Last change: {sw.last_message}</p> : null}
      {!admin ? (
        <div className="adminhelp small">
          <strong>Switching needs rhcsd to run as administrator.</strong> Measuring works without it.
          <ol>
            <li>In rhcsd's window press <strong>Ctrl+C</strong> to stop it.</li>
            <li>Start menu → type <strong>PowerShell</strong> → right-click <strong>Windows PowerShell</strong> → <strong>Run as administrator</strong>.</li>
            <li>
              Run <code>cd D:\setup
              Run <code>{'cd D:\\setup\\rhcs'}</code> then <code>{'.\\target\\debug\\rhcsd.exe'}</code>
hcsd.exe</code>
            </li>
            <li>Reload this page: the switch and the "Switch now" buttons turn on.</li>
          </ol>
        </div>
      ) : null}
    </Panel>
  );
}

function copy(text: string) {
  if (navigator.clipboard && window.isSecureContext) {
    navigator.clipboard.writeText(text).then(
      () => toast('ok', 'Copied', 'Paste it into an admin PowerShell.'),
      () => toast('warn', 'Copy failed', 'Select the command and copy it by hand.'),
    );
  } else toast('info', 'Select and copy', 'This browser only allows copying on the laptop itself. Select the command and copy it by hand.');
}

function Recommendation({ st }: { st: BoostState }) {
  const r = st.recommendation;
  if (!r) return null;
  return (
    <Panel className="boostrec" kicker="Recommendation" title={r.headline}>
      <p className="boostrec__why">{r.explanation}</p>
      <div className="row">
        {r.already_default ? <Badge tone="excellent">already on the best link</Badge> : null}
        {r.close_call ? <Badge tone="good">links are about equal</Badge> : null}
      </div>
      {r.commands.length ? (
        <div className="boostrec__cmds">
          <div className="muted small">Admin PowerShell, in the rhcs folder. Undo any time with -Undo:</div>
          {r.commands.map((c) => (
            <div className="cmd" key={c}>
              <code className="num">{c}</code>
              <button type="button" className="btn btn--small" onClick={() => copy(c)}>
                Copy
              </button>
            </div>
          ))}
        </div>
      ) : null}
    </Panel>
  );
}

function BestHours({ hourly }: { hourly: Hourly | null }) {
  if (!hourly) return null;
  const rows = hourly.hours.filter((h) => h.samples >= 60 && isNum(h.avg_sinr));
  if (hourly.covered_h < 2 || rows.length < 2) {
    return (
      <Panel kicker="From the router's history" title="Best time of day">
        <p className="muted small">
          Not enough history yet ({fmt(hourly.covered_h, 1)} h). rhcsd keeps the last 24 hours of router readings in memory, starting
          when it was launched. Leave it running and check back later.
        </p>
      </Panel>
    );
  }
  const best = [...rows].sort((a, b) => (b.avg_sinr ?? 0) - (a.avg_sinr ?? 0)).slice(0, 3);
  const bestSet = new Set(best.map((b) => b.hour));
  const lo = Math.min(-5, ...rows.map((r) => r.avg_sinr ?? 0));
  const hi = Math.max(25, ...rows.map((r) => r.avg_sinr ?? 0));
  const pad = (n: number) => `${String(n).padStart(2, '0')}:00`;
  return (
    <Panel kicker="From the router's history" title="Best time of day">
      <p className="small">
        Cleanest signal (average SINR) at {best.map((b) => pad(b.hour)).join(', ')}. Based on {fmt(hourly.covered_h, 1)} h of router
        readings; evenings are usually busiest on mobile networks.
      </p>
      <div className="hours" role="img" aria-label="Average SINR by hour of day">
        {hourly.hours.map((h) => {
          const ok = h.samples >= 60 && isNum(h.avg_sinr);
          const v = ok ? (h.avg_sinr as number) : null;
          const pct = v === null ? 0 : ((v - lo) / (hi - lo)) * 100;
          return (
            <div
              key={h.hour}
              className={`hours__col ${bestSet.has(h.hour) ? 'is-best' : ''}`}
              title={ok ? `${pad(h.hour)} · SINR ${fmt(v, 1)} dB · down ${fmtRate(h.avg_rx_bps)}` : `${pad(h.hour)} · no data`}
            >
              <div className={`hours__bar g-${grade('sinr', v)}`} style={{ height: `${Math.max(3, pct)}%` }} />
              <div className="hours__h num">{h.hour % 3 === 0 ? h.hour : ''}</div>
            </div>
          );
        })}
      </div>
    </Panel>
  );
}

function Tips({ st }: { st: BoostState | null }) {
  const setAim = useStore((s) => s.setAim);
  const devices = useStore((s) => s.devices);
  const phone = useStore((s) => s.phone);
  const others = devices.filter((d) => !d.is_self && !d.is_gateway).length + (phone?.hotspot_clients?.length ?? 0);
  const usesPhone = st?.links.some((l) => l.kind !== 'router' && l.kind !== 'other');
  return (
    <Panel kicker="What else helps" title="Tips for lower ping">
      <ol className="tips">
        <li>
          <strong>Put the router where the signal is cleanest.</strong> Higher SINR means fewer resends and steadier ping.{' '}
          <button type="button" className="linkbtn" onClick={() => setAim(true)}>
            Open Aim mode
          </button>
        </li>
        <li>
          <strong>Pause other heavy users while you play.</strong> Downloads, video, app updates and phone backups on the same link
          raise everyone's ping. {others > 0 ? `${others} other device${others === 1 ? ' is' : 's are'} on your networks right now.` : ''}
        </li>
        <li>
          <strong>USB beats Wi-Fi.</strong> USB tethering avoids Wi-Fi interference, so it is usually steadier than the phone's hotspot.
        </li>
        {usesPhone ? (
          <li>
            <strong>Keep the phone cool and charging.</strong> A hot phone slows its modem down.
            {isNum(phone?.battery_temp_c) ? ` It is ${fmt(phone?.battery_temp_c, 1)} °C now.` : ''}
          </li>
        ) : null}
        <li>
          <strong>Pick the nearest server in the game.</strong> Game Boost tests general internet targets, not the game's own servers;
          the game's own ping display is the final word.
        </li>
      </ol>
      <p className="muted small">
        What Game Boost cannot do: make the radio signal stronger, or change how your mobile operator routes traffic. Switching only
        changes which link new connections use.
      </p>
    </Panel>
  );
}

export function BoostPage() {
  useBoostPolling();
  const { state: st, unsupported, at } = useBoost();
  const hourly = useBoost((s) => s.hourly);
  const conn = useStore((s) => s.conn);
  const now = useNow(2000);
  const [watch, setWatch] = useState(false);
  const busy = !!st && (st.running || st.phase === 'measuring');
  const watching = st?.watch ?? false;

  const head = (
    <header className="pagehead">
      <div>
        <div className="pagehead__kicker">Lowest, steadiest ping</div>
        <h1 className="pagehead__title">Game Boost</h1>
      </div>
      <div className="pagehead__actions">
        {watching ? (
          <button type="button" className="btn btn--small" onClick={() => void stopBoostWatch()}>
            Stop watching
          </button>
        ) : (
          <Switch checked={watch} onChange={setWatch} label="Keep measuring" />
        )}
        {st?.finished ? <Badge>{`last run ${fmtAgo(st.finished, now)}`}</Badge> : null}
      </div>
    </header>
  );

  if (unsupported) {
    return (
      <div className="page">
        {head}
        <Panel>
          <Empty title="This rhcsd has no Game Boost">The backend answered but doesn't know /api/boost. Update and restart rhcsd.</Empty>
        </Panel>
      </div>
    );
  }
  if (conn !== 'live' && !at) {
    return (
      <div className="page">
        {head}
        <Panel>
          <Empty title="Not connected">Game Boost runs on the home system. It will appear once PULSE is connected.</Empty>
        </Panel>
      </div>
    );
  }

  const bestId = st?.phase === 'done' ? st.recommendation?.best : undefined;
  return (
    <div className="page page--boost">
      {head}
      <div className="boost-hero">
        <RunButton st={st} onRun={() => void runBoost(watch)} />
        <div className="boost-hero__text">
          <p>
            Game Boost pings the internet through <strong>each</strong> of this computer's connections (router, phone over USB, phone
            hotspot) {st ? `${st.probes} times, ${st.interval_ms} ms apart,` : ''} and tells you which one gives the lowest and steadiest
            ping.
          </p>
          <p className="muted small">
            It measures and recommends; with rhcsd running as administrator it can also switch for you (below). It cannot make the
            signal stronger. Targets: {st?.targets.join(', ') || '—'}
          </p>
          {st?.error ? <div className="boost-error g-poor">{st.error}</div> : null}
        </div>
      </div>

      {st ? <SwitchPanel st={st} /> : null}

      {st?.phase === 'done' ? <Recommendation st={st} /> : null}

      {st && st.links.length ? (
        <div className="linkgrid">
          {st.links.map((l) => (
            <LinkCard key={l.id} l={l} best={l.id === bestId} busy={busy} admin={st.admin ?? false} supported={st.switch?.supported ?? true} />
          ))}
        </div>
      ) : st && st.phase === 'idle' ? (
        <Panel>
          <Empty title="No measurement yet">Press Run Game Boost. It takes about five seconds.</Empty>
        </Panel>
      ) : null}

      <div className="boost-side">
        <Tips st={st} />
        <BestHours hourly={hourly} />
      </div>
    </div>
  );
}
