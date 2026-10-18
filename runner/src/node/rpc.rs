//! Minimal JSON-RPC 2.0 client over HTTP (plain `reqwest` + `serde_json`).

use serde_json::{json, Value};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub enum RpcError {
    /// Connection, HTTP or decoding failure: the node is not usable.
    Transport(String),
    /// The node answered with a JSON-RPC error object.
    Rpc { code: i64, message: String },
}

impl RpcError {
    pub fn is_method_not_found(&self) -> bool {
        matches!(self, RpcError::Rpc { code: -32601, .. })
    }
    pub fn is_transport(&self) -> bool {
        matches!(self, RpcError::Transport(_))
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Transport(m) => write!(f, "transport error: {m}"),
            RpcError::Rpc { code, message } => write!(f, "rpc error {code}: {message}"),
        }
    }
}

#[derive(Clone)]
pub struct RpcClient {
    url: String,
    http: reqwest::Client,
}

impl RpcClient {
    pub fn new(url: &str) -> RpcClient {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .connect_timeout(TIMEOUT)
            .no_proxy()
            .build()
            .expect("http client");
        RpcClient {
            url: url.to_string(),
            http,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let resp = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| RpcError::Transport(e.without_url().to_string()))?;
        if !resp.status().is_success() {
            return Err(RpcError::Transport(format!("HTTP {}", resp.status())));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| RpcError::Transport(format!("invalid response: {}", e.without_url())))?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            return Err(RpcError::Rpc {
                code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| RpcError::Transport("response has neither result nor error".into()))
    }
}

/// Parses a `0x` hex quantity (also accepts a plain JSON number).
pub fn hex_u64(v: &Value) -> Option<u64> {
    match v {
        Value::String(s) => u64::from_str_radix(s.strip_prefix("0x")?, 16).ok(),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}
