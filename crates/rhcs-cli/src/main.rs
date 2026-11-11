//! rhcs: see and understand the hardware on your own network.

mod env;
mod signal;
mod ui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use netscan::{discover, dns, net, oui::OuiDb, ports, router, ssdp};
use rhcs_core::{Device, MacAddr};
use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use ui::{bold, cyan, dim, green, red, yellow};

#[derive(Parser)]
#[command(name = "rhcs", version, about = "Rust Home Hardware Control System: network & device recon")]
struct Cli {
    /// Machine-readable JSON output
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Identify the router/gateway you are connected to and what it exposes
    Router,
    /// Discover every device on the local network
    Scan {
        /// Subnet to scan, e.g. 192.168.1.0/24 (default: your current subnet)
        #[arg(long)]
        cidr: Option<String>,
        /// Skip port probing (faster)
        #[arg(long)]
        no_ports: bool,
    },
    /// Keep watching the network and report devices joining or leaving
    Watch {
        /// Seconds between sweeps
        #[arg(long, default_value_t = 30)]
        interval: u64,
        #[arg(long)]
        cidr: Option<String>,
    },
    /// List UPnP devices (routers, TVs, decoders, speakers announce themselves here)
    Upnp {
        /// Seconds to listen for answers
        #[arg(long, default_value_t = 3)]
        wait: u64,
    },
    /// Live LTE signal view for the ZTE 4G router (RSRP/RSRQ/SINR, tower, band, throughput)
    Signal {
        /// Router address (default: ROUTER_IP from .env, else 192.168.0.1)
        #[arg(long)]
        host: Option<String>,
        /// Seconds between readings
        #[arg(long, default_value_t = 1.0)]
        interval: f64,
        /// Print one reading and exit
        #[arg(long)]
        once: bool,
        /// Don't write logs/signal-<date>.csv
        #[arg(long)]
        no_log: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    ui::init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Router => cmd_router(cli.json).await,
        Cmd::Scan { cidr, no_ports } => cmd_scan(cidr, no_ports, cli.json).await,
        Cmd::Watch { interval, cidr } => cmd_watch(cidr, interval).await,
        Cmd::Upnp { wait } => cmd_upnp(wait, cli.json).await,
        Cmd::Signal { host, interval, once, no_log } => signal::run(host, interval, once, cli.json, !no_log).await,
    }
}

fn target_subnet(info: &net::NetInfo, cidr: Option<String>) -> Result<net::Subnet> {
    if let Some(c) = cidr {
        return net::Subnet::parse(&c);
    }
    if info.subnet.prefix < 22 {
        let s = net::Subnet::new(info.local_ip, 24);
        eprintln!("{}", dim(&format!("subnet {} is large; scanning {} (use --cidr to override)", info.subnet, s)));
        return Ok(s);
    }
    Ok(info.subnet)
}

fn header(info: &net::NetInfo) {
    println!(
        "{} {}   {} {}   {} {}",
        dim("me"),
        bold(&info.local_ip.to_string()),
        dim("gateway"),
        bold(&info.gateway.map(|g| g.to_string()).unwrap_or("-".into())),
        dim("subnet"),
        bold(&info.subnet.to_string())
    );
    if let Some(link) = ui::wifi_link() {
        let get = |k: &str| link.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str()).unwrap_or("-");
        println!(
            "{} {}   {} {}   {} {}   {} {}",
            dim("wifi"),
            bold(get("SSID")),
            dim("signal"),
            get("Signal"),
            dim("radio"),
            get("Radio type"),
            dim("channel"),
            get("Channel")
        );
    }
    println!();
}

async fn cmd_router(json: bool) -> Result<()> {
    let info = net::detect()?;
    let oui = OuiDb::load();
    if !json {
        header(&info);
        eprintln!("{}", dim("probing gateway: ARP, ports, UPnP, reverse DNS ..."));
    }
    let report = router::recon(&info, &oui).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    print_device(&report.gateway, &oui);
    println!("\n{}", bold("Findings"));
    for f in &report.findings {
        let tag = format!("{:<11}", f.evidence.tag());
        let tag = match f.evidence {
            rhcs_core::Evidence::Observed | rhcs_core::Evidence::Tested => green(&tag),
            rhcs_core::Evidence::Inferred => yellow(&tag),
            _ => cyan(&tag),
        };
        println!("  {tag} {}", f.text);
    }
    Ok(())
}

