//! HTTP + WebSocket API. Reads are open on the LAN; anything that changes
//! state needs the RHCSD_TOKEN (or, if no token is set, a request from this
//! machine itself).

use crate::state::{now_ms, Shared};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use tokio::sync::broadcast::error::RecvError;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

pub const CAPABILITIES: &[&str] = &["telemetry", "history", "devices", "phone", "events", "router_relogin", "scan"];

pub fn router(s: Shared) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        // Lets an https page (e.g. the dashboard on Vercel) call this LAN service.
        .allow_private_network(true);

    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/router/telemetry", get(telemetry))
        .route("/api/signal/history", get(history))
        .route("/api/devices", get(devices))
        .route("/api/phone", get(phone))
        .route("/api/events", get(events))
        .route("/api/scan", post(scan))
        .route("/api/router/action", post(router_action))
        .route("/api/settings", get(get_settings).post(set_settings))
        .route("/ws", get(ws))
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

fn authorized(s: &Shared, addr: SocketAddr, headers: &HeaderMap) -> bool {
    match &s.cfg.token {
        Some(token) => {
            let direct = headers.get("x-rhcs-token").and_then(|v| v.to_str().ok());
            let bearer = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "));
            direct == Some(token.as_str()) || bearer == Some(token.as_str())
        }
        None => addr.ip().is_loopback(),
    }
}

fn denied() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "ok": false, "message": "not allowed: send x-rhcs-token, or call from this machine" })),
    )
        .into_response()
}

async fn health(State(s): State<Shared>) -> Json<Value> {
    let sources = s.inner.read().await.sources.clone();
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": s.started.elapsed().as_secs(),
        "capabilities": CAPABILITIES,
        "auth_required": s.cfg.token.is_some(),
        "sources": sources,
    }))
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
