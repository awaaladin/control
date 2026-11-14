//! HTTP + WebSocket API. Reads are open on the LAN; anything that changes
//! state needs the RHCSD_TOKEN (or, if no token is set, a request from this
//! machine itself). Requests through a tunnel always need the token: see auth.rs.

use crate::state::{now_ms, Shared};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use tokio::sync::broadcast::error::RecvError;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

pub const CAPABILITIES: &[&str] = &[
    "telemetry", "history", "devices", "phone", "events", "router_relogin", "scan", "boost", "towers", "location", "connections",
    "places", "switch", "boost_auto",
];

/// The hosted PULSE site, always allowed to call a home system.
pub const DEFAULT_ORIGIN: &str = "https://rhcs-pulse.vercel.app";

/// May a page from this browser origin read the API? The hosted PULSE site, origins listed in
/// RHCSD_ALLOWED_ORIGINS, and pages on this very machine (rhcsd serves PULSE itself on
/// localhost / 127.0.0.1, any port). Requests without an Origin header (same-origin
/// pages, curl, the app) never reach this check. Any other website gets no CORS headers,
/// so the browser will not let it read answers.
pub fn origin_allowed(origin: &str, extra: &[String]) -> bool {
    let o = origin.trim().trim_end_matches('/');
    if o.eq_ignore_ascii_case(DEFAULT_ORIGIN) || extra.iter().any(|e| e.eq_ignore_ascii_case(o)) {
        return true;
    }
    let Some(rest) = o.strip_prefix("http://").or_else(|| o.strip_prefix("https://")) else { return false };
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if !rest.starts_with('[') || h.ends_with(']') => (h, Some(p)),
        _ => (rest, None),
    };
    if port.is_some_and(|p| p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

pub fn router(s: Shared) -> Router {
    let extra = s.cfg.allowed_origins.clone();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin: &HeaderValue, _| {
            origin.to_str().is_ok_and(|o| origin_allowed(o, &extra))
        }))
        .allow_methods(Any)
        // `authorization` is not covered by a wildcard, so list the headers.
        .allow_headers([
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::AUTHORIZATION,
            HeaderName::from_static("x-rhcs-token"),
        ])
        // Lets an https page (e.g. the dashboard on Vercel) call this LAN service.
        .allow_private_network(true);

    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/engine", get(engine))
        .route("/api/engine/stop", post(engine_stop))
        .route("/api/router/telemetry", get(telemetry))
        .route("/api/signal/history", get(history))
        .route("/api/devices", get(devices))
        .route("/api/phone", get(phone))
        .route("/api/events", get(events))
        .route("/api/scan", post(scan))
        .route("/api/router/action", post(router_action))
        .route("/api/settings", get(get_settings).post(set_settings))
        .route("/ws", get(ws))
        .merge(crate::boost::routes())
        .merge(crate::towers::routes())
        .merge(crate::places::routes())
        .merge(crate::connections::routes())
        .merge(crate::switch::routes())
        .merge(crate::push::routes())
        .layer(axum::middleware::from_fn_with_state(s.clone(), crate::auth::gate))
        .with_state(s.clone());

    let web = s.cfg.web_dir.clone();
    let app = if web.join("index.html").is_file() {
        api.fallback_service(ServeDir::new(&web).fallback(ServeFile::new(web.join("index.html"))))
    } else {
        api.fallback(|| async {
            (StatusCode::NOT_FOUND, "rhcsd is running. No dashboard build found (web/dist); the API is under /api and /ws.")
        })
    };
    app.layer(cors)
}

pub(crate) fn authorized(s: &Shared, addr: SocketAddr, headers: &HeaderMap) -> bool {
    crate::auth::check_write(s, addr, headers)
}

pub(crate) fn denied() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "ok": false, "message": "not allowed: send the access token (x-rhcs-token), or call from this machine" })),
    )
        .into_response()
}

