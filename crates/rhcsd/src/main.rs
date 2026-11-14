//! rhcsd: keeps polling the router, the phone and the LAN, and serves the
//! results over HTTP + WebSocket for the dashboard and the phone app.
//! Each source runs independently: if the router or phone is unplugged,
//! the rest keeps working and the missing one reconnects by itself.

mod api;
mod auth;
mod boost;
mod connections;
mod links;
mod location;
mod phone;
mod places;
mod push;
mod router_task;
mod scan_task;
mod state;
mod switch;
mod towers;

use anyhow::{Context, Result};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = state::Config::from_env();
    if cfg.app_mode && !cfg.token.as_deref().is_some_and(|t| t.len() >= auth::MIN_REMOTE_TOKEN) {
        anyhow::bail!("app mode (RHCSD_APP=1) needs RHCSD_TOKEN of at least {} characters", auth::MIN_REMOTE_TOKEN);
    }
    if cfg.app_mode {
        std::fs::create_dir_all(&cfg.data_dir).with_context(|| format!("cannot create {}", cfg.data_dir.display()))?;
    }
    let s = state::AppState::new(cfg.clone());

    if cfg.router_configured {
        tokio::spawn(router_task::run(s.clone()));
    }
    if cfg.app_mode {
        // The app pushes the phone's readings and position; no ADB, no Termux.
        tokio::spawn(push::watch(s.clone()));
    } else {
        tokio::spawn(phone::run(s.clone()));
        tokio::spawn(location::run(s.clone()));
    }
    tokio::spawn(scan_task::run(s.clone()));
    tokio::spawn(towers::run(s.clone()));
    tokio::spawn(towers::geolocate(s.clone()));

    let app = api::router(s.clone());
    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("cannot listen on {}", cfg.bind))?;

    println!("rhcsd {} listening on http://{}", env!("CARGO_PKG_VERSION"), cfg.bind);
    if cfg.router_configured {
        println!("  router  {} ({})", cfg.router_ip, cfg.router_model);
    } else {
        println!("  router  none configured (set ROUTER_IP to poll one)");
    }
    if cfg.app_mode {
        println!("  phone   pushed by the app (RHCSD_APP=1); data in {}", cfg.data_dir.display());
        println!("  auth    token required for every /api and /ws call, even from this phone");
    } else {
        println!(
            "  phone   adb {}{}",
            cfg.phone_serial.as_deref().unwrap_or("(first device)"),
            cfg.phone_adb_addr.as_deref().map(|a| format!(", wifi {a}")).unwrap_or_default()
        );
        println!("  auth    {}", if cfg.token.is_some() { "token required for changes" } else { "changes allowed from this machine only" });
    }
    println!(
        "  remote  {}",
        match &cfg.token {
            Some(t) if t.len() >= auth::MIN_REMOTE_TOKEN => "tunnels allowed (token required for everything)",
            _ => "tunnels refused: set RHCSD_TOKEN of at least 20 characters to use PULSE away from home",
        }
    );
    if cfg.lock_reads {
        println!("  reads   token required on the LAN too (RHCSD_LOCK_READS)");
    }
    println!(
        "  admin   {}",
        if s.admin { "yes: PULSE can switch links" } else { "no: link switching off (start rhcsd as administrator to enable it)" }
    );
    if cfg.web_dir.join("index.html").is_file() {
        println!("  web     {}", cfg.web_dir.display());
    }

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown({
            let s = s.clone();
            async move {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = s.shutdown.notified() => {}
                }
                println!("shutting down");
            }
        })
        .await?;
    Ok(())
}
