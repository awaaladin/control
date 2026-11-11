//! rhcsd: keeps polling the router, the phone and the LAN, and serves the
//! results over HTTP + WebSocket for the dashboard and the phone app.
//! Each source runs independently: if the router or phone is unplugged,
//! the rest keeps working and the missing one reconnects by itself.

mod api;
mod phone;
mod router_task;
mod scan_task;
mod state;

use anyhow::{Context, Result};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = state::Config::from_env();
    let s = state::AppState::new(cfg.clone());

    tokio::spawn(router_task::run(s.clone()));
    tokio::spawn(phone::run(s.clone()));
    tokio::spawn(scan_task::run(s.clone()));

    let app = api::router(s.clone());
    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("cannot listen on {}", cfg.bind))?;

    println!("rhcsd {} listening on http://{}", env!("CARGO_PKG_VERSION"), cfg.bind);
    println!("  router  {} ({})", cfg.router_ip, cfg.router_model);
    println!(
        "  phone   adb {}{}",
        cfg.phone_serial.as_deref().unwrap_or("(first device)"),
        cfg.phone_adb_addr.as_deref().map(|a| format!(", wifi {a}")).unwrap_or_default()
    );
    println!("  auth    {}", if cfg.token.is_some() { "token required for changes" } else { "changes allowed from this machine only" });
    if cfg.web_dir.join("index.html").is_file() {
        println!("  web     {}", cfg.web_dir.display());
    }

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            println!("shutting down");
        })
        .await?;
    Ok(())
}
