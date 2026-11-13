import { useEffect, useRef } from 'react';
import L from 'leaflet';
import 'leaflet/dist/leaflet.css';
import { fmt, gradeColor, grade, isNum } from '../lib/format';
import type { HomeLocation, TowerCell } from '../lib/towers';

const avgSinr = (c: TowerCell) => c.router?.avg_sinr ?? c.phone?.avg_sinr ?? null;

/** OpenStreetMap with home, towers, bearing lines, coloured by observed SINR. */
export function TowerMap({ home, cells, bestKey }: { home: HomeLocation | null; cells: TowerCell[]; bestKey: string | null }) {
  const el = useRef<HTMLDivElement>(null);
  const map = useRef<L.Map | null>(null);
  const layer = useRef<L.LayerGroup | null>(null);
  const fitted = useRef('');

  useEffect(() => {
    if (!el.current || map.current) return;
    const m = L.map(el.current, { zoomControl: true, attributionControl: true, worldCopyJump: true });
    L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
      maxZoom: 19,
      attribution: '&copy; <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noreferrer">OpenStreetMap</a> contributors',
    }).addTo(m);
    m.setView([0, 0], 2);
    map.current = m;
    layer.current = L.layerGroup().addTo(m);
    return () => {
      m.remove();
      map.current = null;
      layer.current = null;
      fitted.current = '';
    };
  }, []);

  useEffect(() => {
    const m = map.current;
    const g = layer.current;
    if (!m || !g) return;
    g.clearLayers();
    const pts: L.LatLngExpression[] = [];
    const located = cells.filter((c) => c.location);

    for (const c of located) {
      const loc = c.location!;
      const s = avgSinr(c);
      const col = gradeColor(grade('sinr', s));
      const ll: L.LatLngExpression = [loc.lat, loc.lon];
      pts.push(ll);
      if (isNum(loc.range_m) && loc.range_m > 0 && loc.range_m < 20_000) {
        L.circle(ll, { radius: loc.range_m, color: col, weight: 1, opacity: 0.25, fillOpacity: 0.04, interactive: false }).addTo(g);
      }
      if (home) {
        L.polyline(
          [
            [home.lat, home.lon],
            [loc.lat, loc.lon],
          ],
          { color: col, weight: c.key === bestKey ? 3 : 1.5, opacity: c.serving.length ? 0.95 : 0.55, dashArray: c.serving.length ? undefined : '4 6' },
        ).addTo(g);
      }
      const label = `eNB ${c.enb} · sector ${c.sector}${c.pci !== null ? ` · PCI ${c.pci}` : ''}`;
      const lines = [
        `<b>${label}</b>`,
        c.serving.length ? `serving the ${c.serving.join(' and ')} now` : `seen by ${c.seen_by.join(' and ')}`,
        isNum(s) ? `average SINR ${fmt(s, 1)} dB` : 'no SINR yet',
        isNum(c.distance_m) ? `${fmt(c.distance_m / 1000, 2)} km, ${c.compass} (${fmt(c.bearing_deg)}°)` : '',
        isNum(loc.samples) ? `position from ${loc.samples} OpenCellID measurements` : '',
      ].filter(Boolean);
      L.circleMarker(ll, {
        radius: c.serving.length ? 9 : 6,
        color: col,
        weight: c.key === bestKey ? 3 : 2,
        fillColor: col,
        fillOpacity: 0.45,
      })
        .bindTooltip(lines.join('<br>'))
        .addTo(g);
    }

    if (home) {
      const ll: L.LatLngExpression = [home.lat, home.lon];
      pts.push(ll);
      L.circleMarker(ll, { radius: 8, color: '#ffffff', weight: 2, fillColor: 'hsl(168 85% 62%)', fillOpacity: 0.9 })
        .bindTooltip(`<b>Home</b><br>from ${home.source}`)
        .addTo(g);
    }

    // Fit once per set of points, so the user's own panning isn't undone every refresh.
    const sig = pts.map((p) => String(p)).join('|');
    if (pts.length && sig !== fitted.current) {
      fitted.current = sig;
      if (pts.length === 1) m.setView(pts[0], 15);
      else m.fitBounds(L.latLngBounds(pts), { padding: [36, 36], maxZoom: 16 });
    }
  }, [home, cells, bestKey]);

  return <div ref={el} className="towermap" role="region" aria-label="Map of home and towers" />;
}
