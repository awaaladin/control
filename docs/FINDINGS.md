# Hardware findings log

Facts about the actual hardware, each tagged with how we know it.

## Dev machine (2026-11-11)
- OBSERVED: Windows 10 Pro 19045, Intel Dual Band Wireless-AC 7260
- OBSERVED: Rust 1.99.1, `stable-x86_64-pc-windows-gnu` (default) and `-msvc` installed; no Visual Studio C++ tools
- OBSERVED: windows-sys 0.61+ needs `dlltool` + `as` on the GNU target, so the MinGW-w64 (msvcrt) toolchain is installed at `D:\setup\toolchains\mingw64`
- OBSERVED: C: has ~4 GB free → all code, builds and toolchains live on D: (FAT32 flash drive)

## Current network (2026-11-11)
- OBSERVED: connected over Wi-Fi to SSID "vivo Y", 802.11n, channel 1, WPA2-Personal
- OBSERVED: gateway 10.234.55.62, client 10.234.55.83
- OBSERVED: gateway MAC 3a:7d:67:d2:9d:3b is locally administered (randomized)
- OBSERVED: no answer on TCP 22, 23, 53, 80, 443, 8080, 8443, 161, 1900, 7547
- INFERRED: this is a phone hotspot (vivo phone), **not** the home router

## Home router: ZTE MF293N (MTN Nigeria branded)
- OBSERVED (label): ZTE MF293N, 4G LTE, 12 V 1 A, SIM slot, micro-USB, 1× WAN/LAN RJ45,
  1× RJ11 phone port, 2× SMA external-antenna connectors, admin site http://mtnbroadband.ng
- DOCUMENTED: 4G LTE Cat4, Wi-Fi 802.11b/g/n (speedguide.net)
- UNSUPPORTED (as far as we know): no OpenWrt port found for MF293N. Related ZTE models
  (MF286/MF287/MF289) have OpenWrt support; MF283V port is experimental
- THEORY: ZTE MF-series stock web UIs talk to `/goform/goform_get_cmd_process` (JSON) for
  signal (RSRP/RSRQ/SINR/band/cell), data counters, connected clients, SMS. If the MF293N
  does too, we can build a full telemetry tool with zero firmware modification
- OBSERVED: LAN IP 192.168.0.1, LAN MAC prefix B0:0A:D5 (ZTE Corporation, IEEE)
- OBSERVED: open TCP 53, 80, 443; closed 22 (SSH), 23 (Telnet). Server header `WebServer-Webs`
- OBSERVED: firmware BD_MTNNGMF293NV1.0.0B06, hardware MF293NHW1.0, web UI is a ZTE `cpe/MF283U` build
- OBSERVED: its HTTP responses are malformed enough that .NET rejects them; curl and our raw client accept them
- TESTED: JSON API `/goform/goform_get_cmd_process` works. Without login: modem state, operator,
  signal bars, PPP status, monthly bytes. Login (SHA-256 challenge with `LD`, see .env notes) gives us:
  `lte_rsrp lte_rsrq lte_snr lte_rssi lte_pci cell_id enodeb_id wan_active_band wan_active_channel
  rmcc rmnc lac_code realtime_rx/tx_thrpt realtime_rx/tx_bytes realtime_time hardware_version net_select`
- TESTED 2026-11-11 with an MTN SIM: LTE Band 7, EARFCN 2852, RSRP −83, RSRQ −7, SINR 26.0, RSSI −56,
  MCC/MNC 621/30. Cell ID / eNB / TAC recorded locally only: tower IDs reveal approximate location
- TESTED: the phone (vivo Y03, over ADB, no root) camps on the **same eNB** as the router but on
  Band 3 (EARFCN 1650) and a different sector: router B7 vs phone B3 on one tower
- OBSERVED: web UI module `network/net_select` can set the bearer preference (auto / LTE-only) and
  manually scan for and register on networks. No band-lock UI found yet
- OBSERVED: the router reported `modem_sim_undetected` until a SIM was inserted
- Windows: Ethernet metric set to 500 (scripts/lan-only.ps1) so internet stays on Wi-Fi
- Secrets (Wi-Fi key, IMEI, S/N, SIM number) are on the label: never commit them

## Decoder: Strong SRT 4940
- OBSERVED (front panel): DVB-S2 MPEG-4 HD satellite receiver, Dolby Digital Plus,
  DVR-ready (USB), DiSEqC 1.2, HDMI out, AC 90-250 V 30 W
- DOCUMENTED (spec sheet, unverified for this unit): USB 2.0 host (media playback,
  firmware update, recording), Ethernet RJ-45 used for software updates
- UNSUPPORTED (unknown): no evidence of RS-232, open SDK or HDMI-CEC
- THEORY: primary control path is its IR remote (Strong remotes commonly use NEC protocol)
- OBSERVED 2026-11-11: has an "IP Settings" menu with DHCP; on DHCP it gets a lease from the
  router. MAC prefix 00:22:D3 = Hub-Tech (the OEM behind this Strong model). Router lists it in
  `lan_station_list` with hostname "host"
- OBSERVED: from the laptop (same LAN, router client isolation `NoForwarding=0`) it answers no
  ARP, no ping, and has no open TCP port in 1-10000: it runs no network services
- ~~THEORY: it is a network media client~~ → disproved, see below
- TESTED 2026-11-12 (decoder cabled straight to the laptop, `scripts/decoder-lab.ps1` as its
  DHCP/DNS server, `scripts/capture-decoder.ps1` recording its packets with pktmon):
  - DHCP: sends DISCOVER every 1-3 s while "DHCP Usage" is ON, hostname "host", requests only
    options 1,3,6,15 (mask, router, DNS, domain). Accepts a lease and keeps it
  - With an address it answers ARP but not ping; runs no services
  - **SCAN, View List and Firmware Update send no packets at all**: no DNS, no UPnP/SSDP, no
    connection attempts. "View List" only toggles DHCP. Firmware Update reports "no new
    software" without touching the network (likely USB or over-the-air via satellite)
  - With DHCP OFF the menu hides SCAN/Save, so a static address can't be used
  - Conclusion: the Ethernet port provides no control or monitoring path. **Control = infrared only**
- Lab gotchas (Windows): a directly cabled device makes an "Unidentified network" (Public
  profile) where an old Block rule for PowerShell dropped its DHCP; fixed by setting that
  network to Private. DHCP replies must be sent from a socket bound to the lab address, and
  within ~2 s, or the client never hears them

## Phone: vivo Y03 (V2333 / V2332), Android 16, MediaTek MT6769, arm64-v8a
- OBSERVED: no IR emitter (no `android.hardware.consumerir`), no magnetometer/compass
- OBSERVED: sensors: accelerometer, light, proximity, step counter, significant motion; GPS, mic, BLE, USB host
- TESTED (ADB shell, no root): `dumpsys telephony.registry` → LTE rsrp/rsrq/rssnr/cell/band;
  `dumpsys tethering` + `ip neigh` → hotspot clients (IP + MAC); `dumpsys battery`
- THEORY: `cmd wifi` soft-AP controls; untested (would drop the laptop's connection)

## Amplifier
- _pending: brand + model_