async fn health(State(s): State<Shared>) -> Json<Value> {
    let sources = s.inner.read().await.sources.clone();
    let mut capabilities: Vec<&str> = CAPABILITIES.to_vec();
    if s.cfg.app_mode {
        capabilities.push("app_push");
    }
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": s.started.elapsed().as_secs(),
        "capabilities": capabilities,
        "app_mode": s.cfg.app_mode,
        "auth_required": s.cfg.token.is_some(),
        "admin": s.admin,
        "switch_supported": crate::switch::supported(),
        "sources": sources,
    }))
}

/// Where this rhcsd runs, for PULSE's "engine" controls.
fn engine_mode(app: bool) -> &'static str {
    if app {
        "app"
    } else if crate::phone::in_termux() {
        "termux"
    } else if cfg!(windows) {
        "windows"
    } else {
        "other"
    }
}

async fn engine(State(s): State<Shared>) -> Json<Value> {
    Json(json!({
        "running": true,
        "mode": engine_mode(s.cfg.app_mode),
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": s.started.elapsed().as_secs(),
        "can_stop": true,
    }))
}

/// Ends the process cleanly (exit code 0) about half a second after answering. The Android
/// service reads exit code 0 as "stopped by the user" and does not restart it.
async fn engine_stop(State(s): State<Shared>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if !authorized(&s, addr, &headers) {
        return denied();
    }
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        s.shutdown.notify_one();
        // Open WebSockets would keep a graceful shutdown waiting: leave anyway after a moment.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        std::process::exit(0);
    });
    Json(json!({ "ok": true, "message": "Engine is stopping" })).into_response()
}

async fn telemetry(State(s): State<Shared>) -> Response {
    match &s.inner.read().await.telemetry {
        Some(t) => Json(json!(t)).into_response(),
        None => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no router reading yet" }))).into_response(),
    }
}

#[derive(Deserialize)]
struct HistoryQuery {
    minutes: Option<u64>,
}

async fn history(State(s): State<Shared>, Query(q): Query<HistoryQuery>) -> Json<Value> {
    let minutes = q.minutes.unwrap_or(60).clamp(1, 1440) as i64;
    let since = now_ms() - minutes * 60_000;
    let i = s.inner.read().await;
    let rows: Vec<_> = i.history.iter().filter(|r| r.ts >= since).collect();
    Json(json!(rows))
}

async fn devices(State(s): State<Shared>) -> Json<Value> {
    Json(json!(s.inner.read().await.devices))
}

async fn phone(State(s): State<Shared>) -> Response {
    match &s.inner.read().await.phone {
        Some(p) => Json(json!(p)).into_response(),
        None => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "phone not connected" }))).into_response(),
    }
}

async fn events(State(s): State<Shared>) -> Json<Value> {
    Json(json!(s.inner.read().await.events))
}

async fn scan(State(s): State<Shared>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if !authorized(&s, addr, &headers) {
        return denied();
    }
    s.scan_now.notify_one();
    Json(json!({ "ok": true, "message": "scan started" })).into_response()
}

#[derive(Deserialize)]
struct ActionRequest {
    action: String,
    #[allow(dead_code)]
    value: Option<String>,
}

async fn router_action(
    State(s): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ActionRequest>,
) -> Response {
    if !authorized(&s, addr, &headers) {
        return denied();
    }
    match req.action.as_str() {
        "relogin" => {
            s.relogin.store(true, Ordering::SeqCst);
            Json(json!({ "ok": true, "message": "router session reset; logging in on the next reading" })).into_response()
        }
        "reconnect" | "disconnect" | "set_network_mode" => (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({ "ok": false, "message": format!("'{}' is not implemented yet: it changes router settings and hasn't been tested on this router", req.action) })),
        )
            .into_response(),
        other => (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "message": format!("unknown action '{other}'") })))
            .into_response(),
    }
}

async fn get_settings(State(s): State<Shared>) -> Json<Value> {
    let i = s.inner.read().await;
    Json(json!({ "interval_s": i.interval_s, "scan_interval_s": s.cfg.scan_interval_s }))
}

