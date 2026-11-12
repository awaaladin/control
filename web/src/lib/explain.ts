// Turns raw backend error strings into plain-language explanations with fixes.
// Matching is on substrings of the messages rhcsd actually produces.

export type ErrorSource = 'router' | 'phone';

export interface Explained {
  title: string;
  detail: string;
  fixes: string[];
  raw: string;
}

interface Rule {
  match: RegExp;
  title: string;
  detail: string;
  fixes: string[];
}

const ROUTER: Rule[] = [
  {
    match: /no router password|ROUTER_ADMIN_PASSWORD in \.env/i,
    title: 'No router password configured',
    detail: 'rhcsd can read the router’s public fields, but needs the admin password for signal, tower and data details.',
    fixes: ['Add ROUTER_ADMIN_PASSWORD=… to the .env file next to rhcsd.', 'Restart rhcsd.'],
  },
  {
    match: /attempt\(s\) left|locks; not trying/i,
    title: 'Login paused to protect the router',
    detail: 'The router locks its login after 5 wrong passwords. rhcsd stopped trying before that could happen.',
    fixes: [
      'Open the router page in a browser and log in once with the correct password; that resets the counter.',
      'Check ROUTER_ADMIN_PASSWORD in .env.',
      'Then press Re-login on the Control page.',
    ],
  },
  {
    match: /login rejected|check ROUTER_ADMIN_PASSWORD/i,
    title: 'The router rejected the password',
    detail: 'rhcsd reached the router, but the admin password in .env was not accepted. It will not retry on its own, so it can’t lock you out.',
    fixes: ['Fix ROUTER_ADMIN_PASSWORD in .env and restart rhcsd.', 'Or press Re-login on the Control page after fixing it.'],
  },
  {
    match: /login challenge|\bLD\b/i,
    title: 'The router didn’t offer a login',
    detail: 'The device at the router address answered, but not like a supported ZTE router.',
    fixes: ['Check ROUTER_IP in .env points at the 4G router.', 'Make sure the laptop is on that router’s Wi-Fi or cable.'],
  },
  {
    match: /non-JSON|not an HTTP response|not an object/i,
    title: 'The router answered in an unexpected format',
    detail: 'The router replied, but rhcsd couldn’t understand the reply. This usually means a different firmware version.',
    fixes: ['Update rhcs to the latest version.', 'If it persists, open an issue with the router model and firmware (no passwords or IDs).'],
  },
  {
    match: /timed out|refused|unreachable|os error 100(5[01]|60|61|64|65)|No route|network is unreachable/i,
    title: 'Can’t reach the router',
    detail: 'rhcsd sent requests to the router and got no answer. It keeps retrying every 5 seconds and reconnects by itself.',
    fixes: [
      'Is the laptop connected to the router (its Wi-Fi or an Ethernet cable)?',
      'Is the router powered on?',
      'Does ROUTER_IP in .env match the router (usually 192.168.0.1)?',
      'Using two connections at once? Run scripts\\lan-only.ps1 so internet and router traffic don’t fight.',
    ],
  },
];

const PHONE: Rule[] = [
  {
    match: /could not run adb|program not found|cannot find the file|No such file/i,
    title: 'ADB is not installed or not found',
    detail: 'rhcsd talks to the phone through Android’s adb tool, and couldn’t start it.',
    fixes: ['Install Android platform-tools.', 'Or set ADB_PATH in .env to the full path of adb.exe, then restart rhcsd.'],
  },
  {
    match: /unauthori[sz]ed/i,
    title: 'The phone hasn’t allowed this computer yet',
    detail: 'The phone is connected, but USB debugging permission hasn’t been granted to this laptop.',
    fixes: ['Unlock the phone and tap “Allow” on the USB debugging prompt.', 'Tick “Always allow from this computer”.'],
  },
  {
    match: /more than one device|more than one emulator/i,
    title: 'More than one Android device is connected',
    detail: 'adb doesn’t know which device to use.',
    fixes: ['Set PHONE_ADB_SERIAL in .env (see `adb devices`), then restart rhcsd.'],
  },
  {
    match: /failed to connect|cannot connect to/i,
    title: 'Can’t reach the phone over Wi-Fi',
    detail: 'ADB over Wi-Fi (PHONE_ADB_ADDR) didn’t answer.',
    fixes: [
      'Phone and laptop must be on the same network.',
      'Re-enable Wireless debugging on the phone, or plug in USB and run `adb tcpip 5555` once.',
      'The phone’s IP may have changed: update PHONE_ADB_ADDR.',
    ],
  },
  {
    match: /no devices|device .*not found|device offline|\boffline\b/i,
    title: 'Phone not connected',
    detail: 'No phone is reachable over USB (and Wi-Fi, if configured). rhcsd checks again every 5 seconds.',
    fixes: [
      'Plug the phone in with a data-capable USB cable.',
      'Turn on Developer options → USB debugging.',
      'For cable-free use, set PHONE_ADB_ADDR in .env (ADB over Wi-Fi).',
    ],
  },
  {
    match: /timed out/i,
    title: 'The phone stopped responding',
    detail: 'adb didn’t answer within 10 seconds. This happens when the USB connection blips or the phone is busy.',
    fixes: [
      'Check the cable and try another USB port.',
      'Unlock the phone once.',
      'If it keeps happening, set the phone’s Default USB configuration to “USB tethering” or “File transfer” so it doesn’t switch modes.',
    ],
  },
];

export function explainError(source: ErrorSource, raw: string | null | undefined): Explained | null {
  if (!raw) return null;
  const rules = source === 'router' ? ROUTER : PHONE;
  const hit = rules.find((r) => r.match.test(raw));
  if (hit) return { title: hit.title, detail: hit.detail, fixes: hit.fixes, raw };
  return {
    title: source === 'router' ? 'Router problem' : 'Phone problem',
    detail: 'rhcsd reported an error it doesn’t have a plain-language explanation for yet.',
    fixes: ['Check the Log page for what happened just before.', 'Restarting rhcsd clears most temporary problems.'],
    raw,
  };
}
