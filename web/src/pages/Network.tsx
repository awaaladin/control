import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { NotifyToggle } from '../components/NotifyToggle';
import { Constellation, deviceKey, deviceName, KIND_LABEL, kindOf, type Kind } from '../components/Constellation';
import { Badge, Empty, Icon, KV, Panel, Segmented } from '../components/ui';
import { fmt, fmtAgo, fmtDateTime, str } from '../lib/format';
import { api } from '../lib/rhcs';
import { confirmAction, toast, useStore } from '../lib/store';
import { useNow } from '../lib/ticker';
import type { DeviceView } from '../lib/types';

export async function runScan(): Promise<void> {
  const ok = await confirmAction({
    title: 'Scan the LAN now?',
    body: 'rhcsd will sweep every configured network (ARP, ping, a few common ports, UPnP). It takes a little while and causes some local traffic.',
    confirmLabel: 'Scan now',
  });
  if (!ok) return;
  const r = await api.scan();
  if (r.ok) toast('ok', 'Scan started', r.message);
  else if (r.status !== 401) toast('error', 'Scan failed', r.message);
}

type SortKey = 'name' | 'ip' | 'kind' | 'vendor' | 'rtt' | 'ports' | 'last_seen';

function ipNum(ip: string): number {
  const p = ip.split('.').map(Number);
  return p.length === 4 && p.every((x) => Number.isFinite(x)) ? ((p[0] * 256 + p[1]) * 256 + p[2]) * 256 + p[3] : 0;
}

