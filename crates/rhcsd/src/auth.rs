//! Who is calling, and are they allowed in.
//!
//! Every request is put in one of three classes:
//!   local  - peer is loopback and no proxy/tunnel header is present
//!   lan    - peer is not loopback (home Wi-Fi, hotspot, Tailscale direct)
//!   tunnel - peer is loopback but a proxy header is present (cloudflared and
//!            `tailscale serve` connect from localhost and add such headers)
//! A tunnel makes the whole internet look like loopback, so tunnel requests
//! always need a token of at least 20 characters. Tokens are never logged.
//!
//! App mode (`RHCSD_APP=1`, the Android app): other apps on the phone can reach
//! loopback, so every class needs the token for everything under /api and /ws.
//! Wrong tokens from loopback are not rate limited there, because one hostile
//! app could otherwise lock the real client out; the token is long and random.

use crate::state::Shared;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const MIN_REMOTE_TOKEN: usize = 20;
const MAX_FAILS: u32 = 10;
const WINDOW: Duration = Duration::from_secs(60);
const LOCKOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Local,
    Lan,
    Tunnel,
}

const PROXY_HEADERS: &[&str] = &[
    "x-forwarded-for",
    "x-forwarded-proto",
    "x-forwarded-host",
    "forwarded",
    "x-real-ip",
    "via",
    "cf-connecting-ip",
    "cf-ray",
    "cf-visitor",
    "true-client-ip",
];

fn has_proxy_header(h: &HeaderMap) -> bool {
    h.keys().any(|k| {
        let n = k.as_str(); // header names are lower-case here
        PROXY_HEADERS.contains(&n) || n.starts_with("cf-") || n.starts_with("tailscale-")
    })
}

pub fn classify(peer: IpAddr, h: &HeaderMap) -> Class {
    if !peer.to_canonical().is_loopback() {
        Class::Lan
    } else if has_proxy_header(h) {
        Class::Tunnel
    } else {
        Class::Local
    }
}

/// The caller's address for rate limiting: behind a tunnel the proxy's
/// header, otherwise the TCP peer. For X-Forwarded-For the LAST entry is used
/// because the proxy appends the address it saw; earlier entries can be forged.
pub fn client_key(class: Class, peer: IpAddr, h: &HeaderMap) -> String {
    if class == Class::Tunnel {
        let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty());
        if let Some(v) = get("cf-connecting-ip") {
            return v.to_string();
        }
        if let Some(v) = get("x-forwarded-for").and_then(|v| v.rsplit(',').next()) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    peer.to_canonical().to_string()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Token from `x-rhcs-token`, `Authorization: Bearer`, or `?token=` (the
/// WebSocket case, since browsers cannot set headers there).
pub fn presented_token(h: &HeaderMap, query: Option<&str>) -> Option<String> {
    let direct = h.get("x-rhcs-token").and_then(|v| v.to_str().ok()).map(str::trim);
    if let Some(t) = direct.filter(|t| !t.is_empty()) {
        return Some(t.to_string());
    }
    let bearer = h
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    if let Some(t) = bearer.filter(|t| !t.is_empty()) {
        return Some(t.to_string());
    }
    query
        .into_iter()
        .flat_map(|q| q.split('&'))
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == "token")
        .map(|(_, v)| percent_decode(v))
        .filter(|t| !t.is_empty())
}

/// Constant-time comparison (time depends only on the longer length).
pub fn token_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    let mut diff = (a.len() != b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Unauthenticated health probe: answer only {ok, version, auth_required}.
    HealthOnly,
    /// Remote access refused because no usable RHCSD_TOKEN is configured.
    RemoteDisabled,
    TokenMissing,
    TokenWrong,
}

/// Gate for every /api and /ws request. Local and lan pass through (the
/// handlers still check changes with `write_allowed`) unless `lock_reads`
/// is on for lan; tunnel always needs the token.
pub fn decide(class: Class, is_health: bool, token: Option<&str>, lock_reads: bool, app: bool, presented: Option<&str>) -> Decision {
    let protected = app
        || match class {
            Class::Tunnel => true,
            Class::Lan => lock_reads,
            Class::Local => false,
        };
    if !protected {
        return Decision::Allow;
    }
    let min = if class == Class::Tunnel || app { MIN_REMOTE_TOKEN } else { 1 };
    let Some(token) = token.filter(|t| t.len() >= min) else {
        return Decision::RemoteDisabled;
    };
    match presented {
        None => {
            if is_health {
                Decision::HealthOnly
            } else {
                Decision::TokenMissing
            }
        }
        Some(p) if token_matches(token, p) => Decision::Allow,
        Some(_) => Decision::TokenWrong,
    }
}

/// May this caller change things?
pub fn write_allowed(class: Class, token: Option<&str>, presented: Option<&str>) -> bool {
    match (class, token) {
        (Class::Tunnel, Some(t)) if t.len() >= MIN_REMOTE_TOKEN => presented.is_some_and(|p| token_matches(t, p)),
        (Class::Tunnel, _) => false,
        (_, Some(t)) => presented.is_some_and(|p| token_matches(t, p)),
        (c, None) => c == Class::Local,
    }
}

struct Entry {
    fails: u32,
    window_start: Instant,
    locked_until: Option<Instant>,
}

#[derive(Default)]
pub struct Limiter {
    map: Mutex<HashMap<String, Entry>>,
}

impl Limiter {
    /// Seconds to wait if this key is locked out.
    pub fn locked(&self, key: &str, now: Instant) -> Option<u64> {
        let m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let until = m.get(key)?.locked_until?;
        (until > now).then(|| (until - now).as_secs().max(1))
    }

    pub fn fail(&self, key: &str, now: Instant) {
        let mut m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() > 4096 {
            m.retain(|_, e| e.locked_until.is_some_and(|u| u > now) || now.duration_since(e.window_start) < WINDOW);
        }
        let e = m.entry(key.to_string()).or_insert(Entry { fails: 0, window_start: now, locked_until: None });
        if e.locked_until.is_some_and(|u| u <= now) {
            e.locked_until = None;
            e.fails = 0;
        }
        if now.duration_since(e.window_start) >= WINDOW {
            e.window_start = now;
            e.fails = 0;
        }
        e.fails += 1;
        if e.fails >= MAX_FAILS {
            e.locked_until = Some(now + LOCKOUT);
        }
    }

    pub fn ok(&self, key: &str) {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
    }
}

pub fn limiter() -> &'static Limiter {
    static L: OnceLock<Limiter> = OnceLock::new();
    L.get_or_init(Limiter::default)
}

