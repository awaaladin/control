# Roadmap

Evidence levels used everywhere: **TESTED · OBSERVED · DOCUMENTED · INFERRED · THEORY · UNSUPPORTED**.

## Phase 1: Router reconnaissance  ← we are here
- [x] Gateway/subnet detection (Windows IP Helper, Linux /proc)
- [x] ARP sweep host discovery, MAC vendor via IEEE registry, private-MAC detection
- [x] TCP service probe + banners (SSH/Telnet/FTP), HTTP fingerprint (server, title, auth realm)
- [x] UPnP/SSDP discovery with device-description parsing (manufacturer/model)
- [x] Hostnames via router DNS PTR + mDNS unicast
- [x] `watch`: join/leave events
- [ ] Run against the **real home router** (currently on a phone hotspot)
- [ ] Record router make/model/revision/firmware/CPU in docs/FINDINGS.md

## Phase 2: Get code onto the router
Depends on Phase 1 results:
- SSH available → `uname -m`, `/etc/os-release`, free RAM/flash → choose target
  (`mipsel-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `armv7-unknown-linux-musleabihf`)
- OpenWrt-supported model → (with explicit approval + recovery plan) flash OpenWrt
- ISP-locked → keep the router stock; run the daemon on a PC / Raspberry Pi / old Android phone instead
- Cross-compile `rhcsd` (daemon), deploy with scp, procd init script

## Phase 3: `rhcsd` daemon + API
- Axum REST + WebSocket: devices, events, latency/packet-loss, DNS health
- Persistent device inventory ("first seen", "last seen", naming)
- Network health history (latency, loss, DNS timing) as time series

## Phase 4: Dashboard
- Live network "radar" UI (Next.js or a ratatui terminal UI) fed by the WebSocket

## Phase 5: Decoder
- Identify model; check UPnP/DLNA, ADB (5555) if Android-based, HDMI-CEC
- ESP32 IR learner/blaster in Rust (esp-hal/Embassy): capture remote codes → command DB → `rhcs ir send power`

## Phase 6: Amplifier
- IR control via the same ESP32 blaster; HDMI-CEC/ARC if supported
- Audio-reactive: line-level tap → attenuator → ESP32 ADC → FFT → LED strip / dashboard spectrum
- Never connect a GPIO to a speaker output

## Ideas
- **Presence detection**: phone joins Wi-Fi → "welcome home" scene (TV on via IR, amp to input X)
- **Network intrusion alarm**: unknown MAC appears → flash LEDs / notify phone
- **DNS observatory**: which device talks to which domains (on the router, via dnsmasq logs)
- **"What's eating my bandwidth"** per-device usage, once running on the router
