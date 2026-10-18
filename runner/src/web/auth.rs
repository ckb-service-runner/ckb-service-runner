//! Sessions, login and the auth/CSRF middleware.

use crate::app::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::RngCore;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

const COOKIE: &str = "csr_session";
const SESSION_TTL: Duration = Duration::from_secs(24 * 3600);
const MAX_SESSIONS: usize = 1000;

type St = State<Arc<AppState>>;

#[derive(Clone)]
pub struct Session {
    pub csrf: String,
    pub authenticated: bool,
    created: Instant,
}

fn token() -> String {
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    hex::encode(b)
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |x, (p, q)| x | (p ^ q)) == 0
}

fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|p| p.trim().strip_prefix("csr_session="))
        .map(String::from)
        .next()
}

fn set_cookie(id: &str) -> (header::HeaderName, HeaderValue) {
    (
        header::SET_COOKIE,
        HeaderValue::from_str(&format!("{COOKIE}={id}; HttpOnly; SameSite=Strict; Path=/"))
            .unwrap(),
    )
}

impl AppState {
    fn session(&self, headers: &HeaderMap) -> Option<(String, Session)> {
        let id = cookie_value(headers)?;
        let mut m = self.sessions.lock().unwrap();
        match m.get(&id) {
            Some(s) if s.created.elapsed() < SESSION_TTL => Some((id, s.clone())),
            Some(_) => {
                m.remove(&id);
                None
            }
            None => None,
        }
    }

    fn new_session(&self, authenticated: bool) -> (String, Session) {
        let id = token();
        let s = Session {
            csrf: token(),
            authenticated,
            created: Instant::now(),
        };
        let mut m = self.sessions.lock().unwrap();
        m.retain(|_, v| v.created.elapsed() < SESSION_TTL);
        if m.len() >= MAX_SESSIONS {
            if let Some(oldest) = m
                .iter()
                .min_by_key(|(_, v)| v.created)
                .map(|(k, _)| k.clone())
            {
                m.remove(&oldest);
            }
        }
        m.insert(id.clone(), s.clone());
        (id, s)
    }

    pub fn password_required(&self) -> bool {
        !self
            .config
            .read()
            .unwrap()
            .dashboard
            .password_hash
            .is_empty()
    }
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({"error": msg}))).into_response()
}

/// Auth for `/api/*` (except session/login) and CSRF for every write.
pub async fn guard(State(s): St, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if !path.starts_with("/api/") {
        return next.run(req).await;
    }
    let exempt = path == "/api/session" || path == "/api/login";
    let sess = s.session(req.headers());
    if !exempt && s.password_required() && !sess.as_ref().is_some_and(|(_, x)| x.authenticated) {
        return err(StatusCode::UNAUTHORIZED, "authentication required");
    }
    let write = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if write {
        let sent = req
            .headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !sess.as_ref().is_some_and(|(_, x)| ct_eq(&x.csrf, sent)) {
            return err(
                StatusCode::FORBIDDEN,
                "missing or invalid session or CSRF token",
            );
        }
    }
    next.run(req).await
}

pub async fn session(State(s): St, headers: HeaderMap) -> Response {
    let (existing, fresh) = match s.session(&headers) {
        Some(x) => (x, false),
        None => (s.new_session(false), true),
    };
    let (id, sess) = existing;
    let body = Json(json!({
        "csrf_token": sess.csrf,
        "authenticated": sess.authenticated || !s.password_required(),
        "password_required": s.password_required(),
    }));
    if fresh {
        ([set_cookie(&id)], body).into_response()
    } else {
        body.into_response()
    }
}

pub async fn login(State(s): St, Json(body): Json<Value>) -> Response {
    let pw = body.get("password").and_then(Value::as_str).unwrap_or("");
    let hash = s.config.read().unwrap().dashboard.password_hash.clone();
    if hash.is_empty() {
        return err(StatusCode::BAD_REQUEST, "no password is set");
    }
    let pw = pw.to_string();
    let ok = tokio::task::spawn_blocking(move || crate::password::verify(&pw, &hash))
        .await
        .unwrap_or(false);
    if !ok {
        tokio::time::sleep(Duration::from_millis(500)).await;
        tracing::warn!("dashboard login failed");
        return err(StatusCode::UNAUTHORIZED, "wrong password");
    }
    // New session id and CSRF token on privilege change.
    let (id, sess) = s.new_session(true);
    tracing::info!("dashboard login ok");
    ([set_cookie(&id)], Json(json!({"csrf_token": sess.csrf}))).into_response()
}
