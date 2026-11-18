import { useEffect, useMemo, useState } from 'react';
import { Badge, Empty, Panel } from './ui';
import {
  planDevBoost,
  startDevBoost,
  stopDevBoost,
  useDevBoost,
  useDevBoostPolling,
  type DevAction,
  type DevCandidate,
  type DevResult,
  type DevSnap,
  type DevView,
} from '../lib/devboost';
import { fmt, fmtAgo, isNum } from '../lib/format';
import { confirmAction } from '../lib/store';
import { useNow } from '../lib/ticker';
import '../styles/extras.css';

const BG_ACTION = 'background_low_priority';

function delta(a: number | null | undefined, b: number | null | undefined, digits = 0): string {
  if (!isNum(a) || !isNum(b)) return '';
  const d = b - a;
  if (Math.abs(d) < Math.pow(10, -digits) / 2) return 'same';
  return `${d > 0 ? '+' : '−'}${Math.abs(d).toFixed(digits)}`;
}

function ResultCard({ r }: { r: DevResult }) {
  const now = useNow(5000);
  const tone = r.verdict.improved === true ? 'g-excellent' : r.verdict.improved === false ? 'g-fair' : 'muted';
  const row = (label: string, k: keyof Pick<DevSnap, 'ping_ms' | 'jitter_ms' | 'loss_pct'>, unit: string, digits: number) => (
    <tr>
      <th scope="row">{label}</th>
      <td className="num">{isNum(r.before?.[k]) ? `${fmt(r.before?.[k], digits)} ${unit}` : '—'}</td>
      <td className="num">{isNum(r.after?.[k]) ? `${fmt(r.after?.[k], digits)} ${unit}` : '—'}</td>
      <td className="num">{delta(r.before?.[k], r.after?.[k], digits)}</td>
    </tr>
  );
  return (
    <Panel kicker={`Measured ${fmtAgo(r.finished_at, now)}`} title="Before and after" className="devres">
      <p className={`devres__verdict ${tone}`}>{r.verdict.text}</p>
      <table className="table devres__table">
        <thead>
          <tr>
            <th scope="col">{r.after?.link_name ?? r.before?.link_name ?? 'Connection'}</th>
            <th scope="col">Before</th>
            <th scope="col">After</th>
            <th scope="col">Change</th>
          </tr>
        </thead>
        <tbody>
          {row('Ping', 'ping_ms', 'ms', 0)}
          {row('Jitter', 'jitter_ms', 'ms', 1)}
          {row('Packet loss', 'loss_pct', '%', 1)}
        </tbody>
      </table>
      <p className="muted small">
        Both numbers are real TCP connection tests to public servers through the connection Windows used at that moment, about 10 seconds
        apart. They show your network, not the game's own server, so the game's ping display is the final word.
      </p>
    </Panel>
  );
}

function ActiveCard({ v }: { v: DevView }) {
  const now = useNow(1000);
  const s = v.state;
  const live = s.live ?? s.before;
  const busy = s.phase !== 'active';
  const title =
    s.phase === 'measuring_before'
      ? 'Measuring your connection first…'
      : s.phase === 'applying'
        ? 'Applying the changes…'
        : s.phase === 'restoring'
          ? 'Restoring…'
          : s.phase === 'restore_failed'
            ? 'Some changes could not be restored'
            : 'Your PC is boosted';
  return (
    <Panel
      kicker={s.game ? `Game: ${s.game.name}` : 'Boost on'}
      title={title}
      className="devactive"
      actions={s.active ? <Badge tone="excellent" pulse>{`on for ${fmtAgo(s.started_at, now).replace(' ago', '')}`}</Badge> : null}
    >
      {s.phase === 'measuring_before' ? (
        <p className="small">Nothing has been changed yet. The ping is measured first so there is a real "before" to compare with.</p>
      ) : null}
      {s.phase === 'restore_failed' ? (
        <p className="small g-poor">{s.message ?? 'Press Restore everything to try again.'} Restarting rhcsd also retries.</p>
      ) : null}
      {s.changes.length ? (
        <>
          <div className="muted small">Changed right now:</div>
          <ul className="devactive__list">
            {s.changes.map((c, i) => (
              <li key={`${c.id}-${i}`}>
                <strong>{c.title}</strong> <span className="muted">{c.detail}</span>
                {!c.reversible ? <span className="muted"> (cannot be undone, harmless)</span> : null}
              </li>
            ))}
          </ul>
        </>
      ) : null}
      {s.phase === 'active' ? (
        <div className="devactive__live">
          <span className="muted small">Live ping on {live?.link_name ?? 'the connection in use'}</span>
          <span className="devactive__ping num">{isNum(live?.ping_ms) ? `${fmt(live?.ping_ms)} ms` : 'measuring…'}</span>
          {isNum(live?.jitter_ms) ? <span className="muted small num">jitter {fmt(live?.jitter_ms, 1)} ms</span> : null}
          {live ? <span className="muted small">{fmtAgo(live.at, now)}</span> : null}
        </div>
      ) : null}
      {s.game ? <p className="muted small">When {s.game.name} closes, everything is put back automatically. Closing rhcsd normally does the same; if it is ever killed, the next start restores from its saved list.</p> : null}
      <div className="row">
        <button type="button" className="btn btn--accent" disabled={s.phase === 'restoring'} onClick={() => void stopDevBoost()}>
          Restore everything
        </button>
        {busy && s.phase !== 'restore_failed' ? <span className="muted small">{s.phase === 'measuring_before' ? 'You can cancel now: nothing is changed yet.' : ''}</span> : null}
      </div>
    </Panel>
  );
}

