// Acceptance tests for csr-07-security phase 1 (S5 and the shutdown hook), written by the spec author.
// Included from runner/src/run.rs.
use super::*;

#[test]
fn ping_listener_only_for_joined_inbound_modes() {
    assert!(ping_listener_wanted(true, "inbound"));
    assert!(ping_listener_wanted(true, "both"));
    assert!(!ping_listener_wanted(true, "outbound"));
    assert!(!ping_listener_wanted(false, "inbound"));
    assert!(!ping_listener_wanted(false, "both"));
    assert!(!ping_listener_wanted(false, "outbound"));
}

#[test]
fn request_shutdown_ends_shutdown_signal() {
    // Called before shutdown_signal() is awaited: the request must not be lost.
    request_shutdown();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), shutdown_signal())
            .await
            .expect("shutdown_signal returns after request_shutdown");
    });
}
