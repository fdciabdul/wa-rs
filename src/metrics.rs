//! Prometheus text-exposition on `GET /metrics`.
//!
//! Bypasses the JWT middleware so a scraper can poll without a token; put
//! the endpoint behind a network ACL if that's a concern.
//!
//! Gauges:
//!
//! - `waxum_sessions_total` — number of session runtimes resident in
//!   memory.
//! - `waxum_sessions_live` — sessions whose upstream client agrees it's
//!   connected AND logged in (source of truth for /status).
//! - `waxum_session_socket_alive{session_id}` — per-session raw socket
//!   liveness (1/0), distinct from login state: catches the "limbo"
//!   where a cached `logged_in` outlives a dead socket.
//! - `waxum_process_threads` — thread count from `/proc/self/status`.
//! - `waxum_process_open_fds` — FD count from `/proc/self/fd`.
//! - `waxum_webhook_circuits_open` — webhook target URLs currently in the
//!   OPEN circuit-breaker state; alert when this is non-zero for long.
//!
//! Counters (per-session, labelled by `session_id`):
//!
//! - `waxum_session_socket_drops_total` — `Event::Disconnected` seen.
//! - `waxum_session_reconnects_total` — `Event::Connected` that closed
//!   an unplanned reconnect window (initial connects don't count).
//!
//! - `waxum_session_devices_unkeyed_total{session_id,reason}` — cumulative
//!   count, read straight from the client's own `StatsSnapshot` each
//!   scrape (so it's a gauge, not an incremented counter: whatsapp-rust
//!   owns the running total, waxum only republishes it), of devices a
//!   send gave up keying for. `reason` is one of `no_bundle`,
//!   `session_setup`, `rejected`, `fetch_failed`, `encrypt` — see
//!   whatsapp-rust's `agent_docs/observability.md` for what separates
//!   them. A send drops the device and keeps going when this fires, so
//!   this answers "how often", not "did the message fail".

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use prometheus::{Encoder, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder};
use std::sync::OnceLock;

use crate::state::AppState;

static REGISTRY: OnceLock<Registry> = OnceLock::new();
static SESSIONS_TOTAL: OnceLock<IntGauge> = OnceLock::new();
static SESSIONS_LIVE: OnceLock<IntGauge> = OnceLock::new();
static PROCESS_THREADS: OnceLock<IntGauge> = OnceLock::new();
static PROCESS_OPEN_FDS: OnceLock<IntGauge> = OnceLock::new();
static WEBHOOK_CIRCUITS_OPEN: OnceLock<IntGauge> = OnceLock::new();
static SESSION_SOCKET_ALIVE: OnceLock<IntGaugeVec> = OnceLock::new();
static SESSION_DROPS: OnceLock<IntCounterVec> = OnceLock::new();
static SESSION_RECONNECTS: OnceLock<IntCounterVec> = OnceLock::new();
static SESSION_DEVICES_UNKEYED: OnceLock<IntGaugeVec> = OnceLock::new();

/// Session ids we've already exported per-session series for, so the
/// scrape loop can prune labels for sessions that were deleted —
/// otherwise their last gauge value would linger forever.
static SEEN_SESSIONS: OnceLock<parking_lot::Mutex<std::collections::HashSet<String>>> =
    OnceLock::new();

fn seen_sessions() -> &'static parking_lot::Mutex<std::collections::HashSet<String>> {
    SEEN_SESSIONS.get_or_init(|| parking_lot::Mutex::new(std::collections::HashSet::new()))
}

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| {
        let r = Registry::new();
        let sessions_total = IntGauge::new(
            "waxum_sessions_total",
            "Total session runtimes resident in the gateway",
        )
        .unwrap();
        let sessions_live = IntGauge::new(
            "waxum_sessions_live",
            "Sessions whose underlying client reports connected + logged in",
        )
        .unwrap();
        let process_threads = IntGauge::new(
            "waxum_process_threads",
            "Thread count for the waxum process (from /proc/self/status)",
        )
        .unwrap();
        let process_open_fds = IntGauge::new(
            "waxum_process_open_fds",
            "Open file descriptor count for the waxum process",
        )
        .unwrap();
        let webhook_circuits_open = IntGauge::new(
            "waxum_webhook_circuits_open",
            "Webhook target URLs currently in open-circuit state (skipped)",
        )
        .unwrap();
        let session_socket_alive = IntGaugeVec::new(
            Opts::new(
                "waxum_session_socket_alive",
                "Per-session raw socket liveness (1 = transport connected, 0 = down)",
            ),
            &["session_id"],
        )
        .unwrap();
        let session_drops = IntCounterVec::new(
            Opts::new(
                "waxum_session_socket_drops_total",
                "Per-session count of socket drops (Event::Disconnected)",
            ),
            &["session_id"],
        )
        .unwrap();
        let session_reconnects = IntCounterVec::new(
            Opts::new(
                "waxum_session_reconnects_total",
                "Per-session count of successful reconnects closing an unplanned retry window",
            ),
            &["session_id"],
        )
        .unwrap();
        let session_devices_unkeyed = IntGaugeVec::new(
            Opts::new(
                "waxum_session_devices_unkeyed_total",
                "Per-session, per-reason count of devices a send gave up keying for (from the client's own running total)",
            ),
            &["session_id", "reason"],
        )
        .unwrap();
        r.register(Box::new(sessions_total.clone())).unwrap();
        r.register(Box::new(sessions_live.clone())).unwrap();
        r.register(Box::new(process_threads.clone())).unwrap();
        r.register(Box::new(process_open_fds.clone())).unwrap();
        r.register(Box::new(webhook_circuits_open.clone())).unwrap();
        r.register(Box::new(session_socket_alive.clone())).unwrap();
        r.register(Box::new(session_drops.clone())).unwrap();
        r.register(Box::new(session_reconnects.clone())).unwrap();
        r.register(Box::new(session_devices_unkeyed.clone()))
            .unwrap();
        SESSIONS_TOTAL.set(sessions_total).ok();
        SESSIONS_LIVE.set(sessions_live).ok();
        PROCESS_THREADS.set(process_threads).ok();
        PROCESS_OPEN_FDS.set(process_open_fds).ok();
        WEBHOOK_CIRCUITS_OPEN.set(webhook_circuits_open).ok();
        SESSION_SOCKET_ALIVE.set(session_socket_alive).ok();
        SESSION_DROPS.set(session_drops).ok();
        SESSION_RECONNECTS.set(session_reconnects).ok();
        SESSION_DEVICES_UNKEYED.set(session_devices_unkeyed).ok();
        r
    })
}

