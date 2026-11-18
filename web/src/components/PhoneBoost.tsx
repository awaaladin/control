import { useState } from 'react';
import {
  phoneListApps,
  phoneOpenDndAccess,
  phoneOpenInternetPanel,
  phoneOpenUsageAccess,
  phoneSetEnabled,
  phoneSetGame,
  phoneSetOption,
  usePhoneBoost,
  usePhoneBoostPolling,
  type PhoneApp,
  type PhoneBoostStatus,
  type PhoneNet,
} from '../lib/phoneBoost';
import { Badge, Panel, Switch } from './ui';
import '../styles/extras.css';

function stateLine(s: PhoneBoostStatus): { tone: string; text: string } {
  switch (s.state) {
    case 'boosting':
      return { tone: 'excellent', text: `Boosting ${s.game?.label ?? 'your game'}` };
    case 'waiting':
      return { tone: 'accent', text: 'Waiting for a game' };
    case 'needs_usage':
      return { tone: 'poor', text: 'Needs Usage access to see when a game opens' };
    default:
      return { tone: 'neutral', text: 'Off' };
  }
}

function NetRow({ name, n }: { name: string; n: PhoneNet | undefined }) {
  const v = !n ? '—' : !n.available ? 'not connected' : n.ms === undefined ? 'no answer' : `${n.ms} ms`;
  return (
    <div className="phoneboost__net">
      <span>
        {name} {n?.in_use ? <Badge tone="accent">in use</Badge> : null}
      </span>
      <strong className="num">{v}</strong>
    </div>
  );
}

function GamePicker() {
  const [apps, setApps] = useState<PhoneApp[] | null>(null);
  const [q, setQ] = useState('');
  const load = () => setApps(phoneListApps());
  const shown = (apps ?? []).filter((a) => a.label.toLowerCase().includes(q.toLowerCase()));
  return (
    <details
      className="phoneboost__games"
      onToggle={(e) => {
        if ((e.currentTarget as HTMLDetailsElement).open && !apps) load();
      }}
    >
      <summary>Choose games</summary>
      <p className="muted small">
        Apps Android marks as games are found automatically. Tick any other app you want treated as a game.
      </p>
      <input className="input" type="search" placeholder="Search apps" value={q} onChange={(e) => setQ(e.target.value)} />
      <ul className="phoneboost__list">
        {shown.slice(0, 120).map((a) => (
          <li key={a.pkg}>
            <label>
              <input
                type="checkbox"
                checked={a.chosen}
                onChange={(e) => {
                  phoneSetGame(a.pkg, e.target.checked);
                  setApps((cur) => (cur ?? []).map((x) => (x.pkg === a.pkg ? { ...x, chosen: e.target.checked } : x)));
                }}
              />{' '}
              {a.label}
              {a.game ? <span className="muted small"> (Android lists it as a game)</span> : null}
            </label>
          </li>
        ))}
        {apps && shown.length === 0 ? <li className="muted small">No app matches.</li> : null}
      </ul>
    </details>
  );
}

