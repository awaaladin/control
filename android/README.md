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
