// Acceptance tests for ping-v1-1-runner, written by the spec author.
// Included from runner/src/ping/protocol.rs with #[cfg(test)] #[path = ...] mod aiworker_ping_v11_tests;
use super::*;

const PK7: &str = "0x025cbdf0646e5db4eaa398f365f2ea7a0e3d419b7e0330e39ce92bddedcac4f9bc";
const VECTOR_SIG: &str = "0x5d3e78f3647e5152620f510d89ce95f941f29cd23cf89f92574720aaec2d80cd6349ce6a93f6734e2575556519bab90bb0b9af8271f6418bab178dd0074c65d2";

fn key7() -> SecretKey {
    let mut b = [0u8; 32];
    b[31] = 7;
    SecretKey::from_slice(&b).unwrap()
}

#[test]
fn checkin_message_layout() {
    assert_eq!(
        checkin_message("QmVectorPeer", PK7, "testnet", 1_792_281_600),
        format!("CSR-CHECKIN-v1\nQmVectorPeer\n{PK7}\ntestnet\n1792281600")
    );
}

#[test]
fn checkin_signature_matches_the_shared_vector() {
    let m = checkin_message("QmVectorPeer", PK7, "testnet", 1_792_281_600);
    assert_eq!(sign(&key7(), &m), VECTOR_SIG);
}

#[test]
fn challenge_freshness_and_height() {
    assert_eq!(challenge_status(100, 90, 1000, 1030), ChallengeStatus::Answer { block_known: true });
    assert_eq!(challenge_status(100, 100, 1000, 1000), ChallengeStatus::Answer { block_known: true });
    // Height above the node's tip: answer with an empty block hash instead of refusing.
    assert_eq!(challenge_status(100, 104, 1000, 1000), ChallengeStatus::Answer { block_known: false });
    assert_eq!(challenge_status(100, 90, 1000, 1061), ChallengeStatus::ClockSkew);
    assert_eq!(challenge_status(100, 90, 1061, 1000), ChallengeStatus::ClockSkew);
}

#[test]
fn effective_monitor_list() {
    use crate::config::Config;
    let d = crate::network::NetworkDefaults::for_network("testnet");
    let c = Config::default();
    let m = c.monitors();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].url, d.monitor_url);
    assert_eq!(m[0].pubkey, d.monitor_pubkey);

    let text = format!(
        "[[monitors]]\nurl = \"https://a.example\"\npubkey = \"{PK7}\"\n[[monitors]]\nurl = \"https://b.example\"\npubkey = \"{}\"\n",
        d.monitor_pubkey
    );
    let c = Config::parse(&text).unwrap();
    let urls: Vec<String> = c.monitors().into_iter().map(|m| m.url).collect();
    assert_eq!(urls, vec!["https://a.example".to_string(), "https://b.example".to_string()]);

    // The old [monitor] section still works as a one-entry list.
    let c = Config::parse(&format!("[monitor]\nurl = \"https://old.example\"\npubkey = \"{PK7}\"\n")).unwrap();
    assert_eq!(c.monitors().len(), 1);
    assert_eq!(c.monitors()[0].url, "https://old.example");
    // Both at once is refused.
    assert!(Config::parse(&format!("[monitor]\nurl = \"https://old.example\"\npubkey = \"{PK7}\"\n{text}")).is_err());
}

#[test]
fn attest_errors_are_explained() {
    use crate::attest::explain_error;
    for (status, err, reason, want) in [
        (403, "stake_in_use", None, "another registration"),
        (403, "stake_invalid", Some("spent"), "spent"),
        (403, "stake_invalid", Some("not_dao"), "Nervos DAO"),
        (403, "stake_invalid", Some("not_deposit"), "deposit"),
        (403, "stake_invalid", Some("too_small"), "10,000"),
        (403, "stake_invalid", Some("not_found"), "not found"),
        (403, "stake_mismatch", None, "registry"),
        (403, "not_eligible", None, "24 hours"),
        (400, "wrong_network", None, "network"),
        (401, "bad_signature", None, "signature"),
        (503, "chain_unavailable", None, "try again"),
        (503, "clock_stale", None, "try again"),
        (429, "rate_limited", None, "try again"),
    ] {
        let m = explain_error(status, err, reason);
        assert!(m.contains(&format!("{status} {err}")), "{m}");
        assert!(m.to_lowercase().contains(&want.to_lowercase()), "{status} {err}: {m}");
    }
}