/** Game Boost for this phone. Only rendered inside the Android app. */
export function PhoneBoost() {
  usePhoneBoostPolling();
  const s = usePhoneBoost((x) => x.status);

  if (!s) return null;
  if (!s.available) {
    return (
      <Panel kicker="This phone" title="Phone Game Boost">
        <p className="muted small">
          Phone Game Boost works when this page is opened from the engine built into the app. Open the built-in engine from the app&apos;s
          start screen to use it.
        </p>
      </Panel>
    );
  }

  const line = stateLine(s);
  const on = !!s.enabled;
  const rec = s.recommendation;
  return (
    <Panel
      className="phoneboost"
      kicker="This phone"
      title="Phone Game Boost"
      actions={<Badge tone={line.tone}>{line.text}</Badge>}
    >
      <Switch checked={on} onChange={phoneSetEnabled} label="Game Boost for this phone" />
      <p className="muted small">
        Off by default. When on, PULSE notices when a game opens, compares Wi-Fi and cellular delay every 10 seconds while you play, and tells you
        if the other one is clearly faster. Everything it changes is put back when the game closes or you switch this off.
      </p>

      {on && s.state === 'needs_usage' ? (
        <div className="phoneboost__need">
          <span>Usage access is needed so PULSE can tell which app is open. It reads only the name of the app in front.</span>
          <button type="button" className="btn btn--small" onClick={phoneOpenUsageAccess}>
            Grant usage access
          </button>
        </div>
      ) : null}

      {on ? (
        <div className="phoneboost__opts">
          <Switch checked={!!s.opt_dnd} onChange={(v) => phoneSetOption('dnd', v)} label="Silence notifications while gaming" />
          {s.opt_dnd && !s.dnd_access ? (
            <div className="phoneboost__need">
              <span>Do Not Disturb access is missing, so notifications are not silenced.</span>
              <button type="button" className="btn btn--small" onClick={phoneOpenDndAccess}>
                Grant Do Not Disturb access
              </button>
            </div>
          ) : null}
          <Switch checked={!!s.opt_lowlat} onChange={(v) => phoneSetOption('lowlat', v)} label="Keep Wi-Fi and CPU awake (best effort)" />
        </div>
      ) : null}

      {on && s.state === 'boosting' ? (
        <div className="phoneboost__live">
          <NetRow name="Wi-Fi" n={s.networks?.wifi} />
          <NetRow name="Cellular" n={s.networks?.cellular} />
          <p className="muted small">
            Delay is the time to open a connection to 1.1.1.1, middle of 3 tries, over each network
            {typeof s.measured_age_s === 'number' ? `, ${s.measured_age_s} s ago` : ''}.
          </p>
          {rec?.text ? <p className={rec.kind === 'switch' ? 'g-excellent' : ''}>{rec.text}</p> : null}
          {rec?.kind === 'switch' ? (
            <button type="button" className="btn" onClick={phoneOpenInternetPanel}>
              Open internet settings to switch
            </button>
          ) : null}
          {s.hint ? <p className="small">{s.hint}</p> : null}
          <ul className="phoneboost__status small muted">
            <li>Notifications silenced: {s.dnd_active ? 'yes, until the game closes' : s.opt_dnd ? 'no (needs access, or Do Not Disturb was already on)' : 'not requested'}</li>
            <li>
              Wi-Fi low-latency lock: {s.wifi_lock ? 'held' : 'not held'} (best effort
              {s.low_latency_api ? '' : '; this Android is too old for the low-latency mode'}). CPU wake lock: {s.wake_lock ? 'held' : 'not held'}.
            </li>
          </ul>
        </div>
      ) : on && s.state === 'waiting' ? (
        <p className="muted small">Open a game and measuring starts by itself.</p>
      ) : null}

      <GamePicker />

      <details className="phoneboost__can">
        <summary>What it can and cannot do</summary>
        <ul className="small">
          <li>Can: notice a game opening, measure Wi-Fi and cellular delay separately, and suggest the faster one with a one-tap button to the system internet settings.</li>
          <li>Can: silence notifications and keep Wi-Fi awake while you play, and put them back afterwards.</li>
          <li>Can: point to a saved better spot only when PULSE has real data for this place.</li>
          <li>Cannot: lock the phone to a cell tower, force 4G/5G or &quot;tower auto&quot;. Android does not allow apps to do that; only the carrier or a rooted phone can.</li>
          <li>Cannot: switch networks for you or move the game&apos;s traffic. You tap the button; Android then moves everything.</li>
          <li>Cannot: make a weak signal stronger. Moving to a better spot is the real fix.</li>
          <li>Wi-Fi low-latency mode is a hint to Android; many phones only honour it when the app is on screen, so it may do nothing for the game.</li>
        </ul>
      </details>
    </Panel>
  );
}
