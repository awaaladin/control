# RHCS: Rust Home Hardware Control System

A Rust toolkit for seeing, understanding and eventually controlling the hardware
in your home: router first, then the TV decoder and the amplifier.

Phase 1 (this code) is **reconnaissance**: everything is read-only.

## Setup (once)

Everything lives on the flash drive (`D:\setup`). C: is never written to.

```powershell
. D:\setup\rhcs\scripts\dev.ps1        # MinGW + cargo cache from D:
.\scripts\get-oui.ps1                  # optional: MAC → manufacturer names
cargo build --release
```

## Commands

| Command | What it does |
|---|---|
| `rhcs router` | Identifies the gateway you're connected to: MAC vendor, open admin services, banners, UPnP model, findings tagged OBSERVED / INFERRED / DOCUMENTED |
| `rhcs scan` | Finds every device on the LAN (ARP sweep, so it sees devices that ignore ping), names them, fingerprints services |
| `rhcs watch` | Live feed of devices joining/leaving the network |
| `rhcs upnp` | Lists everything that announces itself over UPnP. Routers, smart TVs, Android boxes and AV receivers often reveal their exact model here |

Add `--json` to any command for machine-readable output (this feeds the future API/dashboard).

Run with `cargo run --release -- router` or use `target\release\rhcs.exe` directly.

## Layout

```
crates/
  rhcs-core/   shared types: Device, MacAddr, UpnpInfo, Evidence levels
  netscan/     discovery engine: ARP sweep, ports, SSDP, DNS/mDNS, HTTP fingerprint, router recon
  rhcs-cli/    the `rhcs` binary
docs/          spec, roadmap, hardware findings log
scripts/       dev shell + data downloads
data/          IEEE OUI registry (downloaded, git-ignored)
```

No HTTP/DNS/TLS libraries: the HTTP client and DNS packets are hand-written and tiny, so the
same engine can later be cross-compiled into a binary of a few hundred KB that runs *on* the router.
