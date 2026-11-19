# PULSE for Android

PULSE on any 64-bit Android phone, **standalone**: the app carries its own copy of the engine (`rhcsd`) and the dashboard, reads the phone's own cell and signal data natively, and needs no laptop, no router and no Termux. It can still open another PULSE system (the laptop hub) with **Find / Scan** or a typed address / pairing link.

## How it works

```
MainActivity (WebView) --> http://127.0.0.1:7878/#hub=...&token=...   (the dashboard served by the engine)
EngineService (foreground) --> runs librhcsd.so with RHCSD_APP=1, restarts it with backoff, logs to filesDir
  SignalReporter  every ~2 s: TelephonyManager cell info of the data SIM -> POST /api/phone/push
                  LocationManager (network + gps, ~5 s / 10 m)          -> POST /api/location/push
  LookupWorker    every ~15 s: GET /api/towers/lookups -> ask beaconDB (and OpenCellID if a key is typed) -> POST /api/towers/location
```

- The engine is an aarch64 musl binary shipped as `jniLibs/arm64-v8a/librhcsd.so` (an executable named like a library is the allowed way to run code on Android 10+) and started from `applicationInfo.nativeLibraryDir`. The manifest sets `extractNativeLibs="true"` and Gradle `useLegacyPackaging = true` for this.
- PULSE's `web/dist` is bundled as assets (`pulse-web`) and unpacked to `filesDir/web` when the app version changes.
- A random 40-character token is made per install (SecureRandom, private preferences, never logged). In app mode the engine requires it for **every** `/api` and `/ws` call, even from this phone, because other apps can reach loopback.
- 32-bit phones: the app says the built-in engine is not supported; Find / Scan and remote systems still work.
- First start asks in plain words before running the engine (**Start** / **Not now**). The finder has an on/off switch and shows the engine state (starting / running / problem with the last log lines / "Stopped by you"). The foreground notification has **Stop** / **Start**. A stop from PULSE itself (`POST /api/engine/stop`) exits with code 0 and is not restarted.

## Privacy

Everything stays on the phone. The only data that leaves is the cell look-up: a cell's network codes and ID (never the phone's position) sent to beaconDB (`api.beacondb.net`) and, if you type an OpenCellID key in the finder, OpenCellID. The key is stored only in the app's private preferences.

## Permissions

Precise location and phone state (Android shows cell data only to apps that have them), notifications (the engine's notification), foreground service (location + data sync types).

## Phone Game Boost

Off by default; switched on from the Boost page inside the app. While a game is in front (found with Usage access: apps Android marks as games, or ones you tick) it measures Wi-Fi and cellular delay separately (2 tiny TCP connects per network every 15 s, about 260 bytes each, counted in the panel's "Boost cost" line; nothing is measured when no game is in front), suggests the faster one with a one-tap notification button, optionally silences notifications (Do Not Disturb access) and holds a best-effort Wi-Fi low-latency lock only while the game is in front (no CPU wake lock). Everything is restored when the game closes, the switch goes off, or the service restarts.

**Route my game over the best network (VPN, off by default).** `PulseVpnService` is a per-app `VpnService`: only the game's package is added with `addAllowedApplication`, every other app is untouched. `VpnForwarder` is a from-scratch IPv4 user-space forwarder: UDP flows and a simple TCP relay (SYN/ACK/FIN/RST, MSS 1360, client window respected, in-order only, no retransmit because the tunnel is local) re-originate each flow as a normal socket that is `protect()`ed and bound to the chosen `Network`. The network is picked at game start and only changes mid-game (new connections only) if the current one is dead or very slow (200 ms or more) for several rounds. Kill switch: if the game sends and nothing comes back for ~6 s, PULSE stops watching, or anything throws, the VPN stops and the game uses the normal network. It stops when the game closes, Boost goes off or the service stops. Needs Android's one-time VPN dialog (requested from `MainActivity` through the bridge). Not carried: IPv6, ICMP, IP fragments. Switching mid-match can drop the match (the server sees a new address). Untested on a phone.

**Privileged mode (optional, Shizuku).** With the Shizuku app installed, started (Wireless debugging) and PULSE allowed, `Privileged.kt` can run only a fixed allow-list through Shizuku: `svc wifi enable|disable`, `svc data enable|disable`, `cmd connectivity airplane-mode enable|disable`. The web page names an action (`request_permission`, `repick`, `mobile_only`, `wifi_back`), never a command. "Re-pick tower" = airplane mode on, 4 s, off: all connectivity drops for 5-10 s, so it is blocked while a game is open or was seen in the last 2 min, during a call, and needs a confirm tap. Optional auto mode (off by default): no game, on cellular, RSRP at or below -110 dBm for 60 s, at most once per 10 min. A Wi-Fi turn-off or airplane toggle is remembered and undone on game end, Boost off, service stop and next start. It cannot choose a tower: the modem does; PULSE can only ask it to register again. Shizuku API `dev.rikka.shizuku:api` and `provider` 13.1.5 (Maven Central).

It cannot force a cell tower or network mode, or make a signal stronger. Extra permissions: PACKAGE_USAGE_STATS, ACCESS_NOTIFICATION_POLICY, WAKE_LOCK, ACCESS_WIFI_STATE, CHANGE_WIFI_STATE, CHANGE_NETWORK_STATE. Untested on hardware at the time of writing.

## Build (GitHub Actions only)

The app is never built on a laptop. `.github/workflows/build-apk.yml` runs on pushes touching `android/`, `crates/`, `web/`, `Cargo.*` or `.cargo/`: it builds the engine (`cargo build --release --target aarch64-unknown-linux-musl -p rhcsd`) into `jniLibs`, builds PULSE (`npm ci && npm run build`) into `assets/pulse-web`, then runs Gradle and uploads the artifact `app-debug` (also published to the `android-latest` release). The two generated folders are git-ignored.

```sh
gh run list --workflow build-apk.yml --limit 1
gh run watch <run-id> --exit-status
gh run download <run-id> -n app-debug -D apk-output
```

Install: copy `apk-output/app-debug.apk` to the phone and open it (allow "install unknown apps" once), or over USB: `adb install -r apk-output/app-debug.apk`.

## Layout

- `Engine.kt`: token, paths, status, a small authenticated HTTP client
- `EngineService.kt`: foreground service, engine process, notification
- `SignalReporter.kt`: cell, battery and location readings pushed to the engine
- `LookupWorker.kt`: tower position look-ups
- `MainActivity.kt`, `Finder.kt`: WebView, consent / permissions, finder screen
- `res/xml/network_security_config.xml`: plain http is allowed (loopback and your own network)

Release builds need a signing key; none is stored in the repository.
