# PULSE (rhcs web dashboard)

Live control room for `rhcsd`: router LTE telemetry, the Android phone over ADB, and the LAN.
Vite + React 18 + TypeScript (strict), Zustand for state, canvas charts driven by `requestAnimationFrame`.

## Develop

```sh
cd web
npm install
npm run dev        # http://localhost:5173
```

## Build

```sh
npm run build      # type-checks (tsc --noEmit), then writes web/dist
npm run preview    # serve the built bundle locally
```

`rhcsd` serves `web/dist` at `/` automatically, so after a build open `http://localhost:7878`.

## Pointing at rhcsd

The backend URL is resolved in this order:

1. The URL saved in **Control → Settings** (stored in `localStorage`).
2. Same origin, if the page is served by rhcsd (port 7878, or `/api/health` answers on the same origin).
3. `http://localhost:7878`.

If rhcsd requires a token (`/api/health` → `auth_required: true`), enter it in Settings; it is sent as
`x-rhcs-token` on POST requests only. Without a token configured, rhcsd only accepts control actions
from its own machine.

If the backend is unreachable at startup, PULSE keeps retrying and offers **Use demo data**. Demo mode
(also a toggle in Settings) runs a local simulator with obviously fake identifiers (192.0.2.x, 00:00:5e:00:53:xx,
MCC/MNC 001/01) and shows an amber DEMO badge the whole time. Demo and live data are never mixed.

## Deploy to Vercel

Create a project with **Root Directory = `web`**. `vercel.json` sets the build (`npm run build`, output `dist`)
and rewrites every non-asset path to `index.html` for client-side routing. In the deployed app, set the
backend URL in Settings to the laptop's address. rhcsd enables CORS and Private Network Access, but browsers
block plain `http://` calls from an `https://` page unless the backend is reachable as `localhost`.
