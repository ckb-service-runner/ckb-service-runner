//! Sessions, login and the auth/CSRF middleware.

use crate::app::AppState;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::RngCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const COOKIE: &str = "csr_session";
const SESSION_TTL: Duration = Duration::from_secs(24 * 3600);
pub const LOGIN_FAILURES_PER_CLIENT: u32 = 5;
pub const LOGIN_FAILURES_GLOBAL: u32 = 50;
pub const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
pub const MAX_AUTH_SESSIONS: usize = 50;
pub const MAX_ANON_SESSIONS: usize = 200;

type St = State<Arc<AppState>>;

#[derive(Clone)]
pub struct Session {
    pub csrf: String,
    pub authenticated: bool,
    created: Instant,
}

/// Writes that move money or change the runner's configuration need a dashboard password.
pub fn write_needs_password(method: &Method, path: &str) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return false;
    }
    if path == "/api/registry/consent-message" {
        return false;
    }
    path == "/api/settings"
        || path.starts_with("/api/wallet/")
        || path.starts_with("/api/services/")
        || path.starts_with("/api/registry/")
}

/// On a loopback bind only the loopback names with the bound port are accepted (DNS rebinding).
pub fn host_allowed(host: Option<&str>, bind: &str) -> bool {
    let Some((bhost, port)) = bind.rsplit_once(':') else {
        return true;
    };
    if !matches!(bhost, "127.0.0.1" | "[::1]" | "localhost") {
        return true;
    }
    let Some(host) = host else { return false };
    host == format!("127.0.0.1:{port}")
        || host == format!("localhost:{port}")
        || host == format!("[::1]:{port}")
}

pub fn origin_allowed(origin: Option<&str>, host: &str) -> bool {
    match origin {
        None => true,
        Some(o) => o == format!("http://{host}") || o == format!("https://{host}"),
    }
}

pub fn cookie_header(id: &str, secure: bool) -> String {
    let mut c = format!("{COOKIE}={id}; HttpOnly; SameSite=Strict; Path=/");
    if secure {
        c.push_str("; Secure");
    }
    c
}

/// Failed-login counters per client and for all clients together.
pub struct LoginLimiter {
    inner: Mutex<LimiterInner>,
}

#[derive(Default)]
struct LimiterInner {
    per_client: HashMap<IpAddr, Vec<Instant>>,
    global: Vec<Instant>,
}

impl LoginLimiter {
    pub fn new() -> Self {
        LoginLimiter {
            inner: Mutex::new(LimiterInner::default()),
        }
    }

    fn prune(g: &mut LimiterInner, now: Instant) {
        let fresh = |t: &Instant| now.saturating_duration_since(*t) < LOGIN_WINDOW;
        g.global.retain(fresh);
        g.per_client.retain(|_, v| {
            v.retain(fresh);
            !v.is_empty()
        });
    }

    /// `Err(wait)` when this client or all clients together are over the limit.
    pub fn check(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let mut g = self.inner.lock().unwrap();
        Self::prune(&mut g, now);
        let wait = |v: &Vec<Instant>, limit: u32| -> Option<Duration> {
            let limit = limit as usize;
            if v.len() >= limit {
                // The block ends when the failure that made it reach the limit leaves the window.
                let t = v[v.len() - limit];
                Some(LOGIN_WINDOW.saturating_sub(now.saturating_duration_since(t)))
            } else {
                None
            }
        };
        let a = g
            .per_client
            .get(&ip)
            .and_then(|v| wait(v, LOGIN_FAILURES_PER_CLIENT));
        let b = wait(&g.global, LOGIN_FAILURES_GLOBAL);
        match (a, b) {
            (None, None) => Ok(()),
            (x, y) => Err(x.max(y).unwrap().max(Duration::from_secs(1))),
        }
    }

    pub fn failed(&self, ip: IpAddr, now: Instant) {
        let mut g = self.inner.lock().unwrap();
        Self::prune(&mut g, now);
        g.per_client.entry(ip).or_default().push(now);
        g.global.push(now);
    }