async fn cmd_scan(cidr: Option<String>, no_ports: bool, json: bool) -> Result<()> {
    let info = net::detect()?;
    let subnet = target_subnet(&info, cidr)?;
    let oui = OuiDb::load();
    let hosts = subnet.hosts();
    if !json {
        header(&info);
        eprintln!("{}", dim(&format!("sweeping {} addresses on {} + listening for UPnP ...", hosts.len(), subnet)));
    }

    let (seen, hits) = tokio::join!(
        discover::sweep(&hosts, 64),
        ssdp::discover(info.local_ip, Duration::from_secs(3))
    );

    let mut devices: BTreeMap<u32, Device> = BTreeMap::new();
    for s in seen {
        let d = devices.entry(s.ip.into()).or_insert_with(|| Device::new(s.ip));
        d.mac = s.mac;
        d.rtt_ms = Some(s.rtt_ms);
    }
    devices.entry(info.local_ip.into()).or_insert_with(|| Device::new(info.local_ip)).is_self = true;
    if let Some(gw) = info.gateway.filter(|g| subnet.contains(*g)) {
        devices.entry(gw.into()).or_insert_with(|| Device::new(gw)).is_gateway = true;
    }
    for h in hits.unwrap_or_default() {
        if subnet.contains(h.ip) {
            let d = devices.entry(h.ip.into()).or_insert_with(|| Device::new(h.ip));
            if !d.upnp.iter().any(|u| u.identity() == h.info.identity() && u.friendly_name == h.info.friendly_name) {
                d.upnp.push(h.info);
            }
        }
    }

    // Enrich: hostnames + open ports, 16 devices at a time.
    let sem = Arc::new(Semaphore::new(16));
    let mut set = JoinSet::new();
    for (_, mut d) in std::mem::take(&mut devices) {
        let sem = sem.clone();
        let gw = info.gateway;
        set.spawn(async move {
            let _p = sem.acquire_owned().await;
            let probe_ports = async {
                if no_ports || d.is_self {
                    Vec::new()
                } else if d.is_gateway {
                    ports::probe(d.ip, &ports::router_ports(), Duration::from_millis(900)).await
                } else {
                    ports::probe(d.ip, ports::COMMON, Duration::from_millis(900)).await
                }
            };
            let (name, open) = tokio::join!(dns::reverse_lookup(d.ip, gw), probe_ports);
            d.hostname = name;
            d.ports = open;
            d
        });
    }
    while let Some(Ok(mut d)) = set.join_next().await {
        d.vendor = d.mac.and_then(|m| oui.vendor(&m).map(str::to_string));
        devices.insert(d.ip.into(), d);
    }

    if json {
        let list: Vec<&Device> = devices.values().collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }
    for d in devices.values() {
        print_device(d, &oui);
    }
    println!("{}", dim(&format!("{} device(s) on {}", devices.len(), subnet)));
    if oui.is_empty() {
        println!("{}", dim("tip: run scripts/get-oui.ps1 once to show manufacturers"));
    }
    Ok(())
}

fn print_device(d: &Device, oui: &OuiDb) {
    let mut role = Vec::new();
    if d.is_gateway {
        role.push(yellow("gateway"));
    }
    if d.is_self {
        role.push(cyan("this pc"));
    }
    let dot = if d.is_gateway { yellow("◆") } else if d.is_self { cyan("●") } else { green("●") };
    println!("{dot} {}  {}", bold(&d.ip.to_string()), role.join(" "));

    if let Some(m) = d.mac {
        let who = if m.is_locally_administered() {
            dim("randomized / private MAC (no vendor info)")
        } else {
            match oui.vendor(&m) {
                Some(v) => v.to_string(),
                None => dim("vendor unknown"),
            }
        };
        println!("    {}  {m}  {who}", dim("mac  "));
    }
    if let Some(n) = &d.hostname {
        println!("    {}  {n}", dim("name "));
    }
    for u in &d.upnp {
        let id = u.identity().unwrap_or_else(|| u.server.clone().unwrap_or("UPnP device".into()));
        let fname = u.friendly_name.as_ref().map(|f| format!("  \"{f}\"")).unwrap_or_default();
        println!("    {}  {}{}", dim("upnp "), bold(&id), dim(&fname));
    }
    for p in &d.ports {
        let b = p.banner.as_deref().map(|b| format!("  {}", dim(b))).unwrap_or_default();
        println!("    {}  {:<5} {}{b}", dim("port "), p.port, p.label);
    }
    if let Some(r) = d.rtt_ms {
        println!("    {}  {r} ms", dim("arp  "));
    }
    println!();
}

