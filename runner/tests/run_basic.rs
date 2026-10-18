//! Starts `csr run` against a mock JSON-RPC node on an OS-chosen port.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn reply(method: &str) -> String {
    let result = match method {
        "get_blockchain_info" => r#"{"chain":"ckb_testnet","is_initial_block_download":false}"#,
        "sync_state" => r#"{"ibd":false,"best_known_block_number":"0x3e8"}"#,
        "get_tip_header" => r#"{"number":"0x3e8","hash":"0xab","epoch":"0x1"}"#,
        "local_node_info" => r#"{"version":"0.210.0","node_id":"QmMock"}"#,
        "get_peers" => r#"[{"is_outbound":true}]"#,
        "get_indexer_tip" => r#"{"block_number":"0x3e8"}"#,
        "tx_pool_info" => r#"{"pending":"0x1","proposed":"0x2"}"#,
        _ => "null",
    };
    format!(r#"{{"jsonrpc":"2.0","id":1,"result":{result}}}"#)
}

fn serve_mock(l: TcpListener) {
    for conn in l.incoming() {
        let Ok(mut c) = conn else { return };
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            while let Ok(n) = c.read(&mut tmp) {
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                let Some(split) = text.find("\r\n\r\n") else {
                    continue;
                };
                let len = text[..split]
                    .to_ascii_lowercase()
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if buf.len() < split + 4 + len {
                    continue;
                }
                let body = &text[split + 4..];
                let method = body
                    .split("\"method\":\"")
                    .nth(1)
                    .and_then(|r| r.split('"').next())
                    .unwrap_or("");
                let out = reply(method);
                let _ = write!(
                    c,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                    out.len(),
                    out
                );
                buf.clear();
            }
        });
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn get(port: u16, path: &str) -> Option<String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n"
    )
    .ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn run_serves_status_and_creates_keys() {
    let rpc = TcpListener::bind("127.0.0.1:0").unwrap();
    let rpc_port = rpc.local_addr().unwrap().port();
    std::thread::spawn(move || serve_mock(rpc));
    let dash = free_port();
    let dir = tempfile::Builder::new()
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let cfg = dir.path().join("csr.toml");
    std::fs::write(
        &cfg,
        format!(
            "[node]\nrpc_url = \"http://127.0.0.1:{rpc_port}\"\n[dashboard]\nbind = \"127.0.0.1:{dash}\"\n[ranking]\njoin = false\n"
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
    let _guard = Kill(child);
    let start = Instant::now();
    while get(dash, "/healthz").is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "csr did not start"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let status = get(dash, "/api/status").unwrap();
    assert!(status.contains("\"state\":\"Synced\""), "{status}");
    assert!(status.contains("QmMock"), "{status}");
    let node = get(dash, "/api/node").unwrap();
    assert!(node.contains("\"peers_out\":1"), "{node}");
    assert!(dir.path().join("testnet/keys/service.key").exists());
}
