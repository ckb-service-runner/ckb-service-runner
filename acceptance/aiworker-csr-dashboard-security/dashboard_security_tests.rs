// Acceptance tests for csr-dashboard-security, written by the spec author.
// Included from runner/src/web/auth.rs with #[cfg(test)] #[path = ...] mod aiworker_dashboard_security_tests;
use super::*;
use axum::http::Method;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

#[test]
fn money_and_settings_writes_need_a_password() {
    for p in ["/api/wallet/send", "/api/registry/create", "/api/registry/remove", "/api/settings", "/api/services/0xab/approve"] {
        let m = if p == "/api/settings" { Method::PUT } else { Method::POST };
        assert!(write_needs_password(&m, p), "{p}");
    }
    assert!(!write_needs_password(&Method::GET, "/api/wallet"));
    assert!(!write_needs_password(&Method::GET, "/api/settings"));
    assert!(!write_needs_password(&Method::POST, "/api/registry/consent-message"));
    assert!(!write_needs_password(&Method::POST, "/api/login"));
    assert!(!write_needs_password(&Method::GET, "/api/session"));
}

#[test]
fn host_header_must_name_the_loopback_dashboard() {
    let bind = "127.0.0.1:8130";
    assert!(host_allowed(Some("127.0.0.1:8130"), bind));
    assert!(host_allowed(Some("localhost:8130"), bind));
    assert!(host_allowed(Some("[::1]:8130"), bind));
    assert!(!host_allowed(Some("evil.example:8130"), bind));
    assert!(!host_allowed(Some("localhost:9999"), bind));
    assert!(!host_allowed(None, bind));
    // A non-loopback bind is protected by its password, not by the host name.
    assert!(host_allowed(Some("node.lan:8130"), "0.0.0.0:8130"));
}

#[test]
fn origin_must_match_the_host_on_writes() {
    assert!(origin_allowed(None, "127.0.0.1:8130"));
    assert!(origin_allowed(Some("http://127.0.0.1:8130"), "127.0.0.1:8130"));
    assert!(origin_allowed(Some("https://node.lan:8130"), "node.lan:8130"));
    assert!(!origin_allowed(Some("http://evil.example"), "127.0.0.1:8130"));
    assert!(!origin_allowed(Some("null"), "127.0.0.1:8130"));
}

#[test]
fn login_failures_are_limited_per_client() {
    let l = LoginLimiter::new();
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7));
    let other = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8));
    let t = Instant::now();
    for _ in 0..LOGIN_FAILURES_PER_CLIENT {
        assert!(l.check(ip, t).is_ok());
        l.failed(ip, t);
    }
    let wait = l.check(ip, t).unwrap_err();
    assert!(wait > Duration::ZERO && wait <= LOGIN_WINDOW);
    assert!(l.check(other, t).is_ok(), "another client is not blocked");
    assert!(l.check(ip, t + LOGIN_WINDOW + Duration::from_secs(1)).is_ok(), "the block ends with the window");
}

#[test]
fn login_failures_are_limited_globally() {
    let l = LoginLimiter::new();
    let t = Instant::now();
    for i in 0..LOGIN_FAILURES_GLOBAL {
        let ip = IpAddr::V4(Ipv4Addr::new(10, 1, (i / 250) as u8, (i % 250) as u8));
        l.failed(ip, t);
    }
    assert!(l.check(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), t).is_err());
}

#[test]
fn success_clears_the_client_counter() {
    let l = LoginLimiter::new();
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7));
    let t = Instant::now();
    for _ in 0..LOGIN_FAILURES_PER_CLIENT - 1 {
        l.failed(ip, t);
    }
    l.succeeded(ip);
    l.failed(ip, t);
    assert!(l.check(ip, t).is_ok());
}

#[test]
fn anonymous_sessions_never_push_out_a_logged_in_one() {
    let s = Sessions::new();
    let t = Instant::now();
    let (op, _) = s.create(true, t);
    for _ in 0..(MAX_ANON_SESSIONS * 3) {
        s.create(false, t);
    }
    assert!(s.get(&op, t).is_some_and(|x| x.authenticated));
    assert!(s.anonymous_count() <= MAX_ANON_SESSIONS);
}

#[test]
fn sessions_expire() {
    let s = Sessions::new();
    let t = Instant::now();
    let (id, _) = s.create(true, t);
    assert!(s.get(&id, t + Duration::from_secs(3600)).is_some());
    assert!(s.get(&id, t + Duration::from_secs(25 * 3600)).is_none());
}

#[test]
fn cookie_is_secure_behind_https() {
    let plain = cookie_header("abc", false);
    assert!(plain.contains("HttpOnly") && plain.contains("SameSite=Strict") && !plain.contains("Secure"));
    assert!(cookie_header("abc", true).contains("; Secure"));
}

#[test]
fn open_key_files_are_detected() {
    assert!(!crate::identity::key_mode_too_open(0o100600));
    assert!(!crate::identity::key_mode_too_open(0o100400));
    assert!(crate::identity::key_mode_too_open(0o100640));
    assert!(crate::identity::key_mode_too_open(0o100604));
}

#[test]
fn sweep_warning_only_without_sweep_address_above_threshold() {
    const CKB: u64 = 100_000_000;
    assert!(crate::wallet::sweep_warning(None, 1499 * CKB, 1500).is_none());
    let w = crate::wallet::sweep_warning(None, 1500 * CKB, 1500).unwrap();
    assert!(w.contains("sweep"), "{w}");
    assert!(crate::wallet::sweep_warning(Some("ckt1qyq"), 9000 * CKB, 1500).is_none());
}
