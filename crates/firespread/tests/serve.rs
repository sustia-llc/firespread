#![cfg(feature = "serve")]
//! The `serve` example's router: GET of a run or diagrams file returns its
//! bytes, a missing diagrams file returns 404, and a `..` path outside a mount
//! returns 404.

#[path = "../examples/serve.rs"]
#[allow(dead_code)]
mod serve_example;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use std::io::{Read, Write};

use serve_example::{ServeDirs, router};

const MANIFEST: &[u8] =
    br#"{"run_id":"test-run","outputs":{"burn_probability_h1":"burn_probability_h1.tif"}}"#;
const SECRET: &[u8] = b"[package]\nname = \"outside-the-mounts\"\n";

/// Temporary tree: `<root>/Cargo.toml` outside every mount, and the
/// `ui`, `run`, `basemap` and `diagrams` mounts below it.
struct Tree {
    root: PathBuf,
    tif: Vec<u8>,
}

impl Tree {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "firespread-serve-{tag}-{}-{nanos}",
            std::process::id()
        ));
        for d in ["ui", "run", "basemap", "diagrams"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        // Little-endian TIFF magic followed by bytes that are not valid UTF-8.
        let mut tif = b"II*\0".to_vec();
        tif.extend((0u8..=255).cycle().take(4096));
        std::fs::write(root.join("Cargo.toml"), SECRET).unwrap();
        std::fs::write(root.join("ui/index.html"), b"<!doctype html>").unwrap();
        std::fs::write(root.join("run/manifest.json"), MANIFEST).unwrap();
        std::fs::write(root.join("run/burn_probability_h1.tif"), &tif).unwrap();
        Self { root, tif }
    }

    fn dirs(&self) -> ServeDirs {
        ServeDirs {
            ui: self.root.join("ui"),
            run: self.root.join("run"),
            basemap: self.root.join("basemap"),
            diagrams: self.root.join("diagrams"),
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Response {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}

/// Serves `dirs` on 127.0.0.1 port 0 and returns the bound address.
async fn spawn(dirs: &ServeDirs) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(dirs);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Writes `request` verbatim (no client-side path normalisation) on a blocking
/// thread and returns the whole response.
async fn exchange(addr: SocketAddr, request: String) -> Vec<u8> {
    tokio::task::spawn_blocking(move || {
        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        raw
    })
    .await
    .unwrap()
}

/// Sends `GET <target>` and parses the response.
async fn get(addr: SocketAddr, target: &str) -> Response {
    let request = format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let raw = exchange(addr, request).await;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response has a header terminator");
    let head = std::str::from_utf8(&raw[..split]).unwrap();
    let body = raw[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap();
    let status: u16 = status_line.split(' ').nth(1).unwrap().parse().unwrap();
    let mut content_type = None;
    let mut content_length = None;
    for line in lines {
        let (name, value) = line.split_once(':').unwrap();
        match name.to_ascii_lowercase().as_str() {
            "content-type" => content_type = Some(value.trim().to_owned()),
            "content-length" => content_length = Some(value.trim().parse::<usize>().unwrap()),
            "transfer-encoding" => panic!("unexpected transfer-encoding `{value}` for {target}"),
            _ => {}
        }
    }
    if let Some(n) = content_length {
        assert_eq!(body.len(), n, "body length vs content-length for {target}");
    }
    Response {
        status,
        content_type,
        body,
    }
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[tokio::test]
async fn manifest_is_served_from_the_run_dir() {
    let tree = Tree::new("manifest");
    let addr = spawn(&tree.dirs()).await;
    let r = get(addr, "/run/manifest.json").await;
    assert_eq!(r.status, 200, "GET /run/manifest.json status");
    assert_eq!(r.body, read(&tree.root.join("run/manifest.json")));
    assert_eq!(r.content_type.as_deref(), Some("application/json"));
}

#[tokio::test]
async fn cog_is_served_byte_for_byte() {
    let tree = Tree::new("cog");
    let addr = spawn(&tree.dirs()).await;
    let r = get(addr, "/run/burn_probability_h1.tif").await;
    assert_eq!(r.status, 200, "GET /run/burn_probability_h1.tif status");
    assert_eq!(r.body, tree.tif, "tif bytes differ");
    assert_eq!(r.content_type.as_deref(), Some("image/tiff"));
}

#[tokio::test]
async fn ui_index_is_served_at_root() {
    let tree = Tree::new("ui");
    let addr = spawn(&tree.dirs()).await;
    let r = get(addr, "/").await;
    assert_eq!(r.status, 200, "GET / status");
    assert_eq!(r.body, b"<!doctype html>");
}

#[tokio::test]
async fn diagrams_are_served_at_diagrams() {
    let tree = Tree::new("diagrams");
    let png = b"\x89PNG\r\n\x1a\ndiagram".to_vec();
    std::fs::write(tree.root.join("diagrams/d1.png"), &png).unwrap();
    let addr = spawn(&tree.dirs()).await;
    let r = get(addr, "/diagrams/d1.png").await;
    assert_eq!(r.status, 200, "GET /diagrams/d1.png status");
    assert_eq!(r.body, png, "GET /diagrams/d1.png bytes differ");
    let r = get(addr, "/diagrams/missing.png").await;
    assert_eq!(
        r.status, 404,
        "GET /diagrams/missing.png returned {}",
        r.status
    );
}

#[tokio::test]
async fn parent_segments_return_404() {
    let tree = Tree::new("traversal");
    assert!(tree.root.join("Cargo.toml").is_file());
    let addr = spawn(&tree.dirs()).await;
    for target in [
        "/run/../Cargo.toml",
        "/run/%2e%2e/Cargo.toml",
        "/run/%2E%2E/Cargo.toml",
        "/run/..%2fCargo.toml",
        "/basemap/../Cargo.toml",
        "/diagrams/../Cargo.toml",
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
    ] {
        let r = get(addr, target).await;
        assert_eq!(r.status, 404, "GET {target} returned {}", r.status);
        assert_ne!(
            r.body, SECRET,
            "GET {target} leaked the file outside the mounts"
        );
    }
}

#[tokio::test]
async fn write_methods_are_not_served() {
    let tree = Tree::new("write");
    let addr = spawn(&tree.dirs()).await;
    let request = format!(
        "PUT /run/manifest.json HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    let raw = exchange(addr, request).await;
    let status_line = std::str::from_utf8(&raw[..12]).unwrap();
    assert_eq!(status_line, "HTTP/1.1 405", "PUT status line");
    assert_eq!(read(&tree.root.join("run/manifest.json")), MANIFEST);
}

/// Sends `<method> <target>` with a JSON body and returns the status and body.
async fn send_json(addr: SocketAddr, method: &str, target: &str, body: &str) -> (u16, Vec<u8>) {
    let request = format!(
        "{method} {target} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let raw = exchange(addr, request).await;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response has a header terminator");
    let status: u16 = std::str::from_utf8(&raw[9..12]).unwrap().parse().unwrap();
    (status, raw[split + 4..].to_vec())
}

fn alert_json(node: &str, lat: f64, bearing: f64) -> String {
    format!(
        r#"{{"node_id":"{node}","lat":{lat},"lon":-106.3,"bearing_deg":{bearing},"confidence":0.8,"label":"smoke","t":"2026-10-27T10:00:00Z","source":"sim"}}"#
    )
}

#[tokio::test]
async fn alerts_are_stored_paged_and_reset() {
    let tree = Tree::new("alerts");
    let addr = spawn(&tree.dirs()).await;
    let (status, body) =
        send_json(addr, "POST", "/api/alerts", &alert_json("a", 35.8, -10.0)).await;
    assert_eq!(status, 201);
    let stored: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(stored["seq"], 1);
    assert_eq!(stored["bearing_deg"], 350.0, "bearing wraps into [0, 360)");
    let (status, _) = send_json(addr, "POST", "/api/alerts", &alert_json("b", 35.9, 90.0)).await;
    assert_eq!(status, 201);

    let page: serde_json::Value =
        serde_json::from_slice(&get(addr, "/api/alerts?after=1").await.body).unwrap();
    assert_eq!(page["last_seq"], 2);
    let alerts = page["alerts"].as_array().unwrap();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0]["node_id"], "b");

    let (status, _) = send_json(addr, "DELETE", "/api/alerts", "").await;
    assert_eq!(status, 204);
    let page: serde_json::Value =
        serde_json::from_slice(&get(addr, "/api/alerts").await.body).unwrap();
    assert_eq!(page["alerts"].as_array().unwrap().len(), 0);
    assert_eq!(page["reset_seq"], 2);
}

#[tokio::test]
async fn invalid_alerts_are_rejected() {
    let tree = Tree::new("bad-alerts");
    let addr = spawn(&tree.dirs()).await;
    let (status, _) = send_json(addr, "POST", "/api/alerts", &alert_json("a", 95.0, 10.0)).await;
    assert_eq!(status, 422, "latitude out of range");
    let (status, _) = send_json(addr, "POST", "/api/alerts", &alert_json("", 35.0, 10.0)).await;
    assert_eq!(status, 422, "empty node id");
    let page: serde_json::Value =
        serde_json::from_slice(&get(addr, "/api/alerts").await.body).unwrap();
    assert_eq!(page["last_seq"], 0);
}
