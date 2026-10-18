//! `GET /api/network` against the Rust public-API mock and a mock node.

mod mock_monitor;
use mock_monitor::*;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;

struct Csr {
    child: Child,
    dash: u16,
    _dir: tempfile::TempDir,
}
impl Drop for Csr {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_csr(join: bool, monitor_url: &str) -> Csr {
    let node = start_mock_node();
    let dash = free_port();
    let dir = tempfile::Builder::new()
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let cfg = dir.path().join("csr.toml");
    std::fs::write(
        &cfg,
        format!(
            "[node]\nrpc_url = \"http://127.0.0.1:{node}\"\n[dashboard]\nbind = \"127.0.0.1:{dash}\"\n[ranking]\njoin = {join}\nping_mode = \"outbound\"\n[monitor]\nurl = \"{monitor_url}\"\npubkey = \"{}\"\n",
            monitor_pubkey()
        ),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_csr"))
        .arg("--data-root")
        .arg(dir.path())
        .arg("--config")
        .arg(&cfg)
        .arg("run")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(wait_for(20, || http(dash, "GET", "/healthz", "").is_some()));
    Csr {
        child,
        dash,
        _dir: dir,
    }
}

fn network(c: &Csr) -> serde_json::Value {
    let (st, body) = http(c.dash, "GET", "/api/network", "").unwrap();
    assert_eq!(st, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

#[test]
fn not_joined_does_not_contact_monitor() {
    let m = PublicMonitor::start(&["QmMock"]);
    let c = start_csr(false, &m.url());
    let j = network(&c);
    assert_eq!(j["join"], false);
    assert!(j["message"].as_str().unwrap().contains("Join"));
    assert!(m.hits.lock().unwrap().is_empty());
}

#[test]
fn ranked_and_cached() {
    let m = PublicMonitor::start(&["QmMock"]);
    let c = start_csr(true, &m.url());
    let j = network(&c);
    assert_eq!(j["join"], true);
    assert_eq!(j["stale"], false);
    assert!(j["error"].is_null());
    assert_eq!(j["network"]["nodes_total"], 42);
    assert_eq!(j["network"]["countries"][0]["code"], "DE");
    assert_eq!(j["node"]["rank"], 7);
    assert_eq!(j["node"]["availability"]["d30"], 0.97);
    assert_eq!(j["node"]["attestation"]["status"], "valid");
    assert_eq!(j["ping"]["mode"], "outbound");
    network(&c);
    network(&c);
    assert_eq!(m.hit("/v1/network"), 1);
    assert_eq!(m.hit("/v1/nodes"), 1);
}

#[test]
fn not_ranked_gives_null_node() {
    let m = PublicMonitor::start(&[]);
    let c = start_csr(true, &m.url());
    let j = network(&c);
    assert!(j["node"].is_null());
    assert_eq!(j["network"]["nodes_reachable"], 30);
    assert!(j["error"].is_null());
}

#[test]
fn monitor_down_without_cache() {
    let m = PublicMonitor::start(&["QmMock"]);
    m.down.store(true, Ordering::SeqCst);
    let c = start_csr(true, &m.url());
    let j = network(&c);
    assert!(j["network"].is_null() && j["node"].is_null());
    assert!(!j["error"].as_str().unwrap().is_empty());
}

#[test]
fn unreachable_monitor_without_cache() {
    let c = start_csr(true, &format!("http://127.0.0.1:{}", free_port()));
    let j = network(&c);
    assert!(j["network"].is_null());
    assert!(!j["error"].as_str().unwrap().is_empty());
}