fn reply(status: StatusCode, error: &str, message: &str) -> Response {
    (status, Json(json!({ "ok": false, "error": error, "message": message }))).into_response()
}

fn too_many(secs: u64) -> Response {
    let mut r = reply(
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_attempts",
        "too many wrong token attempts from this address; try again in a few minutes",
    );
    if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
        r.headers_mut().insert(header::RETRY_AFTER, v);
    }
    r
}

/// Write check used by the handlers (signature kept for existing callers).
pub fn check_write(s: &Shared, addr: SocketAddr, headers: &HeaderMap) -> bool {
    let class = classify(addr.ip(), headers);
    let key = client_key(class, addr.ip(), headers);
    let now = Instant::now();
    let limited = limit_applies(s.cfg.app_mode, class);
    if limited && limiter().locked(&key, now).is_some() {
        return false;
    }
    let presented = presented_token(headers, None);
    let ok = write_allowed(class, s.cfg.token.as_deref(), presented.as_deref());
    if !limited {
        return ok;
    }
    if !ok && presented.is_some() && s.cfg.token.is_some() {
        limiter().fail(&key, now);
    } else if ok && presented.is_some() {
        limiter().ok(&key);
    }
    ok
}

/// Wrong-token lockouts apply everywhere except loopback in app mode (see the module notes).
pub fn limit_applies(app: bool, class: Class) -> bool {
    !(app && class == Class::Local)
}

