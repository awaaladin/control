# control

**rhcs** stands for **Rust Home Hardware Control System**: tools for seeing, monitoring
and eventually controlling the hardware in a home (router, phone, TV decoder,
amplifier) from one place, written in Rust.

- **Using it?** Read the **[user manual](docs/MANUAL.md)**: opening the PULSE dashboard,
  every page and control, using it from your phone, and what every error message means.
- **Working on the code?** Read the **[complete guide](docs/GUIDE.md)**: every part of the
  codebase, setup, the API and a glossary of every abbreviation.

`rhcs` is a command-line tool for looking at and monitoring a home network:
it finds devices on the LAN, fingerprints what they run, and reads live LTE
signal data from ZTE 4G routers through their own web API.

It's written in Rust, has no runtime dependencies, and only reads. Apart from
logging in to the router, it doesn't change any settings.

## What it can do

- `rhcs scan`: list every device on your LAN with IP, MAC, vendor, hostname, open ports and UPnP model
- `rhcs router`: inspect your gateway: vendor, exposed services, web server, UPnP identity
- `rhcs watch`: print a line whenever a device joins or leaves the network
- `rhcs upnp`: list everything that announces itself over UPnP (routers, smart TVs, media boxes)
- `rhcs signal`: live LTE view for a ZTE router: RSRP, RSRQ, SINR, band, cell/tower, throughput,
  handover detection, with every reading logged to CSV

Every command accepts `--json`.

## Supported routers (`rhcs signal`)

| Router | Firmware | Status |
|---|---|---|
| ZTE MF293N (MTN Nigeria) | BD_MTNNGMF293NV1.0.0B06 | tested |
| Other ZTE MF-series with the `goform` web UI | | likely works, untested |

The other commands work on any network.

## Requirements