function DeviceDrawer({ d, onClose }: { d: DeviceView | null; onClose: () => void }) {
  const closeRef = useRef<HTMLButtonElement>(null);
  const now = useNow(5000);
  useEffect(() => {
    if (!d) return;
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [d, onClose]);
  if (!d) return null;
  const k = kindOf(d);
  return (
    <>
      <div className="drawer-scrim" onClick={onClose} aria-hidden="true" />
      <aside className="drawer" role="dialog" aria-modal="true" aria-label={`Device ${deviceName(d)}`}>
        <header className="drawer__head">
          <div>
            <div className="panel__kicker">{KIND_LABEL[k]}</div>
            <h2 className="drawer__title">{deviceName(d)}</h2>
            <div className="num muted">{d.ip}</div>
          </div>
          <button ref={closeRef} type="button" className="iconbtn" onClick={onClose} aria-label="Close details">
            <Icon name="close" />
          </button>
        </header>
        <dl className="kvs">
          <KV k="Network" v={str(d.network)} />
          <KV k="Label" v={str(d.label)} mono={false} />
          <KV k="Hostname" v={str(d.hostname)} />
          <KV k="IP" v={str(d.ip)} />
          <KV k="MAC" v={str(d.mac)} />
          <KV k="Vendor" v={str(d.vendor)} mono={false} />
          <KV k="RTT" v={d.rtt_ms != null ? `${fmt(d.rtt_ms, 1)} ms` : '—'} />
          <KV k="Gateway" v={d.is_gateway ? 'yes' : 'no'} />
          <KV k="This PC" v={d.is_self ? 'yes' : 'no'} />
          <KV k="First seen" v={`${fmtDateTime(d.first_seen)} · ${fmtAgo(d.first_seen, now)}`} />
          <KV k="Last seen" v={`${fmtDateTime(d.last_seen)} · ${fmtAgo(d.last_seen, now)}`} />
        </dl>
        <h3 className="drawer__sub">Open ports</h3>
        {d.ports && d.ports.length ? (
          <ul className="ports">
            {d.ports.map((p) => (
              <li key={p.port} className="port">
                <span className="port__n num">{p.port}</span>
                <span className="port__label">{str(p.label)}</span>
                {p.banner ? <code className="port__banner">{p.banner}</code> : null}
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted small">None found.</p>
        )}
        <h3 className="drawer__sub">UPnP identity</h3>
        {d.upnp && d.upnp.length ? (
          d.upnp.map((u, i) => (
            <dl key={`${u.location}-${i}`} className="kvs upnp">
              <KV k="Friendly name" v={str(u.friendly_name)} mono={false} />
              <KV k="Manufacturer" v={str(u.manufacturer)} mono={false} />
              <KV k="Model" v={[u.model_name, u.model_number].filter(Boolean).join(' ') || '—'} />
              <KV k="Description" v={str(u.model_description)} mono={false} />
              <KV k="Device type" v={str(u.device_type)} />
              <KV k="Server" v={str(u.server)} />
              <KV k="Location" v={str(u.location)} />
            </dl>
          ))
        ) : (
          <p className="muted small">No UPnP description.</p>
        )}
      </aside>
    </>
  );
}

function DeviceTable({ devices, onSelect }: { devices: DeviceView[]; onSelect: (d: DeviceView) => void }) {
  const [q, setQ] = useState('');
  const [sort, setSort] = useState<{ key: SortKey; dir: 1 | -1 }>({ key: 'ip', dir: 1 });
  const now = useNow(10000);
  const rows = useMemo(() => {
    const s = q.trim().toLowerCase();
    const f = s
      ? devices.filter((d) =>
          [d.label, d.hostname, d.vendor, d.ip, d.mac, d.network, KIND_LABEL[kindOf(d)], ...(d.ports ?? []).map((p) => `${p.port} ${p.label ?? ''}`)]
            .filter(Boolean)
            .join(' ')
            .toLowerCase()
            .includes(s),
        )
      : devices.slice();
    const val = (d: DeviceView): string | number => {
      switch (sort.key) {
        case 'name':
          return deviceName(d).toLowerCase();
        case 'ip':
          return ipNum(d.ip);
        case 'kind':
          return KIND_LABEL[kindOf(d)];
        case 'vendor':
          return (d.vendor ?? '~').toLowerCase();
        case 'rtt':
          return d.rtt_ms ?? Infinity;
        case 'ports':
          return d.ports?.length ?? 0;
        case 'last_seen':
          return d.last_seen ?? 0;
      }
    };
    f.sort((a, b) => {
      const x = val(a);
      const y = val(b);
      return (x < y ? -1 : x > y ? 1 : 0) * sort.dir;
    });
    return f;
  }, [devices, q, sort]);

  const th = (key: SortKey, label: string) => (
    <th scope="col" aria-sort={sort.key === key ? (sort.dir === 1 ? 'ascending' : 'descending') : 'none'}>
      <button type="button" className="th-btn" onClick={() => setSort((s) => ({ key, dir: s.key === key ? (s.dir === 1 ? -1 : 1) : 1 }))}>
        {label}
        <span aria-hidden="true">{sort.key === key ? (sort.dir === 1 ? ' ↑' : ' ↓') : ''}</span>
      </button>
    </th>
  );

  return (
    <Panel
      title="All devices"
      kicker={`${rows.length} of ${devices.length}`}
      actions={
        <input className="input input--search" type="search" placeholder="Search name, vendor, IP, MAC, port…" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search devices" />
      }
    >
      <div className="table-wrap">
        <table className="table table--hover">
          <thead>
            <tr>
              {th('name', 'Name')}
              {th('kind', 'Kind')}
              {th('ip', 'IP')}
              <th scope="col">MAC</th>
              {th('vendor', 'Vendor')}
              {th('rtt', 'RTT')}
              {th('ports', 'Ports')}
              {th('last_seen', 'Last seen')}
              <th scope="col">Network</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((d) => (
              <tr key={deviceKey(d)} tabIndex={0} onClick={() => onSelect(d)} onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), onSelect(d))}>
                <td>
                  <span className={`kdot kdot--${kindOf(d)}`} aria-hidden="true" />
                  {deviceName(d)}
                </td>
                <td>{KIND_LABEL[kindOf(d)]}</td>
                <td className="num">{d.ip}</td>
                <td className="num">{str(d.mac)}</td>
                <td>{str(d.vendor)}</td>
                <td className="num">{d.rtt_ms != null ? `${fmt(d.rtt_ms, 1)} ms` : '—'}</td>
                <td className="num">{d.ports?.length ? d.ports.map((p) => p.port).join(', ') : '—'}</td>
                <td className="num">{fmtAgo(d.last_seen, now)}</td>
                <td className="num small muted">{d.network}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Panel>
  );
}

export function NetworkPage() {
  const devices = useStore((s) => s.devices);
  const scan = useStore((s) => s.sources?.scan);
  const [mode, setMode] = useState<'map' | 'table'>('map');
  const [sel, setSel] = useState<string | null>(null);
  const closeDrawer = useCallback(() => setSel(null), []);
  const now = useNow(5000);

  const groups = useMemo(() => {
    const m = new Map<string, DeviceView[]>();
    for (const d of devices) {
      const k = d.network || 'unknown network';
      if (!m.has(k)) m.set(k, []);
      m.get(k)!.push(d);
    }
    for (const n of scan?.networks ?? []) if (!m.has(n)) m.set(n, []);
    return [...m.entries()];
  }, [devices, scan?.networks]);

  const selected = sel ? devices.find((d) => deviceKey(d) === sel) ?? null : null;
  const counts = useMemo(() => {
    const c: Record<Kind, number> = { gateway: 0, self: 0, phone: 0, unknown: 0 };
    for (const d of devices) c[kindOf(d)]++;
    return c;
  }, [devices]);

  return (
    <div className="page page--network">
      <header className="pagehead">
        <div>
          <div className="pagehead__kicker">
            {devices.length} devices · {groups.length} network{groups.length === 1 ? '' : 's'} · last scan {fmtAgo(scan?.last_run, now)}
          </div>
          <h1 className="pagehead__title">Network</h1>
        </div>
        <div className="pagehead__actions">
          <NotifyToggle />
          <Segmented
            label="View"
            value={mode}
            onChange={setMode}
            options={[
              { value: 'map', label: 'Constellation' },
              { value: 'table', label: 'Table' },
            ]}
          />
          <button type="button" className="btn btn--accent" onClick={() => void runScan()} disabled={!!scan?.running}>
            <Icon name="scan" size={18} className={scan?.running ? 'spin' : ''} /> {scan?.running ? 'Scanning…' : 'Scan now'}
          </button>
        </div>
      </header>

      <div className="legend" aria-label="Legend">
        {(Object.keys(KIND_LABEL) as Kind[]).map((k) => (
          <span key={k} className="legend__item">
            <span className={`kdot kdot--${k}`} aria-hidden="true" />
            {KIND_LABEL[k]} <span className="num muted">{counts[k]}</span>
          </span>
        ))}
        <span className="legend__item muted">size = open ports</span>
      </div>

      {devices.length === 0 && groups.length === 0 ? (
        <Panel>
          <Empty title={scan?.running ? 'Scanning…' : 'No devices yet'}>
            The LAN scanner reports after each sweep (every few minutes by default). Use “Scan now” to run one immediately.
          </Empty>
        </Panel>
      ) : mode === 'map' ? (
        <div className="constellations">
          {groups.map(([net, devs]) => (
            <Panel key={net} kicker={`${devs.length} device${devs.length === 1 ? '' : 's'}`} title={<span className="num">{net}</span>} actions={scan?.running ? <Badge tone="accent" pulse>scanning</Badge> : undefined}>
              <Constellation network={net} devices={devs} onSelect={(d) => setSel(deviceKey(d))} selected={sel} />
            </Panel>
          ))}
        </div>
      ) : (
        <DeviceTable devices={devices} onSelect={(d) => setSel(deviceKey(d))} />
      )}

      <DeviceDrawer d={selected} onClose={closeDrawer} />
    </div>
  );
}