    pub fn succeeded(&self, ip: IpAddr) {
        self.inner.lock().unwrap().per_client.remove(&ip);
    }
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Authenticated and anonymous sessions live in separate pools.
pub struct Sessions {
    inner: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    pub fn new() -> Self {
        Sessions {
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn live(s: &Session, now: Instant) -> bool {
        now.saturating_duration_since(s.created) < SESSION_TTL
    }

    pub fn create(&self, authenticated: bool, now: Instant) -> (String, Session) {
        let id = token();
        let s = Session {
            csrf: token(),
            authenticated,
            created: now,
        };
        let mut m = self.inner.lock().unwrap();
        m.retain(|_, v| Self::live(v, now));
        let cap = if authenticated {
            MAX_AUTH_SESSIONS
        } else {
            MAX_ANON_SESSIONS
        };
        let mut pool: Vec<(&String, Instant)> = m
            .iter()
            .filter(|(_, v)| v.authenticated == authenticated)
            .map(|(k, v)| (k, v.created))
            .collect();
        if pool.len() >= cap {
            pool.sort_by_key(|(_, c)| *c);
            let evict: Vec<String> = pool[..pool.len() + 1 - cap]
                .iter()
                .map(|(k, _)| (*k).clone())
                .collect();
            for k in evict {
                m.remove(&k);
            }
        }
        m.insert(id.clone(), s.clone());
        (id, s)
    }

    pub fn get(&self, id: &str, now: Instant) -> Option<Session> {
        let mut m = self.inner.lock().unwrap();
        match m.get(id) {
            Some(s) if Self::live(s, now) => Some(s.clone()),
            Some(_) => {
                m.remove(id);
                None
            }
            None => None,
        }
    }

    pub fn anonymous_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap()
            .values()
            .filter(|s| !s.authenticated)
            .count()
    }
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
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

fn set_cookie(id: &str, headers: &HeaderMap) -> (header::HeaderName, HeaderValue) {
    let secure = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("https"));
    (
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie_header(id, secure)).unwrap(),
    )
}

impl AppState {
    fn session(&self, headers: &HeaderMap) -> Option<(String, Session)> {
        let id = cookie_value(headers)?;
        let s = self.sessions.get(&id, Instant::now())?;
        Some((id, s))
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
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let bind = s.config.read().unwrap().dashboard.bind.clone();
    if !host_allowed(host.as_deref(), &bind) {
        return err(StatusCode::MISDIRECTED_REQUEST, "host not allowed");
    }
    let write = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if write {
        let origin = req
            .headers()
            .get(header::ORIGIN)
            .map(|v| v.to_str().unwrap_or("null"));
        if !origin_allowed(origin, host.as_deref().unwrap_or("")) {
            return err(StatusCode::FORBIDDEN, "origin not allowed");
        }
    }
    let exempt = path == "/api/session" || path == "/api/login";
    let sess = s.session(req.headers());
    if !exempt && s.password_required() && !sess.as_ref().is_some_and(|(_, x)| x.authenticated) {
        return err(StatusCode::UNAUTHORIZED, "authentication required");
    }
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
    if write && !s.password_required() && write_needs_password(req.method(), req.uri().path()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "password_required",
                "detail": "set a dashboard password (csr set-password) to use this action",
            })),
        )
            .into_response();
    }
    next.run(req).await
}

pub async fn session(State(s): St, headers: HeaderMap) -> Response {
    let (existing, fresh) = match s.session(&headers) {
        Some(x) => (x, false),
        None => (s.sessions.create(false, Instant::now()), true),
    };
    let (id, sess) = existing;
    let body = Json(json!({
        "csrf_token": sess.csrf,
        "authenticated": sess.authenticated || !s.password_required(),
        "password_required": s.password_required(),
    }));
    if fresh {
        ([set_cookie(&id, &headers)], body).into_response()
    } else {
        body.into_response()
    }
}

static ARGON: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub async fn login(
    State(s): St,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let ip = addr.ip();
    if let Err(wait) = s.login_limiter.check(ip, Instant::now()) {
        let secs = wait.as_secs().max(1);
        let mut r = err(StatusCode::TOO_MANY_REQUESTS, "too many failed logins, try later");
        r.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(secs));
        return r;
    }
    let pw = body.get("password").and_then(Value::as_str).unwrap_or("");
    let hash = s.config.read().unwrap().dashboard.password_hash.clone();
    if hash.is_empty() {
        return err(StatusCode::BAD_REQUEST, "no password is set");
    }
    let pw = pw.to_string();
    let _permit = ARGON.acquire().await.ok();
    let ok = tokio::task::spawn_blocking(move || crate::password::verify(&pw, &hash))
        .await
        .unwrap_or(false);
    drop(_permit);
    if !ok {
        s.login_limiter.failed(ip, Instant::now());
        tokio::time::sleep(Duration::from_millis(500)).await;
        tracing::warn!("dashboard login failed from {ip}");
        return err(StatusCode::UNAUTHORIZED, "wrong password");
    }
    s.login_limiter.succeeded(ip);
    // New session id and CSRF token on privilege change.
    let (id, sess) = s.sessions.create(true, Instant::now());
    tracing::info!("dashboard login ok");
    ([set_cookie(&id, &headers)], Json(json!({"csrf_token": sess.csrf}))).into_response()
}

#[cfg(test)]
#[path = "../../../acceptance/aiworker-csr-dashboard-security/dashboard_security_tests.rs"]
mod aiworker_dashboard_security_tests;