function Picker({
  v,
  game,
  setGame,
  bg,
  setBg,
  bgOn,
}: {
  v: DevView;
  game: number | null;
  setGame: (p: number | null) => void;
  bg: Set<number>;
  setBg: (s: Set<number>) => void;
  bgOn: boolean;
}) {
  const list = v.candidates;
  const likely = list.filter((c) => c.likely_game);
  const others = list.filter((c) => !c.likely_game);
  const row = (c: DevCandidate) => (
    <label key={c.pid} className={`devpick__row ${game === c.pid ? 'is-on' : ''}`}>
      <input type="radio" name="devgame" checked={game === c.pid} onChange={() => setGame(c.pid)} />
      <span className="devpick__name">{c.name}</span>
      {c.hint ? <Badge tone="good">{c.hint}</Badge> : null}
      <span className="muted small devpick__sub">{c.window_title ? `${c.window_title} · ` : ''}{c.mem_mb} MB</span>
    </label>
  );
  const bgCandidates = list.filter((c) => c.pid !== game).slice(0, 14);
  return (
    <div className="devpick">
      <div className="devpick__col">
        <h3 className="devpick__h">1. Pick the game (or the emulator running it)</h3>
        {list.length === 0 ? (
          <Empty title="No program with a window found">Open the game or emulator first; it appears here within a few seconds.</Empty>
        ) : (
          <div className="devpick__list" role="radiogroup" aria-label="Game">
            {likely.map(row)}
            {likely.length && others.length ? <div className="muted small devpick__sep">other open programs</div> : null}
            {others.slice(0, 14).map(row)}
          </div>
        )}
        <p className="muted small">"Looks like a game" is only a guess from the program's name. You choose; nothing is assumed.</p>
      </div>
      <div className="devpick__col">
        <h3 className="devpick__h">2. Lower the priority of programs you do not need (optional)</h3>
        {!bgOn ? <p className="muted small">That action is switched off below.</p> : null}
        <div className="devpick__list">
          {bgCandidates.map((c) => (
            <label key={c.pid} className="devpick__row">
              <input
                type="checkbox"
                disabled={!bgOn}
                checked={bg.has(c.pid)}
                onChange={(e) => {
                  const n = new Set(bg);
                  if (e.target.checked) n.add(c.pid);
                  else n.delete(c.pid);
                  setBg(n);
                }}
              />
              <span className="devpick__name">{c.name}</span>
              <span className="muted small devpick__sub">{c.mem_mb} MB</span>
            </label>
          ))}
        </div>
        <p className="muted small">They keep running; Windows just serves the game first. Nothing is closed.</p>
      </div>
    </div>
  );
}

function ActionRow({ a, on, setOn }: { a: DevAction; on: boolean; setOn: (b: boolean) => void }) {
  return (
    <label className={`devact ${a.available ? '' : 'is-off'}`}>
      <input type="checkbox" checked={on && a.available} disabled={!a.available} onChange={(e) => setOn(e.target.checked)} />
      <span className="devact__text">
        <span className="devact__title">
          {a.title} {a.needs_admin ? <Badge tone={a.available ? 'accent' : 'fair'}>administrator</Badge> : null}
        </span>
        <span className="muted small">{a.description}</span>
        {a.detail ? <span className={`small ${a.available ? 'muted' : 'g-fair'}`}>{a.detail}</span> : null}
      </span>
    </label>
  );
}

/** "Boost this PC for gaming": pick the game, choose what to change, see exactly what will
 * change, boost, and see the measured before / after. Everything is restored on demand, when the
 * game closes, and after a crash. */
