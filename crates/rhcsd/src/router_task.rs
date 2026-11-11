//! Polls the ZTE router. Survives the router being unplugged, rebooted or
//! moved: it keeps retrying every 5 s and resumes (re-logging in) by itself.

use crate::state::{now_ms, Sample, Shared, HISTORY_MAX};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;
use zte::ZteClient;

pub async fn run(s: Shared) {
    let new_client = |s: &Shared| ZteClient::new(s.cfg.router_ip.clone(), s.cfg.router_password.clone());
    let mut client = new_client(&s);
    let mut cell: Option<(u32, u32, u16)> = None;
    let mut online: Option<bool> = None;
    let mut last_problem: Option<String> = None;
    let host = s.cfg.router_ip.clone();

    loop {
        if s.relogin.swap(false, Ordering::SeqCst) {
            client = new_client(&s);
            last_problem = None;
            s.event("login", "router session reset; logging in again", json!({})).await;
        }

        match client.telemetry().await {
            Ok(t) => {
                let now = now_ms();
                if online != Some(true) {
                    s.event("router_online", format!("router {host} reachable"), json!({})).await;
                }
                online = Some(true);

                let problem = client.login_problem().map(str::to_string);
                if problem != last_problem {
                    if let Some(p) = &problem {
                        s.event("error", format!("router login: {p}"), json!({})).await;
                    }
                    last_problem = problem.clone();
                }

                if let (Some(c), Some(p), Some(b)) = (t.cell_id, t.pci, t.band) {
                    let now_cell = (c, p, b);
                    if let Some(prev) = cell.filter(|x| *x != now_cell) {
                        s.event(
                            "handover",
                            format!(
                                "PCI {}→{} · eNB {}→{} · B{}→B{}",
                                prev.1, p, prev.0 >> 8, c >> 8, prev.2, b
                            ),
                            json!({
                                "from": { "pci": prev.1, "cell_id": prev.0, "band": prev.2 },
                                "to": { "pci": p, "cell_id": c, "band": b }
                            }),
                        )
                        .await;
                    }
                    cell = Some(now_cell);
                }

                {
                    let mut i = s.inner.write().await;
                    i.history.push_back(Sample::from(now, &t));
                    while i.history.len() > HISTORY_MAX {
                        i.history.pop_front();
                    }
                    i.sources.router.online = true;
                    i.sources.router.logged_in = t.logged_in;
                    i.sources.router.error = problem;
                    i.sources.router.last_ok = Some(now);
                    i.telemetry = Some(t.clone());
                }
                s.send(json!({ "type": "telemetry", "ts": now, "data": t }));
            }
            Err(e) => {
                if online != Some(false) {
                    s.event("disconnect", format!("router {host} unreachable: {e}"), json!({})).await;
                }
                online = Some(false);
                let mut i = s.inner.write().await;
                i.sources.router.online = false;
                i.sources.router.error = Some(e.to_string());
            }
        }
        s.send_status().await;

        let wait = if online == Some(true) { s.inner.read().await.interval_s } else { 5.0 };
        tokio::time::sleep(Duration::from_secs_f64(wait)).await;
    }
}
