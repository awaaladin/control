# PULSE for Android

The PULSE dashboard as an app, plus a **Find my system** button that scans the phone's networks
(its Wi-Fi, its own hotspot, USB tethering, and the phone itself) for your home system (`rhcsd`, port 7878)
and opens it.

- Loads PULSE straight from the home system, so the app always shows the latest PULSE; the APK rarely changes.
- Native, so it can open plain `http://` addresses on your own network (the hosted https website cannot).
- Remembers the last system and reconnects by itself; falls back to scanning when it moves.
- Accepts a typed address or a pairing link (`https://…/#hub=…&token=…`) for remote access.

## Build (GitHub Actions only)

The app is never built on a laptop. Every push that touches `android/` runs
`.github/workflows/build-apk.yml`, which builds a debug APK and uploads it as the artifact `app-debug`.

```sh
gh run list --workflow build-apk.yml --limit 1
gh run watch <run-id> --exit-status
gh run download <run-id> -n app-debug -D apk-output
```

Install: copy `apk-output/app-debug.apk` to the phone and open it (allow "install unknown apps" once),
or with the phone on USB: `adb install -r apk-output/app-debug.apk`.

## Layout

- `app/src/main/kotlin/app/rhcs/pulse/Finder.kt`: network scan and `/api/health` check
- `app/src/main/kotlin/app/rhcs/pulse/MainActivity.kt`: WebView, finder screen, permissions (location for Towers, microphone for Audio)
- `app/src/main/res/xml/network_security_config.xml`: allows plain http, because the home system is on your own network

Release builds need a signing key; none is stored in the repository.