#[derive(Deserialize)]
struct SettingsUpdate {
    interval_s: Option<f64>,
}

async fn set_settings(
    State(s): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<SettingsUpdate>,
) -> Response {
    if !authorized(&s, addr, &headers) {
        return denied();
    }
    let mut i = s.inner.write().await;
    if let Some(v) = req.interval_s {
        i.interval_s = v.clamp(0.5, 60.0);
    }
    Json(json!({ "ok": true, "interval_s": i.interval_s })).into_response()
}

async fn ws(ws: WebSocketUpgrade, State(s): State<Shared>) -> Response {
    ws.on_upgrade(move |socket| client(socket, s))
}

async fn client(mut socket: WebSocket, s: Shared) {
    let mut rx = s.tx.subscribe();
    for msg in s.snapshot().await {
        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, Config};
    use std::collections::HashMap;

    #[test]
    fn cors_origin_decision() {
        let extra = vec!["https://panel.example.org".to_string()];
        for ok in [
            "https://rhcs-pulse.vercel.app",
            "https://RHCS-PULSE.vercel.app/",
            "https://panel.example.org",
            "http://localhost",
            "http://localhost:7878",
            "http://127.0.0.1:7979",
            "https://localhost:5173",
            "http://[::1]:7878",
        ] {
            assert!(origin_allowed(ok, &extra), "{ok}");
        }
        for bad in [
            "https://evil.example",
            "http://rhcs-pulse.vercel.app", // wrong scheme
            "https://rhcs-pulse.vercel.app.evil.example",
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example:80",
            "http://localhost:abc",
            "http://localhost:",
            "http://localhost:1234567",
            "https://panel.example.org.evil.example",
            "null",
            "",
            "file://",
        ] {
            assert!(!origin_allowed(bad, &extra), "{bad}");
        }
        // without RHCSD_ALLOWED_ORIGINS the extra site is not allowed
        assert!(!origin_allowed("https://panel.example.org", &[]));
    }

    fn state(app: bool) -> Shared {
        let mut m = HashMap::new();
        m.insert("RHCSD_TOKEN".to_string(), "k".repeat(32));
        m.insert("RHCSD_DATA_DIR".to_string(), std::env::temp_dir().join(format!("rhcsd-test-api-{}", std::process::id())).display().to_string());
        m.insert("RHCSD_ALLOWED_ORIGINS".to_string(), " https://a.example/ , ,https://b.example".to_string());
        if app {
            m.insert("RHCSD_APP".to_string(), "1".to_string());
        }
        AppState::new(Config::from_map(&m))
    }

    fn headers(token: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(t) = token {
            h.insert("x-rhcs-token", HeaderValue::from_str(t).unwrap());
        }
        h
    }

    #[test]
    fn engine_stop_auth_outcomes() {
        let local: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let lan: SocketAddr = "192.0.2.10:50000".parse().unwrap();
        for app in [true, false] {
            let s = state(app);
            let good = "k".repeat(32);
            assert!(authorized(&s, local, &headers(Some(&good))), "right token from this machine, app={app}");
            assert!(!authorized(&s, local, &headers(None)), "no token, app={app}");
            assert!(!authorized(&s, local, &headers(Some("wrong"))), "wrong token, app={app}");
            assert!(authorized(&s, lan, &headers(Some(&good))), "right token from the LAN, app={app}");
            assert!(!authorized(&s, lan, &headers(None)), "no token from the LAN, app={app}");
            let _ = std::fs::remove_dir_all(&s.cfg.data_dir);
        }
    }

    #[test]
    fn engine_modes_and_origins_from_config() {
        assert_eq!(engine_mode(true), "app");
        assert!(matches!(engine_mode(false), "termux" | "windows" | "other"));
        let s = state(false);
        assert_eq!(s.cfg.allowed_origins, vec!["https://a.example".to_string(), "https://b.example".to_string()]);
        let _ = std::fs::remove_dir_all(&s.cfg.data_dir);
    }
}
