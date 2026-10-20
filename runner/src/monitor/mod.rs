//! Client for the CSR Network Monitor's public API with a 5-minute cache.

use crate::app::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const CACHE_TTL: Duration = Duration::from_secs(300);
/// After a failed refresh, wait this long before asking the monitor again.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(15);
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Country {
    pub code: String,
    pub count: u64,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct NetworkStats {
    pub nodes_total: u64,
    pub nodes_reachable: u64,
    #[serde(default)]
    pub countries: Vec<Country>,
    pub updated_at: i64,
    #[serde(default)]
    pub locations: Vec<Location>,
    #[serde(default)]
    pub unlocated: u64,
    #[serde(default)]
    pub locations_available: bool,
}

/// Centre of a 2° cell with the number of runners in it (display only).
#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    pub count: u64,
}

const MAX_LOCATIONS: usize = 2000;

/// Lenient parse of a `/v1/locations` reply: `None` unless it has a `locations` array.
fn parse_locations(v: &Value) -> Option<(Vec<Location>, u64)> {
    let arr = v.get("locations")?.as_array()?;
    let list = arr
        .iter()
        .filter_map(|e| {
            let lat = e.get("lat")?.as_f64()?;
            let lon = e.get("lon")?.as_f64()?;
            let count = e.get("count")?.as_u64()?;
            let ok = lat.is_finite()
                && lon.is_finite()
                && (-90.0..=90.0).contains(&lat)
                && (-180.0..=180.0).contains(&lon)
                && count > 0;
            ok.then_some(Location { lat, lon, count })
        })
        .take(MAX_LOCATIONS)
        .collect();
    let unlocated = v.get("unlocated").and_then(Value::as_u64).unwrap_or(0);
    Some((list, unlocated))
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Availability {
    pub d7: f64,
    pub d30: f64,
    pub d365: f64,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Attestation {
    pub status: String,
    #[serde(default)]
    pub expiry: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct NodeStats {
    pub rank: u64,
    pub of: u64,
    pub availability: Availability,
    pub check_type: String,
    pub attestation: Attestation,
    pub last_check_at: i64,
    /// The monitor's view of the registration stake (any JSON); `null` if it sends none.
    #[serde(default)]
    pub stake: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct Snapshot {
    network: NetworkStats,
    node: Option<NodeStats>,
    fetched_at: i64,
    at: Instant,
    /// What the snapshot was fetched for (monitor URL, peer id).
    key: (String, Option<String>),
}

#[derive(Default)]
pub struct Cache {
    snap: Option<Snapshot>,
    last_error: Option<String>,
    last_failure: Option<(Instant, (String, Option<String>))>,
}

fn monitor_base(state: &AppState) -> String {
    let c = state.config.read().unwrap();
    let url = if c.monitor.url.is_empty() {
        crate::network::NetworkDefaults::for_network(&c.network).monitor_url
    } else {
        c.monitor.url.clone()
    };
    url.trim_end_matches('/').to_string()
}

fn encode_segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

async fn get(http: &reqwest::Client, url: &str) -> Result<Option<Value>, String> {
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("monitor request failed: {}", e.without_url()))?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(format!("monitor replied {status}"));
    }
    resp.json::<Value>()
        .await
        .map(Some)
        .map_err(|e| format!("bad monitor reply: {}", e.without_url()))
}

async fn fetch(
    base: &str,
    peer_id: Option<&str>,
) -> Result<(NetworkStats, Option<NodeStats>), String> {
    let http = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let net = get(&http, &format!("{base}/v1/network"))
        .await?
        .ok_or_else(|| "monitor has no /v1/network".to_string())?;
    let mut network: NetworkStats =
        serde_json::from_value(net).map_err(|e| format!("bad /v1/network reply: {e}"))?;
    let parsed = match get(&http, &format!("{base}/v1/locations")).await {
        Ok(Some(v)) => parse_locations(&v),
        _ => None,
    };
    network.locations_available = parsed.is_some();
    (network.locations, network.unlocated) = parsed.unwrap_or_default();
    let node = match peer_id {
        None => None,
        Some(id) => match get(&http, &format!("{base}/v1/nodes/{}", encode_segment(id))).await? {
            None => None,
            Some(v) => Some(
                serde_json::from_value::<NodeStats>(v)
                    .map_err(|e| format!("bad /v1/nodes reply: {e}"))?,
            ),
        },
    };
    Ok((network, node))
}

/// Refreshes the cache if it is older than 5 minutes; failures keep the old data.
async fn refresh(state: &AppState) {
    let base = monitor_base(state);
    let peer_id = state.peer_id.read().unwrap().clone();
    let key = (base.clone(), peer_id.clone());
    let mut cache = state.monitor_cache.lock().await;
    if let Some(s) = &cache.snap {
        if s.key == key && s.at.elapsed() < CACHE_TTL {
            return;
        }
    }
    if let Some((t, k)) = &cache.last_failure {
        if *k == key && t.elapsed() < RETRY_AFTER_FAILURE {
            return;
        }
    }
    match fetch(&base, peer_id.as_deref()).await {
        Ok((network, node)) => {
            cache.snap = Some(Snapshot {
                network,
                node,
                fetched_at: crate::run::now(),
                at: Instant::now(),
                key,
            });
            cache.last_error = None;
            cache.last_failure = None;
        }
        Err(e) => {
            tracing::warn!("network stats refresh failed: {e}");
            cache.last_error = Some(e);
            cache.last_failure = Some((Instant::now(), key));
        }
    }
}

/// Body of `GET /api/network`.
pub async fn network_json(state: &AppState) -> Value {
    let (join, mode) = {
        let c = state.config.read().unwrap();
        (c.ranking.join, c.ranking.ping_mode.clone())
    };
    if !join {
        return json!({"join": false, "message": "Join the CSR monitor to see stats"});
    }
    refresh(state).await;
    let base = monitor_base(state);
    let key_peer = state.peer_id.read().unwrap().clone();
    let cache = state.monitor_cache.lock().await;
    let ok_at = state.last_check_ok_at.load(Ordering::Relaxed);
    let ping = json!({
        "mode": mode,
        "last_check_ok_at": if ok_at > 0 { json!(ok_at) } else { Value::Null },
    });
    // Cached data only counts if it belongs to the current monitor and node.
    let snap = cache
        .snap
        .as_ref()
        .filter(|s| s.key == (base.clone(), key_peer.clone()));
    let (error, stale) = match (&cache.last_error, snap) {
        (Some(e), Some(_)) => (json!(e), true),
        (Some(e), None) => (json!(e), false),
        (None, None) => (json!("no data from the monitor yet"), false),
        (None, Some(_)) => (Value::Null, false),
    };
    json!({
        "join": true,
        "monitor_url": base,
        "fetched_at": snap.map(|s| json!(s.fetched_at)).unwrap_or(Value::Null),
        "stale": stale,
        "error": error,
        "network": snap.map(|s| json!(s.network)),
        "node": snap.and_then(|s| s.node.as_ref()).map(|n| json!(n)),
        "ping": ping,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_id_is_url_encoded() {
        assert_eq!(encode_segment("Qm1/a b"), "Qm1%2Fa%20b");
        assert_eq!(encode_segment("Qm-_.~Z9"), "Qm-_.~Z9");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let n: NetworkStats = serde_json::from_value(json!({
            "nodes_total": 1, "nodes_reachable": 1, "updated_at": 5, "x": 1
        }))
        .unwrap();
        assert!(n.countries.is_empty());
    }

    #[test]
    fn locations_valid_list_and_dropped_entries() {
        let (l, u) = parse_locations(&json!({"unlocated": 3, "locations": [
            {"lat": 53, "lon": 13, "count": 2}, {"lat": 95, "lon": 0, "count": 1},
            {"lat": 1, "lon": -181, "count": 1}, {"lat": 1, "lon": 1, "count": 0},
            {"lat": "5", "lon": 1, "count": 1}, {"lat": 1, "lon": 1, "count": 1.5},
            {"lat": -33, "lon": 151, "count": 1}, 7
        ]}))
        .unwrap();
        assert_eq!(u, 3);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0], Location { lat: 53.0, lon: 13.0, count: 2 });
        assert_eq!(l[1].lat, -33.0);
    }

    #[test]
    fn locations_capped() {
        let arr: Vec<Value> = (0..2500).map(|_| json!({"lat": 1, "lon": 1, "count": 1})).collect();
        let (l, u) = parse_locations(&json!({"locations": arr})).unwrap();
        assert_eq!((l.len(), u), (2000, 0));
    }

    #[test]
    fn locations_missing_array() {
        assert!(parse_locations(&json!({"unlocated": 1})).is_none());
        assert!(parse_locations(&json!({"locations": "x"})).is_none());
        assert!(parse_locations(&json!([])).is_none());
    }
}