/// Middleware in front of every /api route and /ws.
pub async fn gate(State(s): State<Shared>, ConnectInfo(addr): ConnectInfo<SocketAddr>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let class = classify(addr.ip(), headers);
    let is_health = req.uri().path() == "/api/health";
    let presented = presented_token(headers, req.uri().query());
    let app = s.cfg.app_mode;
    let decision = decide(class, is_health, s.cfg.token.as_deref(), s.cfg.lock_reads, app, presented.as_deref());
    if decision == Decision::Allow && presented.is_none() {
        return next.run(req).await;
    }
    let key = client_key(class, addr.ip(), headers);
    let now = Instant::now();
    let limited = limit_applies(app, class);
    if limited && (decision != Decision::Allow || class != Class::Local) {
        if let Some(secs) = limiter().locked(&key, now) {
            return too_many(secs);
        }
    }
    match decision {
        Decision::Allow => {
            // a token was sent on a request that did not need one; do not judge it here
            if class != Class::Local {
                if let Some(t) = s.cfg.token.as_deref() {
                    if presented.as_deref().is_some_and(|p| token_matches(t, p)) {
                        limiter().ok(&key);
                    }
                }
            }
            next.run(req).await
        }
        Decision::HealthOnly => Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "auth_required": true })).into_response(),
        Decision::RemoteDisabled => reply(
            StatusCode::FORBIDDEN,
            "remote_token_not_configured",
            "this needs RHCSD_TOKEN of at least 20 characters",
        ),
        Decision::TokenMissing => reply(StatusCode::UNAUTHORIZED, "token_missing", "this backend needs an access token"),
        Decision::TokenWrong => {
            if limited {
                limiter().fail(&key, now);
            }
            reply(StatusCode::UNAUTHORIZED, "token_wrong", "wrong access token")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "abcdefghijklmnopqrstuvwx"; // 24 chars, test only
    const SHORT: &str = "short-token";

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    }
    fn lo() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }
    fn lan() -> IpAddr {
        "192.0.2.10".parse().unwrap()
    }

    #[test]
    fn classifies() {
        assert_eq!(classify(lo(), &hm(&[])), Class::Local);
        assert_eq!(classify("::1".parse().unwrap(), &hm(&[("accept", "x")])), Class::Local);
        assert_eq!(classify("::ffff:127.0.0.1".parse().unwrap(), &hm(&[])), Class::Local);
        assert_eq!(classify(lan(), &hm(&[])), Class::Lan);
        assert_eq!(classify("100.64.0.5".parse().unwrap(), &hm(&[])), Class::Lan);
        // proxy headers from a non-loopback peer stay lan
        assert_eq!(classify(lan(), &hm(&[("x-forwarded-for", "198.51.100.1")])), Class::Lan);
        for name in [
            "x-forwarded-for", "x-forwarded-proto", "x-forwarded-host", "forwarded", "x-real-ip", "via", "cf-connecting-ip",
            "cf-ray", "cf-visitor", "true-client-ip", "cf-ipcountry", "Cf-Anything", "tailscale-user-login", "Tailscale-Foo",
        ] {
            assert_eq!(classify(lo(), &hm(&[(name, "v")])), Class::Tunnel, "{name}");
        }
        assert_eq!(classify(lo(), &hm(&[("x-custom", "v"), ("host", "localhost")])), Class::Local);
    }

    #[test]
    fn extracts_tokens() {
        assert_eq!(presented_token(&hm(&[("x-rhcs-token", "abc")]), None).as_deref(), Some("abc"));
        assert_eq!(presented_token(&hm(&[("authorization", "Bearer abc")]), None).as_deref(), Some("abc"));
        assert_eq!(presented_token(&hm(&[]), Some("a=1&token=abc&b=2")).as_deref(), Some("abc"));
        assert_eq!(presented_token(&hm(&[]), Some("token=a%2Bb%3D")).as_deref(), Some("a+b="));
        assert_eq!(presented_token(&hm(&[]), Some("token=")), None);
        assert_eq!(presented_token(&hm(&[("authorization", "Basic abc")]), None), None);
        assert_eq!(presented_token(&hm(&[]), None), None);
    }

    #[test]
    fn compares() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abd"));
        assert!(!token_matches("abc", "abcd"));
        assert!(!token_matches("abcd", "abc"));
        assert!(!token_matches("", "x"));
    }

    #[test]
    fn tunnel_always_needs_token() {
        let t = Class::Tunnel;
        assert_eq!(decide(t, false, Some(TOKEN), false, false, Some(TOKEN)), Decision::Allow);
        assert_eq!(decide(t, false, Some(TOKEN), false, false, None), Decision::TokenMissing);
        assert_eq!(decide(t, false, Some(TOKEN), false, false, Some("wrong")), Decision::TokenWrong);
        assert_eq!(decide(t, true, Some(TOKEN), false, false, None), Decision::HealthOnly);
        assert_eq!(decide(t, true, Some(TOKEN), false, false, Some("wrong")), Decision::TokenWrong);
        // unset or short token: refuse, never fall back to open
        assert_eq!(decide(t, false, None, false, false, None), Decision::RemoteDisabled);
        assert_eq!(decide(t, true, None, false, false, None), Decision::RemoteDisabled);
        assert_eq!(decide(t, false, Some(SHORT), false, false, Some(SHORT)), Decision::RemoteDisabled);
        assert_eq!(decide(t, false, None, false, false, Some("anything")), Decision::RemoteDisabled);
    }

    #[test]
    fn local_and_lan_rules() {
        assert_eq!(decide(Class::Local, false, None, false, false, None), Decision::Allow);
        assert_eq!(decide(Class::Local, false, Some(TOKEN), true, false, None), Decision::Allow);
        assert_eq!(decide(Class::Lan, false, None, false, false, None), Decision::Allow);
        assert_eq!(decide(Class::Lan, false, Some(TOKEN), false, false, None), Decision::Allow);
        // lock reads on the lan
        assert_eq!(decide(Class::Lan, false, Some(TOKEN), true, false, None), Decision::TokenMissing);
        assert_eq!(decide(Class::Lan, true, Some(TOKEN), true, false, None), Decision::HealthOnly);
        assert_eq!(decide(Class::Lan, false, Some(TOKEN), true, false, Some(TOKEN)), Decision::Allow);
        assert_eq!(decide(Class::Lan, false, Some(TOKEN), true, false, Some("x")), Decision::TokenWrong);
        assert_eq!(decide(Class::Lan, false, None, true, false, None), Decision::RemoteDisabled);
    }

    #[test]
    fn websocket_query_token() {
        let p = presented_token(&hm(&[]), Some(&format!("token={TOKEN}")));
        assert_eq!(decide(Class::Tunnel, false, Some(TOKEN), false, false, p.as_deref()), Decision::Allow);
        let p = presented_token(&hm(&[]), Some("token=nope"));
        assert_eq!(decide(Class::Tunnel, false, Some(TOKEN), false, false, p.as_deref()), Decision::TokenWrong);
    }

    #[test]
    fn write_rules() {
        // local: open without token, token needed when one is set
        assert!(write_allowed(Class::Local, None, None));
        assert!(!write_allowed(Class::Local, Some(TOKEN), None));
        assert!(write_allowed(Class::Local, Some(TOKEN), Some(TOKEN)));
        assert!(!write_allowed(Class::Local, Some(TOKEN), Some("x")));
        // lan: always token
        assert!(!write_allowed(Class::Lan, None, None));
        assert!(!write_allowed(Class::Lan, None, Some(TOKEN)));
        assert!(write_allowed(Class::Lan, Some(TOKEN), Some(TOKEN)));
        assert!(!write_allowed(Class::Lan, Some(TOKEN), None));
        // tunnel: token of 20+ only
        assert!(!write_allowed(Class::Tunnel, None, None));
        assert!(!write_allowed(Class::Tunnel, Some(SHORT), Some(SHORT)));
        assert!(!write_allowed(Class::Tunnel, Some(TOKEN), None));
        assert!(write_allowed(Class::Tunnel, Some(TOKEN), Some(TOKEN)));
    }

    #[test]
    fn client_keys() {
        let t = hm(&[("cf-connecting-ip", "203.0.113.9"), ("x-forwarded-for", "1.1.1.1, 2.2.2.2")]);
        assert_eq!(client_key(Class::Tunnel, lo(), &t), "203.0.113.9");
        let x = hm(&[("x-forwarded-for", "forged, 203.0.113.7")]);
        assert_eq!(client_key(Class::Tunnel, lo(), &x), "203.0.113.7");
        assert_eq!(client_key(Class::Tunnel, lo(), &hm(&[("via", "1.1 x")])), "127.0.0.1");
        assert_eq!(client_key(Class::Lan, lan(), &t), "192.0.2.10");
    }

    #[test]
    fn rate_limit() {
        let l = Limiter::default();
        let t0 = Instant::now();
        for i in 0..9 {
            l.fail("k", t0 + Duration::from_secs(i));
            assert!(l.locked("k", t0 + Duration::from_secs(i)).is_none());
        }
        l.fail("k", t0 + Duration::from_secs(9));
        let wait = l.locked("k", t0 + Duration::from_secs(10)).expect("locked");
        assert!(wait > 280 && wait <= 300);
        assert!(l.locked("other", t0).is_none());
        // lock ends after 5 minutes
        assert!(l.locked("k", t0 + Duration::from_secs(9 + 301)).is_none());
        // failures spread over more than a minute do not lock
        let l = Limiter::default();
        for i in 0..30 {
            l.fail("slow", t0 + Duration::from_secs(i * 7));
        }
        assert!(l.locked("slow", t0 + Duration::from_secs(30 * 7)).is_none());
        // success clears the counter
        let l = Limiter::default();
        for _ in 0..9 {
            l.fail("a", t0);
        }
        l.ok("a");
        l.fail("a", t0);
        assert!(l.locked("a", t0).is_none());
    }
}
