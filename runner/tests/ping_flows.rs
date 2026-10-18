//! Inbound and outbound ping flows against the Rust mock monitor and a mock node.

mod mock_monitor;
use mock_monitor::*;
use std::process::{Child, Command, Stdio};

struct Csr {
    child: Child,
    dash: u16,
    ping: u16,
    _dir: tempfile::TempDir,
}
impl Drop for Csr {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_csr(mode: &str, join: bool, monitor_url: &str) -> Csr {
    let node = start_mock_node();
    let (dash, ping) = (free_port(), free_port());
    let dir = tempfile::Builder::new()
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let cfg = dir.path().join("csr.toml");
    std::fs::write(
        &cfg,
        format!(
            "[node]\nrpc_url = \"http://127.0.0.1:{node}\"\n[dashboard]\nbind = \"127.0.0.1:{dash}\"\n[ranking]\njoin = {join}\nping_mode = \"{mode}\"\nping_bind = \"127.0.0.1:{ping}\"\n[monitor]\nurl = \"{monitor_url}\"\npubkey = \"{}\"\n",
            monitor_pubkey()
        ),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_csr"))
        .args(["--data-root"])
        .arg(dir.path())
        .arg("--config")
        .arg(&cfg)
        .arg("run")
        .env("CSR_TEST_FAST_TICK", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let csr = Csr {
        child,
        dash,
        ping,
        _dir: dir,
    };
    assert!(
        wait_for(20, || http(dash, "GET", "/healthz", "").is_some()),
        "csr did not start"
    );
    csr
}

#[test]
fn outbound_answers_valid_challenge() {
    let m = MockMonitor::start(monitor_key());
    let csr = start_csr("outbound", true, &m.url());
    assert!(wait_for(20, || !m.rec.lock().unwrap().answers.is_empty()));
    let rec = m.rec.lock().unwrap();
    let ci = &rec.checkins[0];
    let pk = ci["service_pubkey"].as_str().unwrap();
    assert_eq!(ci["peer_id"], "QmMock");
    assert_eq!(ci["network"], "testnet");
    assert!(verify(
        pk,
        &format!("CSR-BINDING-v1\nQmMock\n{pk}\ntestnet"),
        ci["binding_sig"].as_str().unwrap()
    ));
    let a = &rec.answers[0];
    assert_eq!(a["answer"]["ping_mode"], "outbound");
    assert_eq!(a["answer"]["block_hash_at_height"], "0xcd");
    assert!(verify(
        pk,
        &answer_message(&a["answer"]),
        a["sig"].as_str().unwrap()
    ));
    drop(rec);
    assert!(
        http(csr.ping, "POST", "/v1/challenge", "{}").is_none(),
        "ping port must stay closed in outbound mode"
    );
    assert!(wait_for(10, || {
        http(csr.dash, "GET", "/api/status", "")
            .is_some_and(|(_, b)| b.contains("\"last_check_ok_at\":1"))
    }));
}

#[test]
fn both_mode_serves_inbound_too() {
    let m = MockMonitor::start(monitor_key());
    let csr = start_csr("both", true, &m.url());
    assert!(wait_for(20, || !m.rec.lock().unwrap().answers.is_empty()));
    let ch = make_challenge(&monitor_key(), 0x77, 900, now());
    let (status, body) = http(csr.ping, "POST", "/v1/challenge", &ch).unwrap();
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"ping_mode\":\"both\""));
}

#[test]
fn invalid_challenge_is_not_answered() {
    let other = secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
    let m = MockMonitor::start(other);
    let _csr = start_csr("outbound", true, &m.url());
    assert!(wait_for(20, || m.rec.lock().unwrap().checkins.len() >= 2));
    assert!(m.rec.lock().unwrap().answers.is_empty());
}

#[test]
fn no_checkins_without_join() {
    let m = MockMonitor::start(monitor_key());
    let _csr = start_csr("outbound", false, &m.url());
    std::thread::sleep(std::time::Duration::from_secs(5));
    assert!(m.rec.lock().unwrap().checkins.is_empty());
}
