//! Rust mock of the CSR Network Monitor (`/v1/checkin`, `/v1/answer`) plus a mock CKB node.
#![allow(dead_code)]

use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn sign(key: &SecretKey, msg: &str) -> String {
    let d = Message::from_digest(Sha256::digest(msg.as_bytes()).into());
    let sig = Secp256k1::new().sign_ecdsa(&d, key);
    format!("0x{}", hex::encode(sig.serialize_compact()))
}

pub fn verify(pubkey: &str, msg: &str, sig: &str) -> bool {
    let Ok(pk) = PublicKey::from_slice(&hex::decode(&pubkey[2..]).unwrap()) else {
        return false;
    };
    let d = Message::from_digest(Sha256::digest(msg.as_bytes()).into());
    let sig = secp256k1::ecdsa::Signature::from_compact(&hex::decode(&sig[2..]).unwrap()).unwrap();
    Secp256k1::new().verify_ecdsa(&d, &sig, &pk).is_ok()
}

pub fn answer_message(a: &serde_json::Value) -> String {
    let mut parts = vec!["CSR-PING-ANSWER-v1".to_string()];
    for k in [
        "nonce",
        "peer_id",
        "service_pubkey",
        "binding_sig",
        "network",
        "csr_version",
        "node_version",
        "r2_state",
        "tip_number",
        "tip_hash",
        "block_hash_at_height",
        "uptime_s",
        "ping_mode",
    ] {
        let v = &a[k];
        parts.push(
            v.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| v.to_string()),
        );
    }
    parts.join("\n")
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

pub fn monitor_key() -> SecretKey {
    SecretKey::from_slice(&[0x42; 32]).unwrap()
}

pub fn monitor_pubkey() -> String {
    let pk = PublicKey::from_secret_key(&Secp256k1::new(), &monitor_key());
    format!("0x{}", hex::encode(pk.serialize()))
}

/// Builds a challenge JSON signed by `key`.
pub fn make_challenge(key: &SecretKey, nonce_byte: u8, height: u64, issued_at: i64) -> String {
    let nonce = format!("0x{}", hex::encode([nonce_byte; 32]));
    let sig = sign(
        key,
        &format!("CSR-PING-CHALLENGE-v1\n{nonce}\n{height}\n{issued_at}"),
    );
    format!(
        r#"{{"nonce":"{nonce}","height":{height},"issued_at":{issued_at},"monitor_sig":"{sig}"}}"#
    )
}

#[derive(Default)]
pub struct Records {
    pub checkins: Vec<serde_json::Value>,
    pub answers: Vec<serde_json::Value>,
}

pub struct MockMonitor {
    pub port: u16,
    pub rec: Arc<Mutex<Records>>,
}

impl MockMonitor {
    /// `signer` signs the challenges; use another key than `monitor_key()` to produce invalid ones.
    pub fn start(signer: SecretKey) -> MockMonitor {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let rec = Arc::new(Mutex::new(Records::default()));
        let r = rec.clone();
        std::thread::spawn(move || {
            let mut n = 0u8;
            for c in l.incoming() {
                let Ok(c) = c else { return };
                n = n.wrapping_add(1);
                let r = r.clone();
                std::thread::spawn(move || {
                    handle(c, |path, body| match path {
                        "/v1/checkin" => {
                            r.lock()
                                .unwrap()
                                .checkins
                                .push(serde_json::from_str(body).unwrap());
                            format!(
                                r#"{{"challenge":{}}}"#,
                                make_challenge(&signer, n, 900, now())
                            )
                        }
                        "/v1/answer" => {
                            r.lock()
                                .unwrap()
                                .answers
                                .push(serde_json::from_str(body).unwrap());
                            r#"{"ok":true}"#.to_string()
                        }
                        _ => "{}".to_string(),
                    })
                });
            }
        });
        MockMonitor { port, rec }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn read_request(c: &mut TcpStream) -> Option<(String, String, String)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = c.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        let Some(split) = text.find("\r\n\r\n") else {
            continue;
        };
        let head = text[..split].to_ascii_lowercase();
        let len = head
            .lines()
            .find_map(|l| {
                l.strip_prefix("content-length:")
                    .map(|v| v.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if buf.len() < split + 4 + len {
            continue;
        }
        let first = text.lines().next()?.to_string();
        let mut it = first.split(' ');
        let _method = it.next()?;
        let path = it.next()?.to_string();
        return Some((first, path, text[split + 4..split + 4 + len].to_string()));
    }
}

fn handle(mut c: TcpStream, f: impl Fn(&str, &str) -> String) {
    let _ = c.set_read_timeout(Some(Duration::from_secs(5)));
    while let Some((_, path, body)) = read_request(&mut c) {
        let out = f(&path, &body);
        if write!(
            c,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            out.len(),
            out
        )
        .is_err()
        {
            return;
        }
    }
}

/// Mock CKB node answering the methods CSR uses.
pub fn start_mock_node() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(c) = c else { return };
            std::thread::spawn(move || {
                handle(c, |_, body| {
                    let method = body
                        .split("\"method\":\"")
                        .nth(1)
                        .and_then(|r| r.split('"').next())
                        .unwrap_or("");
                    let result = match method {
                        "get_blockchain_info" => {
                            r#"{"chain":"ckb_testnet","is_initial_block_download":false}"#
                        }
                        "sync_state" => r#"{"ibd":false,"best_known_block_number":"0x3e8"}"#,
                        "get_tip_header" => r#"{"number":"0x3e8","hash":"0xab","epoch":"0x1"}"#,
                        "get_block_hash" => r#""0xcd""#,
                        "local_node_info" => r#"{"version":"0.210.0","node_id":"QmMock"}"#,
                        "get_peers" => r#"[{"is_outbound":true}]"#,
                        "get_indexer_tip" => r#"{"block_number":"0x3e8"}"#,
                        "tx_pool_info" => r#"{"pending":"0x1","proposed":"0x2"}"#,
                        _ => "null",
                    };
                    format!(r#"{{"jsonrpc":"2.0","id":1,"result":{result}}}"#)
                })
            });
        }
    });
    port
}

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Plain HTTP request; returns (status, body).
pub fn http(port: u16, method: &str, path: &str, body: &str) -> Option<(u16, String)> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nhost: x\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    )
    .ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    let status = out.split(' ').nth(1)?.parse().ok()?;
    let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Some((status, body))
}

pub fn wait_for(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}
