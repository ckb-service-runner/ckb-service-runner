//! Node identity (D6 A, D12 A): the proof is posted with both signatures, the monitor's node is added,
//! and the P2P key never shows up in an API answer or the log.

mod mock_monitor;
use mock_monitor::*;
use serde_json::{json, Value};
use std::io::Write;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// Peer ID of the P2P key `[7; 32]` (acceptance vector).
const PEER_7: &str = "QmS7sXpV62pY1jHiAkiVkftGxJvimJZTLdLuzerj7yafdS";
const MONITOR_PEER: &str = "Qmecbv6CRqPqSyZUvQ8SQErHYAbq4f2Pucq8ofs2MsdZnG";

type Calls = Arc<Mutex<Vec<(String, Value)>>>;

/// Serves `f(path, body) -> (status, body)` on a fresh port.
fn serve(f: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let f = Arc::new(f);
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { return };
            let f = f.clone();
            std::thread::spawn(move || {
                let _ = c.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                while let Some((_, path, body)) = read_request(&mut c) {
                    let (st, out) = f(&path, &body);
                    if write!(c, "HTTP/1.1 {st} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{out}", out.len()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    port
}

/// Mock CKB node reporting `node_id`; records Net calls. `net` = false answers them "method not found".
fn node(node_id: &'static str, net: bool, calls: Calls) -> u16 {
    serve(move |_, body| {
        let v: Value = serde_json::from_str(body).unwrap_or_default();
        let method = v["method"].as_str().unwrap_or("").to_string();
        let result = match method.as_str() {
            "get_blockchain_info" => json!({"chain": "ckb_testnet", "is_initial_block_download": false}),
            "sync_state" => json!({"ibd": false, "best_known_block_number": "0x3e8"}),
            "get_tip_header" => json!({"number": "0x3e8", "hash": "0xab", "epoch": "0x1"}),
            "local_node_info" => json!({"version": "0.210.0", "node_id": node_id}),
            "get_peers" => json!([{"is_outbound": true, "node_id": "QmSomebodyElse"}]),
            "get_indexer_tip" => json!({"block_number": "0x3e8"}),
            "tx_pool_info" => json!({"pending": "0x1", "proposed": "0x2"}),
            "add_node" => Value::Null,
            _ => Value::Null,
        };
        if method == "add_node" || method == "remove_node" || method.ends_with("_ban") {
            calls.lock().unwrap().push((method.clone(), v["params"].clone()));
            if !net {
                return (200, json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32601, "message": "Method not found"}}).to_string());
            }
        }
        (200, json!({"jsonrpc": "2.0", "id": 1, "result": result}).to_string())
    })
}

/// Mock monitor: `/v1/identity` records the proof, `/v1/monitor` publishes `p2p_address`.
fn monitor(proofs: Calls) -> u16 {
    serve(move |path, body| match path {
        "/v1/identity" => {
            proofs.lock().unwrap().push((path.to_string(), serde_json::from_str(body).unwrap()));
            (200, r#"{"ok":true}"#.into())
        }
        "/v1/monitor" => (200, format!(r#"{{"p2p_address":"/ip4/203.0.113.7/tcp/8115/p2p/{MONITOR_PEER}"}}"#)),
        "/v1/network" => (200, r#"{"nodes_total":1,"nodes_reachable":1,"updated_at":1}"#.into()),
        _ => (404, r#"{"error":"not_found"}"#.into()),
    })
}

struct Csr {
    child: Child,
    dash: u16,
    dir: tempfile::TempDir,
}
impl Drop for Csr {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn setup(node_port: u16, monitor_port: u16, key_mode: u32) -> (tempfile::TempDir, std::path::PathBuf, u16) {
    let dash = free_port();
    let dir = tempfile::Builder::new().tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let key = dir.path().join("secret_key");
    std::fs::write(&key, [7u8; 32]).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(key_mode)).unwrap();
    let cfg = dir.path().join("csr.toml");
    std::fs::write(
        &cfg,
        format!(
            "[node]\nrpc_url = \"http://127.0.0.1:{node_port}\"\np2p_key_file = \"{}\"\n[dashboard]\nbind = \"127.0.0.1:{dash}\"\n[ranking]\njoin = true\nping_mode = \"outbound\"\n[monitor]\nurl = \"http://127.0.0.1:{monitor_port}\"\npubkey = \"{}\"\n",
            key.display(),
            monitor_pubkey()
        ),
    )
    .unwrap();
    (dir, cfg, dash)
}

fn start(node_port: u16, monitor_port: u16, key_mode: u32) -> Csr {
    let (dir, cfg, dash) = setup(node_port, monitor_port, key_mode);
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
    Csr { child, dash, dir }
}

fn get(c: &Csr, path: &str) -> Value {
    let (st, body) = http(c.dash, "GET", path, "").unwrap();
    assert_eq!(st, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn assert_no_key(text: &str) {
    let hex7 = hex::encode([7u8; 32]);
    assert!(!text.contains(&hex7), "the P2P key leaked: {text}");
}

#[test]
fn proven_key_posts_the_proof_and_adds_the_monitor_node() {
    let (proofs, net): (Calls, Calls) = Default::default();
    let c = start(node(PEER_7, true, net.clone()), monitor(proofs.clone()), 0o640);
    assert!(wait_for(15, || !proofs.lock().unwrap().is_empty()), "no proof posted");
    let p = proofs.lock().unwrap()[0].1.clone();
    assert_eq!(p["peer_id"], PEER_7);
    assert_eq!(p["network"], "testnet");
    let msg = format!(
        "CSR-NODE-IDENTITY-v1\n{PEER_7}\n{}\ntestnet\n{}",
        p["service_pubkey"].as_str().unwrap(),
        p["ts"]
    );
    assert!(verify(p["p2p_pubkey"].as_str().unwrap(), &msg, p["p2p_sig"].as_str().unwrap()));
    assert!(verify(p["service_pubkey"].as_str().unwrap(), &msg, p["service_sig"].as_str().unwrap()));
    assert!((p["ts"].as_i64().unwrap() - now()).abs() < 30);
    assert_eq!(p.as_object().unwrap().len(), 7);

    assert!(wait_for(15, || !net.lock().unwrap().is_empty()), "add_node not called");
    let calls = net.lock().unwrap().clone();
    assert_eq!(calls[0], ("add_node".to_string(), json!([MONITOR_PEER, "/ip4/203.0.113.7/tcp/8115"])));
    assert!(calls.iter().all(|(m, _)| m == "add_node"));

    let n = get(&c, "/api/node");
    assert_eq!(n["identity"]["state"], "proven");
    assert!(n["net_rpc_hint"].is_null());
    assert!(wait_for(10, || get(&c, "/api/network")["identity"]["proofs"][0]["result"] == "accepted"));

    for path in ["/api/node", "/api/network", "/api/status", "/api/settings"] {
        assert_no_key(&http(c.dash, "GET", path, "").unwrap().1);
    }
    let log = std::fs::read_to_string(c.dir.path().join("testnet/logs/csr.log")).unwrap_or_default();
    assert_no_key(&log);
    // The key file is untouched.
    let m = std::fs::metadata(c.dir.path().join("secret_key")).unwrap();
    assert_eq!(m.permissions().mode() & 0o777, 0o640);
}

#[test]
fn mismatch_sends_no_proof_and_net_off_is_shown() {
    let (proofs, net): (Calls, Calls) = Default::default();
    let c = start(node("QmMock", false, net.clone()), monitor(proofs.clone()), 0o600);
    assert!(wait_for(15, || !net.lock().unwrap().is_empty()), "add_node not called");
    assert!(wait_for(10, || get(&c, "/api/node")["net_rpc_hint"].as_str().is_some_and(|h| h.contains("Net"))));
    let n = get(&c, "/api/node");
    assert_eq!(n["identity"]["state"], "mismatch");
    assert!(n["identity"]["message"].as_str().unwrap().contains("QmMock"));
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(proofs.lock().unwrap().is_empty(), "a proof was sent on a mismatch");
}

#[test]
fn world_readable_key_is_refused() {
    let (proofs, net): (Calls, Calls) = Default::default();
    let c = start(node(PEER_7, true, net), monitor(proofs.clone()), 0o644);
    let n = get(&c, "/api/node");
    assert_eq!(n["identity"]["state"], "unreadable");
    assert!(n["identity"]["message"].as_str().unwrap().contains("readable by every user"));
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(proofs.lock().unwrap().is_empty());
    let m = std::fs::metadata(c.dir.path().join("secret_key")).unwrap();
    assert_eq!(m.permissions().mode() & 0o777, 0o644);
}

#[test]
fn check_prints_the_identity_state() {
    let (proofs, net): (Calls, Calls) = Default::default();
    let (dir, cfg, _) = setup(node(PEER_7, true, net), monitor(proofs), 0o600);
    let run = |json: bool| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_csr"));
        cmd.arg("--data-root").arg(dir.path()).arg("--config").arg(&cfg).arg("check");
        if json {
            cmd.arg("--json");
        }
        String::from_utf8(cmd.output().unwrap().stdout).unwrap()
    };
    let human = run(false);
    assert!(human.contains("node identity: proven"), "{human}");
    let j: Value = serde_json::from_str(run(true).trim()).unwrap();
    assert_eq!(j["node_identity"]["state"], "proven");
    assert_no_key(&human);
}