- Windows 10/11 or Linux (macOS builds, but gateway and ARP discovery aren't implemented there yet)
- Rust 1.80 or newer (`rustup`)
- On Windows, one of:
  - the **MSVC** toolchain (`stable-x86_64-pc-windows-msvc`) with Visual Studio Build Tools, or
  - the **GNU** toolchain plus [MinGW-w64](https://winlibs.com/) (msvcrt build) on your `PATH`.
    The GNU toolchain alone can't link the `windows-sys` crate because it's missing `dlltool`/`as`.

## Install

```sh
git clone https://github.com/awaaladin/control.git
cd control
cargo build --release
```

The binary ends up at `target/release/rhcs` (`rhcs.exe` on Windows).

Optional: download the IEEE MAC vendor list (~4 MB) so devices show a manufacturer name:

```powershell
.\scripts\get-oui.ps1
```

On Linux: `curl -L -o data/oui.csv https://standards-oui.ieee.org/oui/oui.csv`

If you use the GNU toolchain with MinGW installed somewhere other than your `PATH`,
edit the path in `scripts/dev.ps1` and dot-source it before building:

```powershell
. .\scripts\dev.ps1
```

## Configure

Only `rhcs signal` needs configuration. Copy the example file and fill in your router:

```sh
cp .env.example .env
```

```ini
ROUTER_IP=192.168.0.1
ROUTER_ADMIN_PASSWORD=your-router-password
ROUTER_MODEL=ZTE MF293N
```

`.env` is git-ignored. Keep it that way: it holds your router password, and
cell/tower IDs can be used to estimate your location. If your router still uses
the password printed on its label, change it in the router's web UI first.

`rhcs` looks for `.env` in the current directory and next to the executable.

## Usage

```sh
rhcs scan                      # scan your current subnet
rhcs scan --cidr 192.168.0.0/24 --no-ports
rhcs router
rhcs watch --interval 15
rhcs upnp --wait 5
rhcs signal                    # live view, refreshes every second
rhcs signal --once             # one reading
rhcs signal --json             # one reading as JSON
rhcs signal --interval 2 --no-log
```

`rhcs signal` writes readings to `logs/signal-YYYY-MM-DD.csv`:

```
time,rsrp,rsrq,sinr,rssi,band,earfcn,pci,cell_id,enb,sector,rx_bytes_s,tx_bytes_s,connected
```

### Reading the signal values

| Metric | Excellent | Good | Fair | Poor |
|---|---|---|---|---|
| RSRP (dBm) | ≥ −80 | −80 to −90 | −90 to −100 | < −100 |
| RSRQ (dB) | ≥ −10 | −10 to −15 | −15 to −20 | < −20 |
| SINR (dB) | ≥ 20 | 13 to 20 | 0 to 13 | < 0 |

`eNB` is the tower ID (`cell_id >> 8`) and `sector` is the antenna on that tower (`cell_id & 0xff`).
A change in either means the router handed over to another cell.

## Troubleshooting

**Connecting to the router kills my internet.**
Windows sends internet traffic through whichever connection it ranks highest. If the
router has no data, that's a dead end. Mark the router connection as local-only (run
once as administrator) and internet keeps flowing through your other connection,
e.g. a phone over USB tethering, while the router stays reachable:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\lan-only.ps1 -Interface WiFi      # router over Wi-Fi
powershell -ExecutionPolicy Bypass -File scripts\lan-only.ps1 -Interface Ethernet  # router over cable
# undo: add -Undo
```

**`rhcs signal` shows "not logged in".**
Check `ROUTER_ADMIN_PASSWORD`. ZTE routers lock the login after 5 wrong attempts,
so `rhcs` won't even try once fewer than 3 attempts remain. Log in once from a
browser to reset the counter.

**Scan finds only a couple of devices.**
Phone hotspots and some routers isolate Wi-Fi clients from each other. Try from a
wired connection, or check the router's own client list.

**Builds are very slow.**
Building on a USB stick or FAT32 drive works, but it's slow. Set `CARGO_INCREMENTAL=0`
(already done by `scripts/dev.ps1`) or move `target/` to an internal drive with
`CARGO_TARGET_DIR`.

## How the ZTE login works

The router's web UI never sends the plain password. `rhcs` does the same thing:

1. `GET /goform/goform_get_cmd_process?isTest=false&cmd=LD` returns a one-time challenge `LD`
2. `POST /goform/goform_set_cmd_process` with
   `isTest=false&goformId=LOGIN&password=UPPER(SHA256(UPPER(SHA256(password)) + LD))`
3. The router answers `{"result":"0"}` and sets a `zsidn` session cookie

All requests need a `Referer: http://<router>/index.html` header. See `crates/zte`.

## Project layout

```
crates/
  rhcs-core/   shared types (Device, MacAddr, UpnpInfo, ...)
  netscan/     discovery: ARP sweep, port probe, SSDP/UPnP, DNS/mDNS names, minimal HTTP client
  zte/         ZTE router API client: login, telemetry, signal grading
  rhcs-cli/    the `rhcs` binary
docs/          hardware notes and roadmap
scripts/       Windows helper scripts
```

The HTTP client, DNS packets and SHA-256 are written by hand, with no TLS or HTTP
crates, so release builds stay small enough to run on embedded Linux later.

## Development

```sh
cargo test
cargo run -- scan
```

Tests that use real hardware values read them from your local `.env` and skip those
checks when it's missing.

## Roadmap

- `rhcsd`: background service with an HTTP + WebSocket API (live telemetry, history, devices)
- Web dashboard and Android companion app on top of that API
- Android phone as a data source over ADB (LTE signal, hotspot clients, battery)
- Combining multiple LTE links into one connection

## Contributing

Issues and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first:
contributions are accepted under a short agreement that keeps dual licensing possible.

## License

rhcs is dual-licensed:

- **Open source: [GNU AGPL-3.0](LICENSE).** Free to use, study, modify and share. If you
  distribute a modified version, or let people use it over a network (for example as a
  hosted service), you must publish your source code under the same license.
- **Commercial license.** If you want to build rhcs into a closed-source product or
  service without the AGPL's obligations, a commercial license is available. Open an
  issue titled "Commercial license" or contact the maintainer through GitHub.
