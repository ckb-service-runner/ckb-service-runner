//! Minimal blocking JSON-RPC client for the CKB node.

use serde_json::{json, Value};
use std::time::Duration;

pub struct Rpc {
    pub url: String,
    http: reqwest::blocking::Client,
}

impl Rpc {
    pub fn new(url: &str) -> Result<Rpc, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Rpc {
            url: url.to_string(),
            http,
        })
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let resp: Value = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .map_err(|e| format!("{method}: cannot reach {}: {e}", self.url))?
            .json()
            .map_err(|e| format!("{method}: bad response: {e}"))?;
        if let Some(err) = resp.get("error").filter(|e| !e.is_null()) {
            return Err(format!("{method}: {err}"));
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn tip_number(&self) -> Result<u64, String> {
        hex_u64(&self.call("get_tip_header", json!([]))?["number"])
    }
}

pub fn hex_u64(v: &Value) -> Result<u64, String> {
    let s = v.as_str().ok_or("expected a hex quantity")?;
    u64::from_str_radix(s.trim_start_matches("0x"), 16)
        .map_err(|e| format!("bad quantity {s}: {e}"))
}
