// Friendly, dismissible prompt shown when PULSE finds no engine (rhcsd) anywhere.
// It never downloads by itself: the person has to click. It does not poll; it watches the
// connection state the app already maintains (the auto-search keeps retrying).

import { useEffect, useState } from 'react';
import { APK_URL } from './AppDownload';
import { detectOs, engineSeen, insideApp, markEngineSeen, snooze, snoozed, startEngineLink, startEngineQuietly, WINDOWS_INSTALLER_SIZE, WINDOWS_INSTALLER_URL } from '../lib/engine';
import { useStore } from '../lib/store';

export function EnginePrompt() {
  const conn = useStore((s) => s.conn);
  const unreachable = useStore((s) => s.unreachable);
  const authIssue = useStore((s) => s.authIssue);
  const [dismissed, setDismissed] = useState(snoozed);
  const [shown, setShown] = useState(false); // has the prompt been visible this visit
  const [connected, setConnected] = useState(false);
  const os = detectOs();

  const [seen, setSeen] = useState(engineSeen);
  const [starting, setStarting] = useState(false);

  const noEngine = !insideApp() && conn !== 'live' && unreachable && !authIssue;

  // Once the engine has answered here, remember it: next time the button says "Run", not "Download".
  useEffect(() => {
    if (conn === 'live' && !insideApp() && os === 'windows') {
      markEngineSeen();
      setSeen(true);
      setStarting(false);
    }
  }, [conn, os]);

  const run = () => {
    setStarting(true);
    startEngineLink();
    window.setTimeout(() => setStarting(false), 20000);
  };

  // Installed before and stopped now: try to start it once, quietly (the browser may ask once to allow it).
  useEffect(() => {
    if (os !== 'windows' || !noEngine || !seen) return;
    try {
      if (sessionStorage.getItem('pulse.autoRun')) return;
      sessionStorage.setItem('pulse.autoRun', '1');
    } catch {
      return;
    }
    setStarting(true);
    startEngineQuietly();
    const t = window.setTimeout(() => setStarting(false), 20000);
    return () => window.clearTimeout(t);
  }, [os, noEngine, seen]);

  useEffect(() => {
    if (noEngine && !dismissed) setShown(true);
  }, [noEngine, dismissed]);

  // The page found the engine by itself: say so briefly, then go away.
  useEffect(() => {
    if (conn === 'live' && shown) {
      setConnected(true);
      const t = window.setTimeout(() => {
        setShown(false);
        setConnected(false);
      }, 4000);
      return () => window.clearTimeout(t);
    }
  }, [conn, shown]);

  if (insideApp() || dismissed || !shown) return null;
  if (!noEngine && !connected) return null;

  const notNow = () => {
    snooze(7);
    setDismissed(true);
  };

  return (
    <aside className="engine-prompt panel" role="dialog" aria-labelledby="engine-prompt-title">
      <div className="panel__kicker">PULSE engine</div>
      <h2 id="engine-prompt-title" className="engine-prompt__title">
        {connected ? 'Connected' : 'PULSE needs its engine'}
      </h2>
      {connected ? (
        <p className="muted">The engine was found and PULSE is connected. You can stop it any time from the Control page.</p>
      ) : (
        <>
          <p className="muted">
            The engine (rhcsd) is a small program that talks to your router and devices. It stays on your computer; nothing is
            sent to us. PULSE found no engine here yet.
          </p>

          {os === 'windows' ? (
            seen ? (
              <>
                <p className="muted">PULSE is installed on this computer but is not running right now.</p>
                <button type="button" className="btn btn--accent engine-prompt__main" onClick={run} disabled={starting}>
                  {starting ? 'Starting…' : '▶ Run PULSE'}
                </button>
                <div className="engine-prompt__wait" role="status">
                  <span className="dot dot--off" aria-hidden="true" /> {starting ? 'Starting the engine, this page connects by itself…' : 'Press Run to start it.'}
                </div>
                <p className="muted engine-prompt__small">
                  If your browser asks to open PULSE, allow it (tick "always allow" so it never asks again). PULSE also starts by
                  itself when you sign in to Windows, so you rarely need this.{' '}
                  <a href={WINDOWS_INSTALLER_URL}>Reinstall</a>
                </p>
              </>
            ) : (
              <>
                <a className="btn btn--accent engine-prompt__main" href={WINDOWS_INSTALLER_URL} onClick={() => { markEngineSeen(); setSeen(true); }}>
                  ⬇ Download the engine ({WINDOWS_INSTALLER_SIZE})
                </a>
                <ol className="engine-prompt__steps">
                  <li>Open the downloaded file PULSE-Setup.exe. Windows may warn about an unknown publisher: choose More info, then Run anyway.</li>
                  <li>Press Install. The engine starts by itself and starts again every time you sign in.</li>
                  <li>Come back to this page. It connects by itself.</li>
                </ol>
                <div className="engine-prompt__wait" role="status">
                  <span className="dot dot--off" aria-hidden="true" /> Waiting for the engine…
                </div>
                <p className="muted engine-prompt__small">
                  Already installed?{' '}
                  <button type="button" className="linkbtn" onClick={run}>
                    Run it
                  </button>
                </p>
              </>
            )
          ) : os === 'android' ? (
            <>
              <p className="muted">On Android the engine comes inside the PULSE app, so there is nothing separate to install.</p>
              <a className="btn btn--accent engine-prompt__main" href={APK_URL} download="pulse.apk">
                ⬇ Get the app
              </a>
            </>
          ) : (
            <p className="muted">Windows and Android are supported today. Packages for macOS and Linux are planned.</p>
          )}
        </>
      )}
      <div className="engine-prompt__foot">
        <button type="button" className="btn btn--small" onClick={notNow}>
          {connected ? 'Close' : 'Not now'}
        </button>
      </div>
    </aside>
  );
}
