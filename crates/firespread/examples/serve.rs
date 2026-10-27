#![forbid(unsafe_code)]
//! Local http server for the static pages. It binds 127.0.0.1 unless
//! `--bind` names another address, and serves four directories with
//! `ServeDir`: `/` from the ui directory, `/run/` from a run directory,
//! `/basemap/` from a basemap directory and `/diagrams/` from a diagrams
//! directory. `/api/alerts` keeps the smoke alerts that detection nodes post,
//! in memory only, for the detection dashboard (`detect.html`).
//!
//! Usage: `serve --run <dir> [--ui <dir>] [--basemap <dir>] [--diagrams <dir>]
//! [--bind <ip>] [--port <u16>]`. The defaults are `--ui ui`, `--basemap
//! data/basemap`, `--diagrams docs/diagrams`, `--bind 127.0.0.1` and `--port
//! 0`; port 0 takes a free port. `--bind 0.0.0.0` lets other laptops and
//! phones on the network reach the server. The bound URL is printed on
//! stdout.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;

/// Alerts kept in memory; the oldest is dropped past this count.
const ALERT_CAPACITY: usize = 5000;

/// One smoke alert as a node posts it.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlertIn {
    /// Node identifier, 1 to 64 characters.
    pub node_id: String,
    /// Node latitude in degrees (WGS84).
    pub lat: f64,
    /// Node longitude in degrees (WGS84).
    pub lon: f64,
    /// Bearing from the node to the detection, degrees clockwise from true
    /// north, stored in [0, 360).
    pub bearing_deg: f64,
    /// Detector confidence in [0, 1].
    pub confidence: f64,
    /// Detection class, such as `smoke` or `fire`, 1 to 32 characters.
    pub label: String,
    /// Detection time as the node reports it, RFC 3339 UTC.
    pub t: String,
    /// Where the detection came from: `yolo`, `replay`, `phone`.
    #[serde(default)]
    pub source: Option<String>,
}

/// A stored alert: the posted fields plus a sequence number and the server's
/// receive time in Unix milliseconds.
#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub seq: u64,
    pub received_ms: u64,
    #[serde(flatten)]
    pub alert: AlertIn,
}

/// The in-memory alert log. Sequence numbers increase across resets, and
/// `reset_seq` is the last sequence number a reset removed.
#[derive(Debug, Default)]
pub struct AlertLog {
    next_seq: u64,
    reset_seq: u64,
    alerts: VecDeque<Alert>,
}

type SharedLog = Arc<Mutex<AlertLog>>;

#[derive(Debug, Deserialize)]
struct AlertsQuery {
    #[serde(default)]
    after: u64,
}

#[derive(Debug, Serialize)]
struct AlertsPage {
    alerts: Vec<Alert>,
    last_seq: u64,
    reset_seq: u64,
    /// The server's clock in Unix milliseconds, so a client can age alerts
    /// by `now_ms - received_ms` without trusting its own clock.
    now_ms: u64,
}

fn validate(mut a: AlertIn) -> Result<AlertIn, String> {
    if a.node_id.is_empty() || a.node_id.chars().count() > 64 {
        return Err("node_id must have 1 to 64 characters".into());
    }
    if a.label.is_empty() || a.label.chars().count() > 32 {
        return Err("label must have 1 to 32 characters".into());
    }
    if !(-90.0..=90.0).contains(&a.lat) || !(-180.0..=180.0).contains(&a.lon) {
        return Err("lat or lon out of range".into());
    }
    if !a.bearing_deg.is_finite() {
        return Err("bearing_deg must be finite".into());
    }
    if !(0.0..=1.0).contains(&a.confidence) {
        return Err("confidence must lie in [0, 1]".into());
    }
    if a.t.len() > 40 || a.source.as_ref().is_some_and(|s| s.len() > 32) {
        return Err("t or source too long".into());
    }
    a.bearing_deg = a.bearing_deg.rem_euclid(360.0);
    Ok(a)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

async fn post_alert(
    State(log): State<SharedLog>,
    Json(alert): Json<AlertIn>,
) -> Result<(StatusCode, Json<Alert>), (StatusCode, String)> {
    let alert = validate(alert).map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))?;
    let mut log = log.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "alert log poisoned".into(),
        )
    })?;
    log.next_seq += 1;
    let stored = Alert {
        seq: log.next_seq,
        received_ms: now_ms(),
        alert,
    };
    if log.alerts.len() == ALERT_CAPACITY {
        log.alerts.pop_front();
    }
    log.alerts.push_back(stored.clone());
    Ok((StatusCode::CREATED, Json(stored)))
}

