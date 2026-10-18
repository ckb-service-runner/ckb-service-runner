//! R2 sync guard: gathers a node snapshot and evaluates the R2 state.

use super::rpc::{hex_u64, RpcClient, RpcError};
use serde::Serialize;
use serde_json::{json, Value};

pub const LAG_LIMIT: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum R2State {
    Unknown,
    Unreachable,
    Degraded,
    Syncing,
    Synced,
}

impl R2State {
    pub fn as_str(self) -> &'static str {
        match self {
            R2State::Unknown => "Unknown",
            R2State::Unreachable => "Unreachable",
            R2State::Degraded => "Degraded",
            R2State::Syncing => "Syncing",
            R2State::Synced => "Synced",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TipHeader {
    pub number: u64,
    pub hash: String,
    pub epoch: String,
}

/// Raw facts about the node; `None` means the call failed or the method is missing.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// A transport/HTTP failure happened.
    pub transport_error: bool,
    pub tip: Option<TipHeader>,
    /// `(chain, is_initial_block_download)` from `get_blockchain_info`.
    pub chain_info: Option<(String, bool)>,
    /// `(ibd, best_known_block_number)` from `sync_state`.
    pub sync: Option<(bool, u64)>,
    pub node_version: Option<String>,
    pub node_id: Option<String>,
    /// `is_outbound` of every peer.
    pub peers: Option<Vec<bool>>,
    pub indexer_tip: Option<u64>,
    /// `get_indexer_tip` answered "method not found".
    pub indexer_missing: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct R2Report {
    pub state: R2State,
    pub reasons: Vec<String>,
    pub tip: Option<u64>,
    pub best_known: Option<u64>,
    pub tip_lag: Option<u64>,
    pub indexer_tip: Option<u64>,
    pub indexer_lag: Option<u64>,
    pub outbound_peers: Option<u64>,
    pub ibd: Option<bool>,
}

impl R2Report {
    pub fn unknown() -> R2Report {
        R2Report {
            state: R2State::Unknown,
            reasons: vec![],
            tip: None,
            best_known: None,
            tip_lag: None,
            indexer_tip: None,
            indexer_lag: None,
            outbound_peers: None,
            ibd: None,
        }
    }
}

pub fn expected_chain(network: &str) -> &'static str {
    if network == "mainnet" {
        "ckb"
    } else {
        "ckb_testnet"
    }
}

fn opt<T>(r: Result<T, RpcError>, snap: &mut Snapshot) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            if e.is_transport() {
                snap.transport_error = true;
            }
            None
        }
    }
}

/// Queries the node. Stops early on the first transport error.
pub async fn fetch_snapshot(c: &RpcClient) -> Snapshot {
    let mut s = Snapshot::default();
    let info = opt(c.call("get_blockchain_info", json!([])).await, &mut s);
    if s.transport_error {
        return s;
    }
    s.chain_info = info.and_then(|v| {
        Some((
            v.get("chain")?.as_str()?.to_string(),
            v.get("is_initial_block_download")?.as_bool()?,
        ))
    });
    let sync = opt(c.call("sync_state", json!([])).await, &mut s);
    if s.transport_error {
        return s;
    }
    s.sync = sync.and_then(|v| {
        Some((
            v.get("ibd")?.as_bool()?,
            hex_u64(v.get("best_known_block_number")?)?,
        ))
    });
    let tip = opt(c.call("get_tip_header", json!([])).await, &mut s);
    if s.transport_error {
        return s;
    }
    s.tip = tip.and_then(|v| {
        Some(TipHeader {
            number: hex_u64(v.get("number")?)?,
            hash: v.get("hash")?.as_str()?.to_string(),
            epoch: v.get("epoch")?.as_str()?.to_string(),
        })
    });
    let ni = opt(c.call("local_node_info", json!([])).await, &mut s);
    if let Some(v) = ni {
        s.node_version = v.get("version").and_then(Value::as_str).map(String::from);
        s.node_id = v.get("node_id").and_then(Value::as_str).map(String::from);
    }
    let peers = opt(c.call("get_peers", json!([])).await, &mut s);
    s.peers = peers.and_then(|v| {
        v.as_array().map(|a| {
            a.iter()
                .map(|p| p.get("is_outbound").and_then(Value::as_bool) == Some(true))
                .collect()
        })
    });
    match c.call("get_indexer_tip", json!([])).await {
        Ok(v) => s.indexer_tip = v.get("block_number").and_then(hex_u64),
        Err(e) => {
            if e.is_method_not_found() {
                s.indexer_missing = true;
            } else if e.is_transport() {
                s.transport_error = true;
            }
        }
    }
    s
}

