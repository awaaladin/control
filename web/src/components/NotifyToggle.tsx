import { insideApp } from '../lib/engine';
import { useState } from 'react';
import { notifyEnabled, notifySupported, setNotify } from '../lib/notify';
import { toast } from '../lib/store';
import { Switch } from './ui';

/** "Alert me when an unknown device joins" switch. */
export function NotifyToggle() {
  const [on, setOn] = useState(notifyEnabled);
  const change = async (v: boolean) => {
    if (v && !notifySupported()) {
      toast('warn', 'Notifications not available here', insideApp() ? 'Alerts from the engine come as Android notifications. Allow notifications for PULSE in Android settings.' : 'Browsers only allow notifications on https pages or on the laptop itself (http://localhost:7878).', 9000);
      return;
    }
    const now = await setNotify(v);
    if (v && !now) toast('warn', 'Notifications blocked', insideApp() ? 'Allow notifications for PULSE in Android settings (Apps, PULSE, Notifications), then try again.' : 'Allow notifications for this site in the browser settings, then try again.');
    setOn(now);
  };
  return <Switch checked={on} onChange={(v) => void change(v)} label="Alert on unknown devices" />;
}