async fn get_alerts(
    State(log): State<SharedLog>,
    Query(q): Query<AlertsQuery>,
) -> Result<Json<AlertsPage>, (StatusCode, String)> {
    let log = log.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "alert log poisoned".into(),
        )
    })?;
    Ok(Json(AlertsPage {
        alerts: log
            .alerts
            .iter()
            .filter(|a| a.seq > q.after)
            .cloned()
            .collect(),
        last_seq: log.next_seq,
        reset_seq: log.reset_seq,
        now_ms: now_ms(),
    }))
}

async fn reset_alerts(State(log): State<SharedLog>) -> Result<StatusCode, (StatusCode, String)> {
    let mut log = log.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "alert log poisoned".into(),
        )
    })?;
    log.alerts.clear();
    log.reset_seq = log.next_seq;
    Ok(StatusCode::NO_CONTENT)
}

/// Directories served by [`router`].
#[derive(Debug, Clone)]
pub struct ServeDirs {
    /// Served at `/`.
    pub ui: PathBuf,
    /// Served at `/run/`.
    pub run: PathBuf,
    /// Served at `/basemap/`.
    pub basemap: PathBuf,
    /// Served at `/diagrams/`.
    pub diagrams: PathBuf,
}

/// Router serving `dirs.run` at `/run/`, `dirs.basemap` at `/basemap/`,
/// `dirs.diagrams` at `/diagrams/`, the alert API at `/api/alerts` (POST one
/// alert, GET those after `?after=<seq>`, DELETE to reset) and `dirs.ui` for
/// every other path. Every directory route is a `ServeDir`, which answers GET
/// and HEAD and returns 404 for a missing file or a path with a `..` segment.
pub fn router(dirs: &ServeDirs) -> Router {
    let log: SharedLog = Arc::default();
    Router::new()
        .route(
            "/api/alerts",
            get(get_alerts).post(post_alert).delete(reset_alerts),
        )
        .with_state(log)
        .nest_service("/run", ServeDir::new(&dirs.run))
        .nest_service("/basemap", ServeDir::new(&dirs.basemap))
        .nest_service("/diagrams", ServeDir::new(&dirs.diagrams))
        .fallback_service(ServeDir::new(&dirs.ui))
}

/// Parsed command-line arguments.
#[derive(Debug)]
struct Args {
    dirs: ServeDirs,
    bind: IpAddr,
    port: u16,
}

fn parse_args(mut it: impl Iterator<Item = String>) -> anyhow::Result<Args> {
    let mut run: Option<PathBuf> = None;
    let mut ui = PathBuf::from("ui");
    let mut basemap = PathBuf::from("data/basemap");
    let mut diagrams = PathBuf::from("docs/diagrams");
    let mut bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut port: u16 = 0;
    while let Some(flag) = it.next() {
        let value = it
            .next()
            .with_context(|| format!("argument `{flag}` needs a value"))?;
        match flag.as_str() {
            "--run" => run = Some(PathBuf::from(value)),
            "--ui" => ui = PathBuf::from(value),
            "--basemap" => basemap = PathBuf::from(value),
            "--diagrams" => diagrams = PathBuf::from(value),
            "--bind" => {
                bind = value.parse().with_context(|| {
                    format!("argument `--bind`: `{value}` is not an IP address")
                })?;
            }
            "--port" => {
                port = value
                    .parse()
                    .with_context(|| format!("argument `--port`: `{value}` is not a u16"))?;
            }
            other => bail!("unknown argument `{other}`"),
        }
    }
    let run = run.context("argument `--run` is required")?;
    for (name, dir) in [("--run", &run), ("--ui", &ui)] {
        if !Path::new(dir).is_dir() {
            bail!("argument `{name}`: `{}` is not a directory", dir.display());
        }
    }
    for (name, route, dir) in [
        ("basemap", "/basemap/", &basemap),
        ("diagrams", "/diagrams/", &diagrams),
    ] {
        if !dir.is_dir() {
            eprintln!(
                "note: {name} directory `{}` does not exist; {route} will return 404",
                dir.display()
            );
        }
    }
    Ok(Args {
        dirs: ServeDirs {
            ui,
            run,
            basemap,
            diagrams,
        },
        bind,
        port,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    let addr = SocketAddr::from((args.bind, args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    let bound = listener.local_addr().context("local address")?;
    println!("http://{bound}/");
    if bound.ip().is_unspecified() {
        eprintln!(
            "note: listening on every interface; other devices use this machine's LAN address and port {}",
            bound.port()
        );
    }
    axum::serve(listener, router(&args.dirs))
        .with_graceful_shutdown(async {
            // A failed signal handler install ends the server, as Ctrl-C would.
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("serve")?;
    Ok(())
}
