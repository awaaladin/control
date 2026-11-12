# rhcs: the complete guide

This guide explains the whole codebase from start to finish: what each part does,
how to set it up, how to use every command, and what every abbreviation means.
A full glossary is at the end ([jump to glossary](#glossary)).

---

## 1. What rhcs is

**rhcs = Rust Home Hardware Control System.**

It's a set of programs, written mostly in Rust, that let one computer see,
monitor and eventually control the hardware in a home:

| Hardware | What rhcs does with it today |
|---|---|
| **4G LTE router** (ZTE MF293N) | Logs in through the router's own web API and reads live signal, tower, band, data usage and connected devices |
| **Android phone** (vivo Y03) | Over ADB: reads the phone's own LTE signal, battery and the devices on its hotspot |
| **Your local network** | Finds every device, its manufacturer, name, open services and UPnP identity |
| **TV decoder** (Strong SRT 4940) | Network lab that watches what it does on the network (it has no remote-control interface over the network) |

Everything rhcs collects is served by a background program (`rhcsd`) to a web
dashboard (**PULSE**) and, later, an Android app.

```
  ZTE router ──HTTP──┐
  Phone ─────ADB─────┼──►  rhcsd  ──HTTP + WebSocket──►  PULSE (browser)
  LAN devices ─ARP───┘   (Rust, :7878)                 └► Kotlin app (planned)
```

---

## 2. What's in the repository

```
control/
├── Cargo.toml            Rust workspace: lists the crates and shared dependencies
├── Cargo.lock            exact versions of every dependency (for repeatable builds)
├── .env.example          template for your private settings (copy to .env)
├── .env                  YOUR private settings: passwords, IDs (never committed)
├── README.md             short install-and-use page
├── CONTRIBUTING.md       how to contribute + contributor agreement
├── LICENSE               AGPL-3.0 license text
├── crates/               the Rust code, split into crates (packages)
│   ├── rhcs-core/        shared building blocks
│   ├── netscan/          network discovery engine
│   ├── zte/              ZTE router client
│   ├── rhcs-cli/         the `rhcs` command-line program
│   └── rhcsd/            the `rhcsd` background service (the "daemon")
├── web/                  PULSE dashboard (TypeScript + React)
├── scripts/              Windows helper scripts (PowerShell)
├── docs/                 this guide, hardware findings, roadmap
├── data/                 downloaded data, e.g. the MAC vendor list (not committed)
└── logs/                 CSV signal logs and lab logs (not committed)
```

### 2.1 The crates (Rust packages)

A **crate** is a Rust package. A **workspace** groups several crates that build together.

| Crate | Kind | What it contains |
|---|---|---|
| `rhcs-core` | library | Shared types: `Device`, `MacAddr`, `UpnpInfo`, `Evidence`, and the `.env` loader |
| `netscan` | library | Everything about discovering a network: ARP sweep, port probing, UPnP/SSDP, DNS/mDNS names, a tiny HTTP client, multi-network scanning, gateway inspection |
| `zte` | library | Client for ZTE routers: login (SHA-256 challenge), telemetry fields, signal grading, its own SHA-256 implementation |
| `rhcs-cli` | program | The `rhcs` command you type in a terminal |
| `rhcsd` | program | The always-running service that polls router, phone and LAN and serves the API + dashboard |

#### `rhcs-core` files
| File | Purpose |
|---|---|
| `src/lib.rs` | `MacAddr` (parsing, "is this a randomized MAC?"), `OpenPort`, `UpnpInfo`, `Device`, `Evidence` levels, `Finding` |
| `src/env.rs` | Reads `.env` (current folder, or next to the program) and lets real environment variables override it |

#### `netscan` files
| File | Purpose |
|---|---|
| `net.rs` | Finds your IP, default gateway and subnet. `Subnet` type (CIDR maths). `local_ip_for()` = "which of my addresses reaches this device?" |
| `discover.rs` | Host discovery. Windows: one ARP request per address (`SendARP`). Linux: TCP touch + kernel ARP table |
| `win.rs` | Small wrappers over the Windows IP Helper API (`GetBestRoute`, `SendARP`) |
| `ports.rs` | TCP port probe + banner grabbing (SSH/FTP/Telnet greetings, HTTP server/title) |
| `http.rs` | Minimal HTTP/1.0 client (GET/POST, headers, cookies). Tolerates broken embedded servers |
| `ssdp.rs` | UPnP/SSDP discovery and parsing of device description XML (manufacturer, model) |
| `dns.rs` | Hand-built DNS packets: reverse lookup (PTR) via the router's DNS and via mDNS |
| `oui.rs` | MAC prefix → manufacturer, from the official IEEE registry file |
| `scan.rs` | Full scan of one or several networks at once (used by `rhcsd`) |
| `router.rs` | Gateway inspection with findings tagged by how sure we are |

#### `zte` files
| File | Purpose |
|---|---|
| `lib.rs` | `ZteClient` (login, field reading, auto re-login, lockout guard), `Telemetry`, band → frequency table, signal grades |
| `sha256.rs` | SHA-256 written by hand, matching the router's own JavaScript (uppercase hex) |

#### `rhcs-cli` files
| File | Purpose |
|---|---|
| `main.rs` | Command definitions: `router`, `scan`, `watch`, `upnp`, `signal` |
| `signal.rs` | The live full-screen LTE view + CSV logging |
| `ui.rs` | Terminal colours, Wi-Fi link info from `netsh` |
| `env.rs` | Re-exports the `.env` loader from `rhcs-core` |

#### `rhcsd` files
| File | Purpose |
|---|---|
| `main.rs` | Starts the three pollers and the web server |
| `state.rs` | Shared state: settings from `.env`, latest readings, 24 h history, devices, events, WebSocket broadcast |
| `router_task.rs` | Polls the router every second; detects handovers; survives the router disappearing |
| `phone.rs` | Polls the phone over ADB every 5 s (USB first, then Wi-Fi); hotspot join/leave events |
| `scan_task.rs` | Scans every attached network every 5 min (or on demand); device join/leave events |
| `api.rs` | HTTP routes, WebSocket, permissions, serves the PULSE build |

### 2.2 The dashboard (`web/`)

**PULSE** is a single-page web app (TypeScript + React, built with Vite).

| Folder | Contents |
|---|---|
| `src/lib/` | `rhcs.ts` (API + WebSocket client), `store.ts` (app state), `types.ts` (data shapes matching the Rust API), `demo.ts` (clearly labelled demo data), chart and formatting helpers |
| `src/components/` | Glow-ring gauges, live canvas charts, network constellation, app shell |
| `src/pages/` | Signal, Aim mode, Phone, Network, Audio, Control, Log |
| `src/styles/` | Theme and layout CSS |
| `dist/` | The built site. `rhcsd` serves it at `http://localhost:7878` |

### 2.3 The scripts (`scripts/`)

| Script | Admin? | What it does |
|---|---|---|
| `dev.ps1` | no | Sets up a terminal for building: puts MinGW on PATH, disables incremental builds on FAT32. Use: `. .\scripts\dev.ps1` |
| `get-oui.ps1` | no | Downloads the IEEE MAC vendor list to `data/oui.csv` |
| `lan-only.ps1` | **yes** | Makes one network adapter "local only" so internet goes through another one. `-Interface WiFi` or `Ethernet`, `-Undo` to revert |
| `lab-ethernet.ps1` | **yes** | Gives the Ethernet port a fixed address (default `192.168.50.1`, or `-Ip ...`) for the decoder lab, and opens the firewall for it on Ethernet only. `-Undo` to revert |
| `decoder-lab.ps1` | no | Acts as DHCP + DNS server for one device on the Ethernet cable and logs everything it says. Only answers the MAC in `DECODER_MAC` |

---

## 3. Setting up (once)

### 3.1 Requirements
- Windows 10/11 (Linux works for most parts)
- Rust (installed with `rustup`)
- On Windows with the GNU Rust toolchain: **MinGW-w64** (here: `D:\setup\toolchains\mingw64`)
- Node.js + npm, only to rebuild PULSE
- Android platform-tools (`adb`), only for the phone features

### 3.2 Build
```powershell
cd D:\setup\rhcs
. .\scripts\dev.ps1          # once per terminal
cargo build --release        # programs end up in target\release\
.\scripts\get-oui.ps1        # optional: manufacturer names
```
Building on the USB flash drive is slow (a full build takes 15-30 min); later
builds only recompile what changed.

### 3.3 Configure `.env`
Copy `.env.example` to `.env` and fill it in. Everything private lives here.

| Key | Meaning |
|---|---|
| `ROUTER_IP` | Router address, e.g. `192.168.0.1` |
| `ROUTER_ADMIN_PASSWORD` | Router web admin password |
| `ROUTER_MODEL` | Name shown in the UI |
| `ROUTER_WIFI_SSID` / `ROUTER_WIFI_KEY` | Router Wi-Fi name and password |
| `ROUTER_IMEI`, `ROUTER_SERIAL`, `ROUTER_SIM_NUMBER` | Label details, kept for reference |
| `ROUTER_CELL_ID`, `ROUTER_ENB`, `ROUTER_SECTOR`, `ROUTER_PCI`, `ROUTER_TAC` | Real tower IDs, used by tests |
| `ROUTER2_...` keys | Same settings for the second router |
| `PHONE_ADB_SERIAL` | Phone's USB ADB serial (from `adb devices`) |
| `PHONE_ADB_ADDR` | Phone's ADB-over-Wi-Fi address `ip:port` (optional) |
| `PHONE_CELL_ID`, `PHONE_ENB`, ... | The phone's tower IDs, used by tests |
| `LAPTOP_WIFI_MAC`, `HOTSPOT_KNOWN_GIONEE_MAC` | Any key whose name ends in `_MAC` gives that device a friendly name in the UI |
| `DECODER_IP`, `DECODER_MAC` | Decoder address and MAC (the lab only answers this MAC) |
| `RHCSD_BIND` | Where `rhcsd` listens (default `0.0.0.0:7878`) |
| `RHCSD_TOKEN` | Password for actions from other devices (optional) |
| `SCAN_CIDRS` | Extra networks to scan, e.g. `192.168.30.0/24` |
| `SCAN_INTERVAL_S` | Seconds between automatic scans (default 300) |
| `ADB_PATH` | Path to `adb.exe` if it isn't found automatically |

**Never commit `.env`.** It's in `.gitignore`. Tower IDs reveal your approximate location.

---

## 4. Using the command line (`rhcs`)

| Command | What it shows |
|---|---|
| `rhcs scan` | Every device on your current network: IP, MAC, vendor, name, open ports, UPnP model |
| `rhcs scan --cidr 192.168.0.0/24` | Scan a specific network |
| `rhcs scan --no-ports` | Faster scan without port probing |
| `rhcs router` | What your gateway is and what it exposes, with findings |
| `rhcs watch --interval 15` | Prints a line every time a device joins or leaves |
| `rhcs upnp --wait 5` | Everything that announces itself over UPnP |
| `rhcs signal` | Live LTE view of the ZTE router (1 reading/s), logged to `logs/signal-<date>.csv` |
| `rhcs signal --once` | One reading |
| `rhcs signal --no-log` | Don't write the CSV |

Add `--json` to any command for machine-readable output.

### Findings labels (`rhcs router`)
| Label | Meaning |
|---|---|
| TESTED | Exercised end to end on the real device |
| OBSERVED | Measured directly just now |
| DOCUMENTED | The manufacturer or project says so |
| INFERRED | A reasoned conclusion from observations |
| THEORY | Possible, not checked |
| UNSUPPORTED | Checked and not available |

---

## 5. Running the service (`rhcsd`) and the dashboard (PULSE)

```powershell
cd D:\setup\rhcs
.\target\release\rhcsd.exe
```
Then open **http://localhost:7878** for PULSE.

`rhcsd` keeps running even if the router or phone disappears; each part
reconnects on its own. It prints events as they happen:
```
[phone_online] phone connected over usb: Y03 (V2332)
[router_online] router 192.168.0.1 reachable
[handover] PCI 55→102 · eNB …
[device_join] laptop wifi joined 10.150.40.0/24 (10.150.40.57)
```

### 5.1 PULSE pages
| Page | What's on it |
|---|---|
| **Signal** | Glow-ring gauges (RSRP, SINR, RSRQ), live charts, throughput, tower card, handover markers. **Aim mode**: full-screen SINR with an optional rising tone for positioning the router |
| **Phone** | Phone model, battery, its own LTE signal compared with the router, hotspot clients |
| **Network** | Devices as an animated constellation per network, details on click, "Scan now" |
| **Audio** | Spectrum, oscilloscope, spectrogram from the browser microphone |
| **Control** | Router re-login, refresh interval, scan, settings, source health |
| **Log** | All events, filters, CSV export |

If the backend can't be reached, PULSE can show **demo data**, always under a
large amber **DEMO** badge, never mixed with live data.

### 5.2 Permissions
- Reading data: allowed from any device on your network.
- Changing things (scan now, re-login, settings): allowed from the laptop itself,
  or from anywhere if the request carries `RHCSD_TOKEN` in the `x-rhcs-token` header.

### 5.3 The API (for PULSE, the Kotlin app, or your own scripts)
| Method + path | Returns |
|---|---|
| `GET /api/health` | Version, uptime, capabilities, status of router/phone/scanner |
| `GET /api/router/telemetry` | Latest router reading |
| `GET /api/signal/history?minutes=60` | Router readings for the last N minutes (max 1440) |
| `GET /api/devices` | Devices found on all networks |
| `GET /api/phone` | Latest phone reading |
| `GET /api/events` | Recent events (handover, join/leave, errors…) |
| `POST /api/scan` | Start a scan now |
| `POST /api/router/action` `{"action":"relogin"}` | Reset the router session (other actions return "not implemented yet") |
| `GET/POST /api/settings` | Router polling interval |
| `GET /ws` | WebSocket: snapshot on connect, then live `status`, `telemetry`, `phone`, `devices`, `event` messages |

---

## 6. Network setups that work

The laptop can be on several networks at once. Windows sends internet traffic
through the connection with the **lowest metric** (a priority number).

**Recipe A: router over Wi-Fi, internet from the phone over USB**
1. Phone: turn on **USB tethering** (or set "Developer options → Default USB configuration → USB tethering" so it stays on).
2. Admin, once: `scripts\lan-only.ps1 -Interface WiFi`
3. Connect the laptop's Wi-Fi to the router.

**Recipe B: router over the Ethernet cable, internet over Wi-Fi**
1. Admin, once: `scripts\lan-only.ps1 -Interface Ethernet`
2. Plug the cable into the router.

**Recipe C: decoder lab (decoder cabled straight to the laptop)**
1. Admin: `scripts\lab-ethernet.ps1 -Ip 192.168.30.1`
2. Run `scripts\decoder-lab.ps1 -ServerIp 192.168.30.1 -ClientIp 192.168.30.10`
3. On the decoder: DHCP off, IP `192.168.030.010`, mask `192.000.000.000`, gateway and DNS `192.168.030.001`
4. Use the decoder's SCAN / View List / Firmware Update (check only) and read `logs/decoder-lab-<date>.log`
5. Afterwards (admin): `scripts\lab-ethernet.ps1 -Undo`

---

## 7. How the ZTE router login works

The router's web page never sends the plain password, and neither does rhcs:
1. Ask the router for a one-time challenge: `GET /goform/goform_get_cmd_process?isTest=false&cmd=LD`
2. Send `UPPER(SHA256( UPPER(SHA256(password)) + LD ))` with `goformId=LOGIN`
3. The router answers `{"result":"0"}` and sets the session cookie `zsidn`
4. Every request carries `Referer: http://<router>/index.html`

Safety: after 5 wrong passwords the router locks the login. rhcs won't even try
when fewer than 3 attempts remain, and stops retrying after a failed login.

---

## 8. Troubleshooting

| Problem | Fix |
|---|---|
| Plugging into the router kills internet | Make that connection local-only (`lan-only.ps1`) |
| Phone's USB internet keeps dropping | Phone: Default USB configuration → USB tethering; try another USB port/cable |
| Laptop won't join the router's Wi-Fi ("key exchange timed out") | Weak or unstable signal: move closer, retry |
| `rhcs signal` says "not logged in" | Check `ROUTER_ADMIN_PASSWORD`; log in once in a browser to reset the attempt counter |
| Build fails with "device does not exist" | The flash drive disconnected; reconnect, rebuild |
| Build fails on `windows-sys` with `dlltool` | Run `. .\scripts\dev.ps1` first (puts MinGW on PATH) |
| Decoder shows `000.000.000.000` | Use static settings (Recipe C), its DHCP is unreliable |

---

## 9. Licensing in one paragraph

rhcs is **dual-licensed**. Anyone may use, change and share it under the
**AGPL-3.0**: if they distribute a modified version or run it as an online
service, they must publish their source code. Companies that want to build it
into a closed product can buy a **commercial license** instead. Contributors
agree (see `CONTRIBUTING.md`) that their contributions can be included in both.

---

## Glossary

### Project names
| Term | Meaning |
|---|---|
| **rhcs** | **R**ust **H**ome **H**ardware **C**ontrol **S**ystem: the project, and the command-line program |
| **rhcsd** | rhcs **d**aemon: the background service (Unix convention: a trailing "d" means daemon) |
| **PULSE** | The web dashboard (a name, not an abbreviation) |
| **netscan** | **net**work **scan**ner crate |
| **cli** | **C**ommand-**L**ine **I**nterface |

### LTE / mobile network
| Term | Meaning |
|---|---|
| **LTE** | **L**ong-**T**erm **E**volution: the 4G mobile standard |
| **4G / 5G** | 4th / 5th **G**eneration mobile networks |
| **RSRP** | **R**eference **S**ignal **R**eceived **P**ower: signal strength in dBm. −80 or higher is excellent |
| **RSRQ** | **R**eference **S**ignal **R**eceived **Q**uality: signal quality in dB. −10 or higher is excellent |
| **SINR** | **S**ignal-to-**I**nterference-plus-**N**oise **R**atio: how clean the signal is, in dB. 20 or higher is excellent |
| **RSSNR** | **R**eference **S**ignal **S**ignal-to-**N**oise **R**atio: Android's name for SINR |
| **RSSI** | **R**eceived **S**ignal **S**trength **I**ndicator: total received power including noise |
| **dB / dBm** | **d**eci**B**el (a ratio) / decibels relative to 1 **m**illiwatt (a power level) |
| **eNB / eNodeB** | **e**volved **Node B**: an LTE base station (the tower's equipment). eNB ID = cell ID ÷ 256 |
| **Cell ID (CI)** | Identifier of one antenna/sector: eNB ID × 256 + sector |
| **Sector** | One direction-facing antenna on a tower (cell ID mod 256) |
| **PCI** | **P**hysical **C**ell **I**dentity: short radio ID (0–503) of a cell |
| **TAC** | **T**racking **A**rea **C**ode: a group of towers in one area |
| **LAC** | **L**ocation **A**rea **C**ode: the 2G/3G equivalent of TAC (ZTE reports TAC as `lac_code`) |
| **EARFCN** | **E**-UTRA **A**bsolute **R**adio **F**requency **C**hannel **N**umber: the exact LTE channel |
| **E-UTRA** | **E**volved **U**MTS **T**errestrial **R**adio **A**ccess: the formal name of LTE radio |
| **Band (B3, B7…)** | Frequency band. B3 = 1800 MHz, B7 = 2600 MHz, B20 = 800 MHz, B28 = 700 MHz |
| **MCC / MNC** | **M**obile **C**ountry **C**ode / **M**obile **N**etwork **C**ode. 621/30 = MTN Nigeria |
| **CA** | **C**arrier **A**ggregation: combining bands for more speed ("4G+") |
| **Handover** | The device switching from one cell/tower to another |
| **APN** | **A**ccess **P**oint **N**ame: the operator's data gateway setting |
| **PPP** | **P**oint-to-**P**oint **P**rotocol: the router's mobile data session (`ppp_connected`) |
| **SIM** | **S**ubscriber **I**dentity **M**odule: the SIM card |
| **IMEI** | **I**nternational **M**obile **E**quipment **I**dentity: the modem's unique number |
| **IMSI** | **I**nternational **M**obile **S**ubscriber **I**dentity: the SIM's unique number |
| **USSD** | **U**nstructured **S**upplementary **S**ervice **D**ata: the short codes you dial with star and hash to check balance or buy data |
| **CPE** | **C**ustomer **P**remises **E**quipment: a home router supplied by an operator |
| **ISP** | **I**nternet **S**ervice **P**rovider |
| **TR-069** | Remote management protocol ISPs use to control their routers |

### Networking
| Term | Meaning |
|---|---|
| **LAN / WAN** | **L**ocal / **W**ide **A**rea **N**etwork (your home network / the internet side) |
| **IP / IPv4 / IPv6** | **I**nternet **P**rotocol (version 4: `192.168.0.1`; version 6: longer addresses) |
| **CIDR** | **C**lassless **I**nter-**D**omain **R**outing: `192.168.0.0/24` notation (/24 = 256 addresses) |
| **Subnet / mask** | A range of addresses on one local network; the mask defines its size |
| **Gateway** | The device that forwards traffic off your network (normally the router) |
| **Metric** | Windows' priority number for routes; lowest wins internet traffic |
| **MAC** | **M**edia **A**ccess **C**ontrol address: a device's hardware address `aa:bb:cc:dd:ee:ff` |
| **OUI** | **O**rganizationally **U**nique **I**dentifier: first 3 bytes of a MAC, identifies the manufacturer |
| **IEEE** | **I**nstitute of **E**lectrical and **E**lectronics **E**ngineers: assigns OUIs, defines Wi-Fi/Ethernet |
| **Randomized / private MAC** | A MAC made up by the device for privacy (phones do this per Wi-Fi network) |
| **ARP** | **A**ddress **R**esolution **P**rotocol: "who has IP X? tell me your MAC". Used to find devices |
| **DHCP** | **D**ynamic **H**ost **C**onfiguration **P**rotocol: hands out IP addresses automatically |
| **APIPA** | **A**utomatic **P**rivate **IP** **A**ddressing: the `169.254.x.x` address Windows gives itself when DHCP fails |
| **DNS** | **D**omain **N**ame **S**ystem: turns names into IP addresses |
| **PTR** | **P**oin**t**e**r** record: reverse DNS (IP → name) |
| **mDNS** | **m**ulticast **DNS**: devices answering name questions themselves (`.local` names) |
| **NAT** | **N**etwork **A**ddress **T**ranslation: how a router shares one internet address |
| **TCP / UDP** | **T**ransmission **C**ontrol **P**rotocol (reliable connections) / **U**ser **D**atagram **P**rotocol (single packets) |
| **Port** | Number identifying a service on a device (80 = web, 22 = SSH, 53 = DNS) |
| **Banner** | The greeting a service sends, often revealing its software |
| **Multicast / broadcast** | Sending to a group / to everyone on the local network |
| **UPnP** | **U**niversal **P**lug a**n**d **P**lay: devices announcing themselves and their services |
| **SSDP** | **S**imple **S**ervice **D**iscovery **P**rotocol: the "search" part of UPnP (`M-SEARCH`) |
| **IGD** | **I**nternet **G**ateway **D**evice: the UPnP service that lets devices open router ports |
| **DLNA** | **D**igital **L**iving **N**etwork **A**lliance: media sharing over UPnP (play laptop videos on a TV) |
| **DIAL** | **DI**scovery **A**nd **L**aunch: protocol for casting apps to TVs |
| **SSID / BSSID** | Wi-Fi network name / the access point's MAC |
| **WPA2 / WPA3 / PSK** | **W**i-Fi **P**rotected **A**ccess v2/v3 / **P**re-**S**hared **K**ey (the Wi-Fi password) |
| **WPS** | **W**i-Fi **P**rotected **S**etup: connect by pressing a button on the router |
| **AP / client isolation** | Router setting that stops connected devices talking to each other (`NoForwarding` on ZTE) |
| **Tethering** | Sharing a phone's internet (Wi-Fi hotspot, USB, Bluetooth) |
| **RNDIS** | **R**emote **N**etwork **D**river **I**nterface **S**pecification: how USB tethering appears to Windows |
| **SSH / Telnet** | **S**ecure **Sh**ell / unencrypted remote terminal |
| **SMB / NetBIOS** | **S**erver **M**essage **B**lock / **Net**work **B**asic **I**nput/**O**utput **S**ystem: Windows file sharing and naming |
| **RTSP** | **R**eal **T**ime **S**treaming **P**rotocol: cameras and media streams |
| **MQTT** | **M**essage **Q**ueuing **T**elemetry **T**ransport: lightweight IoT messaging |
| **IoT** | **I**nternet **o**f **T**hings |

### Web and software
| Term | Meaning |
|---|---|
| **HTTP / HTTPS** | **H**yper**t**ext **T**ransfer **P**rotocol (**S**ecure = encrypted with TLS) |
| **TLS** | **T**ransport **L**ayer **S**ecurity: encryption for web traffic |
| **API** | **A**pplication **P**rogramming **I**nterface: how programs talk to each other |
| **REST** | **Re**presentational **S**tate **T**ransfer: API style using URLs + GET/POST |
| **JSON** | **J**ava**S**cript **O**bject **N**otation: the data format of the API |
| **WebSocket (WS)** | A connection that stays open so the server can push live data |
| **CORS** | **C**ross-**O**rigin **R**esource **S**haring: lets a page from one site call another |
| **PNA** | **P**rivate **N**etwork **A**ccess: browser rule for public sites calling `localhost`/LAN |
| **SPA** | **S**ingle-**P**age **A**pplication: a web app that loads once (PULSE) |
| **CSV** | **C**omma-**S**eparated **V**alues: spreadsheet-friendly log format |
| **XML** | e**X**tensible **M**arkup **L**anguage: format of UPnP device descriptions |
| **UI** | **U**ser **I**nterface |
| **SHA-256** | **S**ecure **H**ash **A**lgorithm, 256-bit: one-way fingerprint used for the router login |
| **LD** | The ZTE router's one-time login challenge value (field name; no official expansion) |
| **goform** | ZTE's web API endpoint prefix (`/goform/...`) |
| **zsidn** | ZTE's session cookie name |
| **Crate / workspace** | Rust package / group of packages built together |
| **Cargo** | Rust's build tool and package manager |
| **tokio** | Rust library for doing many network tasks at once (async runtime) |
| **axum** | Rust web server library used by `rhcsd` |
| **serde** | Rust library for converting data to/from JSON |
| **clap** | Rust library for command-line arguments |
| **React / Vite / TypeScript** | UI library / build tool / typed JavaScript, used by PULSE |
| **Zustand** | Small state-management library used by PULSE |
| **npm** | **N**ode **P**ackage **M**anager |
| **Daemon** | A program that runs in the background as a service |
| **Toolchain** | Compiler and tools for building programs |
| **MSVC / GNU / MinGW** | **M**icro**s**oft **V**isual **C**++ toolchain / GNU toolchain / **Min**imalist **G**NU for **W**indows |
| **dlltool / as** | MinGW tools that build Windows import libraries / assemble code |
| **musl** | Small C library used to build self-contained Linux programs (for routers later) |
| **ABI / arm64** | **A**pplication **B**inary **I**nterface / 64-bit ARM processors (phones) |
| **SoC / CPU** | **S**ystem **o**n a **C**hip / **C**entral **P**rocessing **U**nit |
| **FAT32 / NTFS** | Older USB-drive file system (no files before 1980, no hard links) / Windows' main file system |

### Android
| Term | Meaning |
|---|---|
| **ADB** | **A**ndroid **D**ebug **B**ridge: control a phone from a computer, over USB or Wi-Fi |
| **USB debugging / Wireless debugging** | Phone settings that allow ADB over cable / Wi-Fi |
| **Shizuku** | App that gives your own app ADB-level permissions without root (planned) |
| **Root** | Full admin access to Android (not used by rhcs) |
| **OTG** | **O**n-**T**he-**G**o: the phone acting as a USB host |
| **IR blaster** | Infrared transmitter that can act as a TV remote (the Y03 has none) |
| **Kotlin** | Programming language for the planned Android app |

### TV and audio
| Term | Meaning |
|---|---|
| **Decoder / STB** | Satellite receiver / **S**et-**T**op **B**ox |
| **DVB-S2** | **D**igital **V**ideo **B**roadcasting – **S**atellite, 2nd generation |
| **DiSEqC** | **Di**gital **S**atellite **Eq**uipment **C**ontrol: controls dish switches/motors |
| **DVR / PVR** | **D**igital / **P**ersonal **V**ideo **R**ecorder |
| **EPG** | **E**lectronic **P**rogram **G**uide |
| **HDMI** | **H**igh-**D**efinition **M**ultimedia **I**nterface |
| **HDMI-CEC** | **C**onsumer **E**lectronics **C**ontrol: devices controlling each other over HDMI |
| **ARC / eARC** | (enhanced) **A**udio **R**eturn **C**hannel over HDMI |
| **NEC** | A common infrared remote-control protocol |
| **OEM** | **O**riginal **E**quipment **M**anufacturer: the company that actually built a branded device (Hub-Tech for this Strong decoder) |
| **FFT** | **F**ast **F**ourier **T**ransform: turns sound into frequency bars (spectrum) |
| **RMS** | **R**oot **M**ean **S**quare: average loudness |

### Legal
| Term | Meaning |
|---|---|
| **AGPL-3.0** | GNU **A**ffero **G**eneral **P**ublic **L**icense v3: open source; network use counts as distribution |
| **MIT** | Very permissive license (rhcs used it before switching to AGPL + commercial) |
| **Dual licensing** | Same code offered under an open license and a paid commercial license |
| **CLA** | **C**ontributor **L**icense **A**greement: contributors' permission to relicense their code |
