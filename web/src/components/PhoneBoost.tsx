import { useState } from 'react';
import {
  phoneListApps,
  phoneOpenDndAccess,
  phoneOpenInternetPanel,
  phoneOpenShizuku,
  phoneOpenShizukuDownload,
  phoneOpenUsageAccess,
  phoneOpenWifiSettings,
  phonePrivAction,
  phoneSetEnabled,
  phoneSetGame,
  phoneSetOption,
  usePhoneBoost,
  usePhoneBoostPolling,
  type PhoneApp,
  type PhoneBoostStatus,
  type PhoneNet,
  type PhonePriv,
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
  const v = !n ? 'â€”' : !n.available ? 'not connected' : n.ms === undefined ? 'no answer' : `${n.ms} ms`;
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


function kb(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

const SHIZUKU_STEPS: Record<string, string> = {
  not_installed: 'Shizuku is not installed. Install it, open it, and start it with Wireless debugging (the Shizuku app shows the steps).',
  not_running: 'Shizuku is installed but not running. Open it and press Start (Wireless debugging). It must be started again after every phone restart.',
  needs_permission: 'Shizuku is running. Allow PULSE to use it.',
  ready: 'Ready. PULSE can run a small fixed list of settings commands.',
};

/** Privileged mode through Shizuku (option B). */
function PrivSection({ p, gameOpen }: { p: PhonePriv | undefined; gameOpen: boolean }) {
  const [msg, setMsg] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<'repick' | 'mobile_only' | null>(null);
  if (!p) return null;
  const go = (a: 'request_permission' | 'repick' | 'mobile_only' | 'wifi_back', ok = false) => {
    setConfirm(null);
    setMsg(phonePrivAction(a, ok).message);
  };
  const tone = p.state === 'ready' ? 'excellent' : p.state === 'needs_permission' ? 'accent' : 'neutral';
  const label = { not_installed: 'Shizuku not installed', not_running: 'Shizuku not running', needs_permission: 'Needs permission', ready: 'Ready' }[p.state];
  return (
    <details className="phoneboost__can" open={p.state !== 'not_installed' || undefined}>
      <summary>Privileged mode (optional, needs Shizuku)</summary>
      <p className="small">
        <Badge tone={tone}>{label}</Badge> {SHIZUKU_STEPS[p.state]}
      </p>
      <p className="muted small">
        Shizuku is a free app that lets other apps run a few phone-settings commands after you start it yourself. PULSE only ever runs
        these fixed ones: Wi-Fi on/off, mobile data on/off, airplane mode on/off. The web page cannot send any other command.
      </p>
      {p.state === 'not_installed' ? (
        <button type="button" className="btn btn--small" onClick={phoneOpenShizukuDownload}>
          Get Shizuku (opens its download page)
        </button>
      ) : null}
      {p.state === 'not_running' ? (
        <button type="button" className="btn btn--small" onClick={phoneOpenShizuku}>
          Open Shizuku
        </button>
      ) : null}
      {p.state === 'needs_permission' ? (
        <>
          <button type="button" className="btn btn--small" onClick={() => go('request_permission')}>
            Allow PULSE in Shizuku
          </button>
          {p.denied_forever ? <p className="small">Shizuku will not ask again. Open Shizuku, Authorized apps, and switch PULSE on.</p> : null}
        </>
      ) : null}

      {p.state === 'ready' ? (
        <div className="phoneboost__opts">
          <p className="small">
            <strong>Re-pick tower.</strong> Your phone&apos;s modem chooses the tower, not PULSE. This only asks it to register again by
            switching airplane mode on for 2 seconds and off. It may land on the same tower or a better one. <strong>All connections drop for
            5 to 10 seconds</strong> (calls, downloads, games), so it is blocked while a game is open or was open in the last 2 minutes, and
            while a call is active. Never use it during a match.
          </p>
          {p.repick_blocked ? <p className="small g-poor">{p.repick_blocked}</p> : null}
          {confirm === 'repick' ? (
            <div className="phoneboost__need">
              <span>Really drop all connections for about 5 to 10 seconds?</span>
              <button type="button" className="btn btn--small" onClick={() => go('repick', true)}>
                Yes, re-pick now
              </button>
              <button type="button" className="btn btn--small" onClick={() => setConfirm(null)}>
                Cancel
              </button>
            </div>
          ) : (
            <button type="button" className="btn btn--small" disabled={!!p.repick_blocked || p.busy || gameOpen} onClick={() => setConfirm('repick')}>
              Re-pick tower
            </button>
          )}
          <Switch checked={!!p.tower_auto} onChange={(on) => phoneSetOption('tower_auto', on)} label="Switch tower by itself when my game lags" />
          <p className="muted small">
            Off by default. While a game is open on mobile data and lags for two checks in a row, PULSE turns airplane mode on for 2 seconds and
            off, with no tap. <strong>Your match drops for about 3 to 8 seconds each time.</strong> At most once every 3 minutes and 4 times an
            hour; never during a call or while your hotspot is on (airplane mode would switch the hotspot off).
          </p>
          <Switch checked={p.auto} onChange={(on) => phoneSetOption('priv_auto', on)} label="Re-pick automatically when signal is poor" />
          <p className="muted small">
            Off by default. Only while Game Boost is on, no game is open, you are on mobile data, signal has been at or below -110 dBm for 60
            seconds, and no call is active. At most once every 10 minutes.
          </p>

          <p className="small">
            <strong>Network choice.</strong> Turn Wi-Fi off so the phone uses mobile data only, or back on. PULSE puts Wi-Fi back by itself
            when your game closes, Game Boost is switched off or the app stops.
          </p>
          {confirm === 'mobile_only' ? (
            <div className="phoneboost__need">
              <span>Turn Wi-Fi off? Anything using Wi-Fi reconnects over mobile data (uses your data plan).</span>
              <button type="button" className="btn btn--small" onClick={() => go('mobile_only', true)}>
                Yes, mobile data only
              </button>
              <button type="button" className="btn btn--small" onClick={() => setConfirm(null)}>
                Cancel
              </button>
            </div>
          ) : (
            <span className="phoneboost__row">
              <button type="button" className="btn btn--small" disabled={p.busy} onClick={() => setConfirm('mobile_only')}>
                Use mobile data only
              </button>{' '}
              <button type="button" className="btn btn--small" disabled={p.busy} onClick={() => go('wifi_back')}>
                Use Wi-Fi again
              </button>
            </span>
          )}
        </div>
      ) : null}

      {p.wifi_off_by_us || p.airplane_pending ? (
        <div className="phoneboost__need">
          <span>
            {p.airplane_pending ? 'PULSE may have left airplane mode on. ' : ''}
            {p.wifi_off_by_us ? 'PULSE turned Wi-Fi off and will turn it back on.' : ''}
          </span>
          <button type="button" className="btn btn--small" onClick={phoneOpenWifiSettings}>
            Open Wi-Fi settings
          </button>
        </div>
      ) : null}
      {p.busy ? <p className="small">Working…</p> : null}
      {msg ? <p className="small">{msg}</p> : null}
      {p.last ? <p className="small muted">{p.last}</p> : null}
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
        Off by default. When on, PULSE notices when a game opens, compares Wi-Fi and cellular delay every {s.cost?.interval_s ?? 15} seconds while
        you play (only while the game is in front), and tells you if the other one is clearly faster. Everything it changes is put back when the game closes or you switch this off.
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
          <Switch checked={!!s.opt_lowlat} onChange={(v) => phoneSetOption('lowlat', v)} label="Keep Wi-Fi in low-latency mode while the game is in front (best effort)" />
        </div>
      ) : null}

      {on && s.state === 'boosting' ? (
        <div className="phoneboost__live">
          <NetRow name="Wi-Fi" n={s.networks?.wifi} />
          <NetRow name="Cellular" n={s.networks?.cellular} />
          <p className="muted small">
            Delay is the time to open a connection to 1.1.1.1, average of 2 tiny tries, over each network
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
              {s.low_latency_api ? '' : '; this Android is too old for the low-latency mode'}). PULSE holds no CPU wake lock.
            </li>
            {s.cost ? (
              <li>
                Boost cost: about {kb(s.cost.probe_bytes)} of data used by {s.cost.probes} test connections since Game Boost was switched on (a
                rough estimate; they carry no game data). Nothing is measured while no game is in front.
              </li>
            ) : null}
          </ul>
        </div>
      ) : on && s.state === 'waiting' ? (
        <p className="muted small">Open a game and measuring starts by itself.</p>
      ) : null}


      <GamePicker />

      <PrivSection p={s.priv} gameOpen={s.state === 'boosting'} />

      <details className="phoneboost__can">
        <summary>What it can and cannot do</summary>
        <ul className="small">
          <li>Can: notice a game opening and measure Wi-Fi and cellular delay separately, with a few tiny test connections.</li>
          <li>Can, with Shizuku: turn Wi-Fi off/on, and ask the modem to register again (airplane mode on for 2 s). Everything is put back afterwards.</li>
          <li>Can, with Shizuku: when mobile data lags for two checks in a row during a game, show a popup with a Switch tower button. Tapping it turns airplane mode on for about 2 s and off again so the phone picks a tower; the connection drops for about 3 to 8 s, and it never does this without your tap.</li>
          <li>Can: silence notifications and keep Wi-Fi in low-latency mode while the game is in front, and put them back afterwards.</li>
          <li>Can: point to a saved better spot only when PULSE has real data for this place.</li>
          <li>
            Cannot: choose or lock a cell tower, or force 4G/5G. The phone&apos;s modem and the carrier decide that. PULSE can only ask the
            modem to re-register, which drops every connection for 5 to 10 seconds, so it never does that during a game.
          </li>
          <li>Cannot: make a weak signal stronger. Moving to a better spot is the real fix.</li>
          <li>Cannot: speed up a game server that is far away.</li>
          <li>Wi-Fi low-latency mode is a hint to Android; many phones only honour it when the app is on screen, so it may do nothing for the game.</li>
        </ul>
      </details>
    </Panel>
  );
}
