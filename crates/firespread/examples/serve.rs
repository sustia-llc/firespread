#![forbid(unsafe_code)]
//! Read-only local http server for the static page. It binds 127.0.0.1 and
//! serves four directories with `ServeDir`: `/` from the ui directory,
//! `/run/` from a run directory, `/basemap/` from a basemap directory and
//! `/diagrams/` from a diagrams directory.
//!
//! Usage: `serve --run <dir> [--ui <dir>] [--basemap <dir>] [--diagrams <dir>]
//! [--port <u16>]`. The defaults are `--ui ui`, `--basemap data/basemap`,
//! `--diagrams docs/diagrams` and `--port 0`; port 0 takes a free port. The
//! bound URL is printed on stdout.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use axum::Router;
use tower_http::services::ServeDir;

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
/// `dirs.diagrams` at `/diagrams/` and `dirs.ui` for every other path. Every
/// route is a `ServeDir`, which answers GET and HEAD and returns 404 for a
/// missing file or a path with a `..` segment.
pub fn router(dirs: &ServeDirs) -> Router {
    Router::new()
        .nest_service("/run", ServeDir::new(&dirs.run))
        .nest_service("/basemap", ServeDir::new(&dirs.basemap))
        .nest_service("/diagrams", ServeDir::new(&dirs.diagrams))
        .fallback_service(ServeDir::new(&dirs.ui))
}

/// Parsed command-line arguments.
#[derive(Debug)]
struct Args {
    dirs: ServeDirs,
    port: u16,
}

fn parse_args(mut it: impl Iterator<Item = String>) -> anyhow::Result<Args> {
    let mut run: Option<PathBuf> = None;
    let mut ui = PathBuf::from("ui");
    let mut basemap = PathBuf::from("data/basemap");
    let mut diagrams = PathBuf::from("docs/diagrams");
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
        port,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    let bound = listener.local_addr().context("local address")?;
    println!("http://{bound}/");
    axum::serve(listener, router(&args.dirs))
        .with_graceful_shutdown(async {
            // A failed signal handler install ends the server, as Ctrl-C would.
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("serve")?;
    Ok(())
}
