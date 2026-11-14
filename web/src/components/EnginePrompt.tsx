// Friendly, dismissible prompt shown when PULSE finds no engine (rhcsd) anywhere.
// It never downloads by itself: the person has to click. It does not poll; it watches the
// connection state the app already maintains (the auto-search keeps retrying).

import { useEffect, useState } from 'react';
import { APK_URL } from './AppDownload';
import { detectOs, insideApp, snooze, snoozed, startEngineLink, WINDOWS_INSTALLER_SIZE, WINDOWS_INSTALLER_URL } from '../lib/engine';
import { useStore } from '../lib/store';

export function EnginePrompt() {
  const conn = useStore((s) => s.conn);
  const unreachable = useStore((s) => s.unreachable);
  const authIssue = useStore((s) => s.authIssue);
  const [dismissed, setDismissed] = useState(snoozed);
  const [shown, setShown] = useState(false); // has the prompt been visible this visit
  const [connected, setConnected] = useState(false);
  const os = detectOs();

  const noEngine = !insideApp() && conn !== 'live' && unreachable && !authIssue;

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
            <>
              <a className="btn btn--accent engine-prompt__main" href={WINDOWS_INSTALLER_URL}>
                ⬇ Download the engine ({WINDOWS_INSTALLER_SIZE})
              </a>
              <ol className="engine-prompt__steps">
                <li>Open the downloaded file PULSE-Setup.exe. Windows may warn about an unknown publisher: choose More info, then Run anyway.</li>
                <li>Press Install. The engine starts by itself.</li>
                <li>Come back to this page. It connects by itself.</li>
              </ol>
              <div className="engine-prompt__wait" role="status">
                <span className="dot dot--off" aria-hidden="true" /> Waiting for the engine…
              </div>
              <p className="muted engine-prompt__small">
                Already installed?{' '}
                <button type="button" className="linkbtn" onClick={startEngineLink}>
                  Start it
                </button>
              </p>
            </>
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