/// Pure R2 evaluation; the first matching rule wins.
pub fn evaluate(s: &Snapshot, expected_chain: &str) -> R2Report {
    let mut r = R2Report::unknown();
    r.tip = s.tip.as_ref().map(|t| t.number);
    r.best_known = s.sync.map(|x| x.1);
    r.ibd = match (&s.chain_info, s.sync) {
        (Some((_, a)), Some((b, _))) => Some(*a || b),
        _ => None,
    };
    if let (Some(tip), Some(bk)) = (r.tip, r.best_known) {
        r.tip_lag = Some(bk.saturating_sub(tip));
    }
    r.outbound_peers = s
        .peers
        .as_ref()
        .map(|p| p.iter().filter(|o| **o).count() as u64);
    r.indexer_tip = s.indexer_tip;
    if let (Some(tip), Some(it)) = (r.tip, s.indexer_tip) {
        r.indexer_lag = Some(tip.saturating_sub(it));
    }

    if s.transport_error || s.tip.is_none() {
        r.state = R2State::Unreachable;
        r.reasons
            .push("node unreachable or get_tip_header failed".into());
        return r;
    }
    let (Some((chain, _)), Some(_)) = (&s.chain_info, s.sync) else {
        r.state = R2State::Degraded;
        r.reasons.push(
            "unsupported node: sync_state or get_blockchain_info missing (light client?)".into(),
        );
        return r;
    };
    if chain != expected_chain {
        r.state = R2State::Degraded;
        r.reasons.push(format!(
            "chain mismatch: expected {expected_chain}, node reports {chain}"
        ));
        return r;
    }
    if r.ibd == Some(true) || r.tip_lag.unwrap_or(0) > LAG_LIMIT {
        r.state = R2State::Syncing;
        if r.ibd == Some(true) {
            r.reasons.push("node is in initial block download".into());
        }
        if let Some(l) = r.tip_lag.filter(|l| *l > LAG_LIMIT) {
            r.reasons
                .push(format!("tip lag {l} blocks (limit {LAG_LIMIT})"));
        }
        return r;
    }
    if r.outbound_peers.unwrap_or(0) == 0 {
        r.reasons.push("no outbound peers".into());
    }
    if s.indexer_tip.is_none() {
        r.reasons.push(if s.indexer_missing {
            "indexer module disabled".into()
        } else {
            "indexer tip unavailable".into()
        });
    } else if let Some(l) = r.indexer_lag.filter(|l| *l > LAG_LIMIT) {
        r.reasons
            .push(format!("indexer lag {l} blocks (limit {LAG_LIMIT})"));
    }
    r.state = if r.reasons.is_empty() {
        R2State::Synced
    } else {
        R2State::Degraded
    };
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Snapshot {
        Snapshot {
            transport_error: false,
            tip: Some(TipHeader {
                number: 1000,
                hash: "0xab".into(),
                epoch: "0x1".into(),
            }),
            chain_info: Some(("ckb_testnet".into(), false)),
            sync: Some((false, 1000)),
            node_version: Some("0.210.0".into()),
            node_id: Some("Qm".into()),
            peers: Some(vec![true, false]),
            indexer_tip: Some(1000),
            indexer_missing: false,
        }
    }

    fn eval(s: &Snapshot) -> R2Report {
        evaluate(s, "ckb_testnet")
    }

    fn joined(r: &R2Report) -> String {
        r.reasons.join(" | ")
    }

    #[test]
    fn synced() {
        let r = eval(&good());
        assert_eq!(r.state, R2State::Synced);
        assert!(r.reasons.is_empty());
        assert_eq!(r.tip_lag, Some(0));
        assert_eq!(r.outbound_peers, Some(1));
        assert_eq!(r.ibd, Some(false));
    }

    #[test]
    fn unreachable_by_transport_or_missing_tip() {
        let mut s = good();
        s.transport_error = true;
        assert_eq!(eval(&s).state, R2State::Unreachable);
        let mut s = good();
        s.tip = None;
        assert_eq!(eval(&s).state, R2State::Unreachable);
    }

    #[test]
    fn unreachable_beats_everything() {
        let mut s = good();
        s.tip = None;
        s.sync = None;
        s.chain_info = Some(("ckb".into(), true));
        assert_eq!(eval(&s).state, R2State::Unreachable);
    }

    #[test]
    fn unsupported_light_client() {
        for which in 0..2 {
            let mut s = good();
            if which == 0 {
                s.sync = None;
            } else {
                s.chain_info = None;
            }
            let r = eval(&s);
            assert_eq!(r.state, R2State::Degraded);
            assert!(joined(&r).contains("unsupported"));
        }
        // beats chain mismatch and IBD
        let mut s = good();
        s.sync = None;
        s.chain_info = Some(("ckb".into(), true));
        assert!(joined(&eval(&s)).contains("unsupported"));
    }

    #[test]
    fn wrong_chain_beats_syncing() {
        let mut s = good();
        s.chain_info = Some(("ckb".into(), true));
        let r = eval(&s);
        assert_eq!(r.state, R2State::Degraded);
        assert!(joined(&r).contains("chain"));
        assert_eq!(evaluate(&good(), "ckb").state, R2State::Degraded);
    }

    #[test]
    fn ibd_from_either_source() {
        let mut s = good();
        s.chain_info = Some(("ckb_testnet".into(), true));
        assert_eq!(eval(&s).state, R2State::Syncing);
        let mut s = good();
        s.sync = Some((true, 1000));
        assert_eq!(eval(&s).state, R2State::Syncing);
    }

    #[test]
    fn tip_lag_boundaries() {
        let mut s = good();
        s.sync = Some((false, 1005));
        assert_eq!(eval(&s).state, R2State::Synced);
        s.sync = Some((false, 1006));
        let r = eval(&s);
        assert_eq!(r.state, R2State::Syncing);
        assert_eq!(r.tip_lag, Some(6));
        s.sync = Some((false, 990));
        let r = eval(&s);
        assert_eq!(r.state, R2State::Synced);
        assert_eq!(r.tip_lag, Some(0));
    }

    #[test]
    fn syncing_beats_degraded() {
        let mut s = good();
        s.sync = Some((true, 1000));
        s.peers = Some(vec![]);
        s.indexer_missing = true;
        s.indexer_tip = None;
        assert_eq!(eval(&s).state, R2State::Syncing);
    }

    #[test]
    fn outbound_peers() {
        let mut s = good();
        s.peers = Some(vec![false]);
        let r = eval(&s);
        assert_eq!(r.state, R2State::Degraded);
        assert!(joined(&r).contains("outbound peers"));
        s.peers = Some(vec![]);
        assert_eq!(eval(&s).state, R2State::Degraded);
        s.peers = Some(vec![true]);
        assert_eq!(eval(&s).state, R2State::Synced);
    }

    #[test]
    fn indexer_boundaries() {
        let mut s = good();
        s.indexer_tip = Some(995);
        assert_eq!(eval(&s).state, R2State::Synced);
        s.indexer_tip = Some(994);
        let r = eval(&s);
        assert_eq!(r.state, R2State::Degraded);
        assert_eq!(r.indexer_lag, Some(6));
        assert!(joined(&r).contains("indexer"));
        s.indexer_tip = Some(1003); // indexer ahead of tip
        let r = eval(&s);
        assert_eq!(r.state, R2State::Synced);
        assert_eq!(r.indexer_lag, Some(0));
    }

    #[test]
    fn indexer_missing_and_multiple_reasons() {
        let mut s = good();
        s.indexer_tip = None;
        s.indexer_missing = true;
        s.peers = Some(vec![]);
        let r = eval(&s);
        assert_eq!(r.state, R2State::Degraded);
        assert!(joined(&r).contains("indexer"));
        assert!(joined(&r).contains("outbound peers"));
        assert_eq!(r.reasons.len(), 2);
    }

    #[test]
    fn unknown_report() {
        assert_eq!(R2Report::unknown().state, R2State::Unknown);
    }

    /// Tiny HTTP/JSON-RPC mock bound to an OS-chosen port.
    async fn mock_server(light: bool) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = l.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    let body = loop {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some(h) = text.find("\r\n\r\n") {
                            let len: usize = text[..h]
                                .to_lowercase()
                                .lines()
                                .find_map(|l| {
                                    l.strip_prefix("content-length:")
                                        .map(|v| v.trim().parse().unwrap())
                                })
                                .unwrap_or(0);
                            if text.len() >= h + 4 + len {
                                break text[h + 4..h + 4 + len].to_string();
                            }
                        }
                    };
                    let req: Value = serde_json::from_str(&body).unwrap();
                    let m = req["method"].as_str().unwrap();
                    let missing = json!({"code": -32601, "message": "Method not found"});
                    let res: Result<Value, Value> = match m {
                        "get_blockchain_info" | "sync_state" if light => Err(missing),
                        "get_blockchain_info" => {
                            Ok(json!({"chain": "ckb_testnet", "is_initial_block_download": false}))
                        }
                        "sync_state" => {
                            Ok(json!({"ibd": false, "best_known_block_number": "0x3e8"}))
                        }
                        "get_tip_header" => Ok(
                            json!({"number": "0x3e8", "hash": "0xab", "epoch": "0x7080018000001"}),
                        ),
                        "local_node_info" => Ok(json!({"version": "0.210.0", "node_id": "Qm"})),
                        "get_peers" => Ok(json!([{"is_outbound": true}])),
                        _ => Err(missing),
                    };
                    let out = match res {
                        Ok(r) => json!({"jsonrpc": "2.0", "id": 1, "result": r}),
                        Err(e) => json!({"jsonrpc": "2.0", "id": 1, "error": e}),
                    }
                    .to_string();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        out.len(),
                        out
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        url
    }

    #[tokio::test]
    async fn fetch_against_mock() {
        let c = RpcClient::new(&mock_server(false).await);
        let s = fetch_snapshot(&c).await;
        assert!(s.indexer_missing);
        let r = eval(&s);
        assert_eq!(r.state, R2State::Degraded);
        assert_eq!(r.tip, Some(1000));
        assert_eq!(s.node_version.as_deref(), Some("0.210.0"));
        let c = RpcClient::new(&mock_server(true).await);
        let r = eval(&fetch_snapshot(&c).await);
        assert!(joined(&r).contains("unsupported"));
    }

    #[tokio::test]
    async fn fetch_unreachable() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        drop(l);
        let s = fetch_snapshot(&RpcClient::new(&url)).await;
        assert_eq!(eval(&s).state, R2State::Unreachable);
    }
}
