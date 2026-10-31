//! `GET/PUT /api/settings` and the live status event stream.

use crate::app::AppState;
use crate::config::Config;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream::{self, Stream};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

type St = State<Arc<AppState>>;

fn bad(msg: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": msg.into()}))).into_response()
}

/// The config as the API shows it: no hash, only `dashboard.password_set`.
fn public_json(cfg: &Config) -> Value {
    let mut v = serde_json::to_value(cfg).unwrap();
    let d = v["dashboard"].as_object_mut().unwrap();
    d.remove("password_hash");
    d.insert(
        "password_set".into(),
        json!(!cfg.dashboard.password_hash.is_empty()),
    );
    v
}

fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(&k) {
                    Some(slot) if slot.is_object() && v.is_object() => merge(slot, v),
                    _ => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, p) => *b = p,
    }
}

pub async fn get(State(s): St) -> Json<Value> {
    Json(public_json(&s.config.read().unwrap()))
}

pub async fn put(State(s): St, Json(patch): Json<Value>) -> Response {
    let Some(p) = patch.as_object() else {
        return bad("body must be a JSON object");
    };
    // D31: the network follows the node; it is set by the installer, never from the dashboard.
    if p.contains_key("network") {
        return bad("network follows the node and cannot be changed here");
    }
    if let Some(d) = p.get("dashboard") {
        let Some(d) = d.as_object() else {
            return bad("dashboard must be an object");
        };
        if d.contains_key("password_set") || d.contains_key("password_hash") {
            return bad("dashboard.password_set and the password hash are read-only (use `csr set-password`)");
        }
    }
    let _guard = s.config_write.lock().unwrap();
    let old = s.config.read().unwrap().clone();
    let mut v = serde_json::to_value(&old).unwrap();
    merge(&mut v, patch);
    let new: Config = match serde_json::from_value(v) {
        Ok(c) => c,
        Err(e) => return bad(format!("invalid settings: {e}")),
    };
    if let Err(e) = new.validate() {
        return bad(e);
    }
    if let Err(e) = new.save(&s.config_path) {
        tracing::error!("saving settings failed: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "cannot save the config file"})),
        )
            .into_response();
    }
    let restart = old.needs_restart(&new);
    if old.logging.level != new.logging.level {
        (s.set_log_level)(&new.logging.level);
    }
    let out = public_json(&new);
    *s.config.write().unwrap() = new;
    tracing::info!("settings saved");
    Json(json!({"settings": out, "restart_required": restart})).into_response()
}

fn comparable(mut v: Value) -> String {
    if let Some(o) = v.as_object_mut() {
        o.remove("uptime_s");
    }
    v.to_string()
}

struct Sse1 {
    s: Arc<AppState>,
    last: Option<String>,
    sent: Instant,
}

pub async fn events(State(s): St) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let st = Sse1 {
        s,
        last: None,
        sent: Instant::now(),
    };
    let stream = stream::unfold(st, |mut st| async move {
        loop {
            let v = st.s.status_json();
            let key = comparable(v.clone());
            let first = st.last.is_none();
            if first
                || st.last.as_deref() != Some(&key)
                || st.sent.elapsed() >= Duration::from_secs(10)
            {
                st.last = Some(key);
                st.sent = Instant::now();
                let ev = Event::default().event("status").data(v.to_string());
                return Some((Ok(ev), st));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_merge() {
        let mut a = json!({"a": {"b": 1, "c": 2}, "d": [1]});
        merge(&mut a, json!({"a": {"b": 5}, "d": [2, 3]}));
        assert_eq!(a, json!({"a": {"b": 5, "c": 2}, "d": [2, 3]}));
    }

    #[test]
    fn hash_hidden() {
        let mut c = Config::default();
        c.dashboard.password_hash = "$argon2id$x".into();
        let j = public_json(&c);
        assert_eq!(j["dashboard"]["password_set"], true);
        assert!(!j.to_string().contains("argon2"));
    }
}