async fn cmd_watch(cidr: Option<String>, interval: u64) -> Result<()> {
    let info = net::detect()?;
    let subnet = target_subnet(&info, cidr)?;
    let oui = OuiDb::load();
    let hosts = subnet.hosts();
    header(&info);
    println!("{}", dim(&format!("watching {subnet} every {interval}s; Ctrl+C to stop")));

    let mut known: HashMap<Ipv4Addr, (Option<MacAddr>, u32)> = HashMap::new();
    let mut first = true;
    loop {
        let seen = discover::sweep(&hosts, 64).await;
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        let describe = |mac: Option<MacAddr>| match mac {
            Some(m) if m.is_locally_administered() => format!("{m} (private MAC)"),
            Some(m) => format!("{m} {}", oui.vendor(&m).unwrap_or("")),
            None => String::new(),
        };

        let mut present = Vec::new();
        for s in &seen {
            present.push(s.ip);
            match known.get_mut(&s.ip) {
                None => {
                    if !first {
                        println!("{} {} {:<15} {}", dim(&now), green("+ joined"), s.ip, describe(s.mac));
                    }
                    known.insert(s.ip, (s.mac, 0));
                }
                Some((mac, misses)) => {
                    *misses = 0;
                    if s.mac.is_some() && *mac != s.mac {
                        println!("{} {} {:<15} now {}", dim(&now), yellow("~ changed"), s.ip, describe(s.mac));
                        *mac = s.mac;
                    }
                }
            }
        }
        // A device must miss two sweeps before we call it gone; phones nap.
        let mut gone = Vec::new();
        for (ip, (mac, misses)) in known.iter_mut() {
            if !present.contains(ip) {
                *misses += 1;
                if *misses >= 2 {
                    gone.push((*ip, *mac));
                }
            }
        }
        for (ip, mac) in gone {
            known.remove(&ip);
            println!("{} {} {:<15} {}", dim(&now), red("- left  "), ip, describe(mac));
        }
        if first {
            println!("{} {} device(s) online", dim(&now), bold(&known.len().to_string()));
            let mut ips: Vec<_> = known.iter().collect();
            ips.sort_by_key(|(ip, _)| u32::from(**ip));
            for (ip, (mac, _)) in ips {
                println!("         {:<15} {}", ip, dim(&describe(*mac)));
            }
            first = false;
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

async fn cmd_upnp(wait: u64, json: bool) -> Result<()> {
    let info = net::detect()?;
    let hits = ssdp::discover(info.local_ip, Duration::from_secs(wait)).await?;
    if json {
        let v: Vec<_> = hits.iter().map(|h| serde_json::json!({ "ip": h.ip, "info": h.info })).collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if hits.is_empty() {
        println!("No UPnP answers in {wait}s. Phone hotspots usually have none; home routers, smart TVs and Android boxes usually do.");
    }
    for h in &hits {
        let i = &h.info;
        println!("{}  {}", bold(&h.ip.to_string()), i.identity().unwrap_or("-".into()));
        if let Some(n) = &i.friendly_name {
            println!("    {}  {n}", dim("name  "));
        }
        if let Some(t) = &i.device_type {
            println!("    {}  {t}", dim("type  "));
        }
        if let Some(s) = &i.server {
            println!("    {}  {s}", dim("server"));
        }
        println!("    {}  {}\n", dim("desc  "), dim(&i.location));
    }
    Ok(())
}
