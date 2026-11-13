import { useEffect } from 'react';
import { ConfirmDialog, DemoBadge, Rail, Toasts, TopBar, UnreachableOverlay, useRoute } from './components/Shell';
import { startPulse, stopPulse } from './lib/rhcs';
import { useStore } from './lib/store';
import { AimMode } from './pages/AimMode';
import { AudioPage } from './pages/Audio';
import { BoostPage } from './pages/Boost';
import { ControlPage } from './pages/Control';
import { LogPage } from './pages/Log';
import { NetworkPage } from './pages/Network';
import { PhonePage } from './pages/Phone';
import { SignalPage } from './pages/Signal';
import { TowersPage } from './pages/Towers';

let started = false;

export function App() {
  const [route, go] = useRoute();
  const hue = useStore((s) => s.prefs.accentHue);
  const conn = useStore((s) => s.conn);

  useEffect(() => {
    // Guard against StrictMode's double effect in development.
    if (started) return;
    started = true;
    startPulse();
    window.addEventListener('beforeunload', stopPulse);
  }, []);

  useEffect(() => {
    document.documentElement.style.setProperty('--accent-h', String(hue));
  }, [hue]);

  useEffect(() => {
    document.title = `PULSE · ${route[0].toUpperCase()}${route.slice(1)}${conn === 'demo' ? ' (demo)' : ''}`;
  }, [route, conn]);

  return (
    <div className={`app ${conn === 'demo' ? 'is-demo' : ''}`}>
      <a className="skip" href="#main">
        Skip to content
      </a>
      <Rail route={route} go={go} />
      <div className="app__main">
        <TopBar />
        <main id="main" className="app__content" tabIndex={-1}>
          {route === 'signal' && <SignalPage />}
          {route === 'boost' && <BoostPage />}
          {route === 'towers' && <TowersPage />}
          {route === 'phone' && <PhonePage />}
          {route === 'network' && <NetworkPage />}
          {route === 'audio' && <AudioPage />}
          {route === 'control' && <ControlPage />}
          {route === 'log' && <LogPage />}
        </main>
      </div>
      <DemoBadge />
      <AimMode />
      <UnreachableOverlay onSettings={() => go('control')} />
      <ConfirmDialog />
      <Toasts />
    </div>
  );
}