export function DeviceBoost() {
  useDevBoostPolling();
  const { view: v, unsupported } = useDevBoost();
  const [game, setGame] = useState<number | null>(null);
  const [bg, setBg] = useState<Set<number>>(new Set());
  const [checked, setChecked] = useState<Record<string, boolean> | null>(null);
  const [busy, setBusy] = useState(false);

  // Start from the recommended choices once the list has arrived.
  useEffect(() => {
    if (v && !checked) setChecked(Object.fromEntries(v.actions.map((a) => [a.id, a.default_on && a.available])));
  }, [v, checked]);

  // Forget a picked program that has closed.
  useEffect(() => {
    if (!v) return;
    const alive = new Set(v.candidates.map((c) => c.pid));
    if (game !== null && !alive.has(game)) setGame(null);
    if ([...bg].some((p) => !alive.has(p))) setBg(new Set([...bg].filter((p) => alive.has(p))));
  }, [v, game, bg]);

  const bgOn = !!checked?.[BG_ACTION];
  const chosen = useMemo(() => (v && checked ? v.actions.filter((a) => checked[a.id] && a.available).map((a) => a.id) : []), [v, checked]);

  const head = (
    <div className="devboost__intro">
      <p>
        <strong>No software can make your radio signal stronger.</strong> Boost frees your PC from background load, raises the game's
        priority and picks the best link; PULSE shows the measured ping before and after.
      </p>
    </div>
  );

  if (unsupported) {
    return (
      <Panel kicker="This PC" title="Boost this PC for gaming">
        {head}
        <Empty title="This rhcsd cannot boost the PC">The backend has no /api/boost/device. Update and restart rhcsd.</Empty>
      </Panel>
    );
  }
  if (!v) {
    return (
      <Panel kicker="This PC" title="Boost this PC for gaming">
        {head}
        <p className="muted small">Loading…</p>
      </Panel>
    );
  }
  if (!v.supported) {
    return (
      <Panel kicker="This PC" title="Boost this PC for gaming">
        {head}
        <Empty title="Not available here">
          {v.unsupported_reason ?? 'Boosting the PC works on Windows only.'} The ping measurement above still works everywhere.
        </Empty>
      </Panel>
    );
  }

  const running = v.state.phase !== 'idle';

  const onBoost = async () => {
    if (!checked) return;
    const actions = chosen.filter((id) => id !== BG_ACTION || bg.size > 0);
    const req = { actions, game_pid: game ?? undefined, background_pids: [...bg] };
    setBusy(true);
    try {
      const plan = await planDevBoost(req);
      if (plan.error) {
        await confirmAction({ title: 'Cannot boost with these choices', body: plan.error, confirmLabel: 'OK' });
        return;
      }
      if (plan.items.length === 0) {
        const why = plan.skipped.map((s) => `${s.id.replace(/_/g, ' ')}: ${s.reason}`).join('\n');
        await confirmAction({ title: 'Nothing to change', body: why || 'Pick at least one action.', confirmLabel: 'OK' });
        return;
      }
      const lines = plan.items.map((i) => `• ${i.detail}`).join('\n');
      const skipped = plan.skipped.length ? `\n\nSkipped:\n${plan.skipped.map((s) => `• ${s.id.replace(/_/g, ' ')}: ${s.reason}`).join('\n')}` : '';
      const ok = await confirmAction({
        title: 'Boost this PC now?',
        body: `PULSE will measure your ping, then change exactly this:\n\n${lines}${skipped}\n\nNothing is closed or deleted. Everything is put back when you press Restore everything, when the game closes, or after a restart.`,
        confirmLabel: 'Boost now',
      });
      if (ok) await startDevBoost(req);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="devboost">
      <Panel
        kicker="This PC"
        title="Boost this PC for gaming"
        actions={v.admin ? <Badge tone="excellent">administrator</Badge> : <Badge tone="fair">not administrator</Badge>}
      >
        {head}
        {running ? (
          <p className="muted small">A boost is in progress: see below.</p>
        ) : (
          <>
            <Picker v={v} game={game} setGame={setGame} bg={bg} setBg={setBg} bgOn={bgOn} />
            <h3 className="devpick__h">3. Choose what to change</h3>
            <div className="devacts">
              {v.actions.map((a) => (
                <ActionRow key={a.id} a={a} on={!!checked?.[a.id]} setOn={(b) => setChecked({ ...(checked ?? {}), [a.id]: b })} />
              ))}
            </div>
            {!v.admin ? (
              <p className="muted small">
                Actions marked <em>administrator</em> need rhcsd started as administrator (see "Use the best link automatically" above).
                The others work without.
              </p>
            ) : null}
            <div className="row devboost__go">
              <button type="button" className="btn btn--accent devboost__btn" disabled={busy || chosen.length === 0} onClick={() => void onBoost()}>
                {busy ? 'Checking…' : 'Boost now'}
              </button>
              <span className="muted small">You will see exactly what changes before anything happens.</span>
            </div>
          </>
        )}
      </Panel>
      {running ? <ActiveCard v={v} /> : null}
      {v.last_result ? <ResultCard r={v.last_result} /> : null}
      {!running && v.state.message ? <p className="muted small devboost__msg">{v.state.message}</p> : null}
    </div>
  );
}