/// Bump the per-session socket-drop counter. Called from the
/// `Event::Disconnected` arm of the session event loop; a no-op before
/// the first /metrics scrape initialised the registry.
pub fn record_session_drop(session_id: &str) {
    if let Some(c) = SESSION_DROPS.get() {
        c.with_label_values(&[session_id]).inc();
    }
}

/// Bump the per-session successful-reconnect counter. Called from the
/// `Event::Connected` arm when the connect closed an unplanned retry
/// window, so initial connects don't inflate it.
pub fn record_session_reconnect(session_id: &str) {
    if let Some(c) = SESSION_RECONNECTS.get() {
        c.with_label_values(&[session_id]).inc();
    }
}

fn read_proc_threads() -> Option<i64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("Threads:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

fn read_proc_open_fds() -> Option<i64> {
    let entries = std::fs::read_dir("/proc/self/fd").ok()?;
    Some(entries.count() as i64)
}

pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let reg = registry();

    let mut total = 0i64;
    let mut live = 0i64;
    let socket_alive_vec = SESSION_SOCKET_ALIVE.get().unwrap();
    let devices_unkeyed_vec = SESSION_DEVICES_UNKEYED.get().unwrap();
    let mut current_ids = std::collections::HashSet::new();
    for (session_id, s) in state.session_iter_with_ids() {
        total += 1;
        if s.is_alive() {
            live += 1;
        }
        socket_alive_vec
            .with_label_values(&[session_id.as_str()])
            .set(if s.socket_alive() { 1 } else { 0 });
        if let Some(client) = s.get_client() {
            let stats = client.stats();
            for (reason, value) in [
                ("no_bundle", stats.devices_unkeyed_no_bundle),
                ("session_setup", stats.devices_unkeyed_session_setup),
                ("rejected", stats.devices_unkeyed_rejected),
                ("fetch_failed", stats.devices_unkeyed_fetch_failed),
                ("encrypt", stats.devices_unkeyed_encrypt),
            ] {
                devices_unkeyed_vec
                    .with_label_values(&[session_id.as_str(), reason])
                    .set(value as i64);
            }
        }
        current_ids.insert(session_id);
    }
    SESSIONS_TOTAL.get().unwrap().set(total);
    SESSIONS_LIVE.get().unwrap().set(live);

    let mut seen = seen_sessions().lock();
    for stale in seen.difference(&current_ids).cloned().collect::<Vec<_>>() {
        let _ = socket_alive_vec.remove_label_values(&[stale.as_str()]);
        if let Some(c) = SESSION_DROPS.get() {
            let _ = c.remove_label_values(&[stale.as_str()]);
        }
        if let Some(c) = SESSION_RECONNECTS.get() {
            let _ = c.remove_label_values(&[stale.as_str()]);
        }
        for reason in [
            "no_bundle",
            "session_setup",
            "rejected",
            "fetch_failed",
            "encrypt",
        ] {
            let _ = devices_unkeyed_vec.remove_label_values(&[stale.as_str(), reason]);
        }
    }
    *seen = current_ids;
    drop(seen);

    if let Some(t) = read_proc_threads() {
        PROCESS_THREADS.get().unwrap().set(t);
    }
    if let Some(f) = read_proc_open_fds() {
        PROCESS_OPEN_FDS.get().unwrap().set(f);
    }
    WEBHOOK_CIRCUITS_OPEN
        .get()
        .unwrap()
        .set(state.webhook_circuits_open_count() as i64);

    let encoder = TextEncoder::new();
    let metric_families = reg.gather();
    let mut buf = Vec::new();
    match encoder.encode(&metric_families, &mut buf) {
        Ok(()) => (
            StatusCode::OK,
            [("Content-Type", encoder.format_type().to_string())],
            buf,
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
