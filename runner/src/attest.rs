//! `csr register attest`: requests the monitor's attestation (`POST /v1/attest`) and checks it.

use crate::config::{parse_pubkey, Config};
use crate::identity::Identity;
use crate::node::rpc::RpcClient;
use crate::ping::protocol as proto;
use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
use secp256k1::{Message, PublicKey, Secp256k1};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

fn network_id(network: &str) -> Option<u8> {
    match network {
        "mainnet" => Some(0),
        "testnet" => Some(1),
        "devnet" => Some(2),
        _ => None,
    }
}

/// Molecule `AttestationBody` and the signed message (`spec/registry.md` §3).
pub fn attestation_message(peer_id: &[u8], pubkey: &[u8], network: u8, expiry_ms: u64) -> [u8; 32] {
    let p = peer_id.len() as u32;
    let mut body = Vec::new();
    body.extend_from_slice(&(20 + 4 + p + 33 + 1 + 8).to_le_bytes());
    for off in [20, 24 + p, 57 + p, 58 + p] {
        body.extend_from_slice(&off.to_le_bytes());
    }
    body.extend_from_slice(&p.to_le_bytes());
    body.extend_from_slice(peer_id);
    body.extend_from_slice(pubkey);
    body.push(network);
    body.extend_from_slice(&expiry_ms.to_le_bytes());
    let mut hasher = ckb_hash::new_blake2b();
    hasher.update(b"CSR-ATTEST-v1");
    hasher.update(&body);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

/// Checks that `att` is for this node and signed by `monitor`.
pub fn check(
    att: &Value,
    peer_id: &str,
    service_pubkey: &str,
    network: &str,
    monitor: &PublicKey,
    now_ms: u64,
) -> Result<(), String> {
    let text = |k: &str| att.get(k).and_then(Value::as_str).unwrap_or("");
    if text("peer_id") != peer_id {
        return Err(format!(
            "the attestation is for peer \"{}\", this node is {peer_id}",
            text("peer_id")
        ));
    }
    if !text("service_pubkey").eq_ignore_ascii_case(service_pubkey) {
        return Err("the attestation is for a different service key".into());
    }
    if text("network") != network {
        return Err(format!(
            "the attestation is for network \"{}\", this node is on {network}",
            text("network")
        ));
    }
    let expiry_ms = match att.get("expiry_ms").and_then(Value::as_u64) {
        Some(e) if e > now_ms => e,
        _ => return Err("the attestation has expired".into()),
    };
    let not_signed = || "the attestation is not signed by the configured monitor key".to_string();
    let pk = hex::decode(service_pubkey.trim_start_matches("0x")).map_err(|_| not_signed())?;
    let net = network_id(network).ok_or_else(not_signed)?;
    let sig = hex::decode(text("signature").trim_start_matches("0x")).map_err(|_| not_signed())?;
    if sig.len() != 65 {
        return Err(not_signed());
    }
    let msg = attestation_message(peer_id.as_bytes(), &pk, net, expiry_ms);
    let recovered = RecoveryId::from_i32(sig[64] as i32)
        .and_then(|rec| RecoverableSignature::from_compact(&sig[..64], rec))
        .and_then(|s| Secp256k1::verification_only().recover_ecdsa(&Message::from_digest(msg), &s))
        .map_err(|_| not_signed())?;
    if &recovered != monitor {
        return Err(not_signed());
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Requests an attestation from the monitor for the registration staking `stake` (a checked,
/// lower-case `0x<tx>:<index>`), checks it and writes it to `out`.
pub async fn run(cfg: &Config, id: &Identity, stake: &str, out: &Path) -> Result<Value, String> {
    let body = request(cfg, &id.service_key, &id.service_pubkey, stake).await?;
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(&body).map_err(|e| e.to_string())? + "\n";
    std::fs::write(out, text).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(json!({
        "file": out,
        "peer_id": body["peer_id"],
        "expiry_ms": body["expiry_ms"],
    }))
}

/// One explanation for every reply of `POST /v1/attest` (`spec/monitor-public-api.md`).
pub fn explain_error(status: u16, err: &str, reason: Option<&str>) -> String {
    let hint = match (err, reason) {
        ("stake_in_use", _) => "this deposit already backs another registration; use another one",
        ("stake_invalid", Some("spent")) => "the stake cell is already spent; use a live deposit",
        ("stake_invalid", Some("not_found")) => "the stake cell was not found on chain",
        ("stake_invalid", Some("not_dao")) => "the stake cell is not a Nervos DAO cell",
        ("stake_invalid", Some("not_deposit")) => {
            "the stake cell is a Nervos DAO cell but not a deposit"
        }
        ("stake_invalid", Some("too_small")) => "the deposit must be at least 10,000 CKB",
        ("stake_invalid", _) => "the stake must be a live Nervos DAO deposit of at least 10,000 CKB",
        ("stake_mismatch", _) => "the stake is not the stake of the registry cell",
        ("not_eligible", _) => {
            "the node needs successful checks in the last 24 hours; set [ranking] join = true and keep the node running"
        }
        ("wrong_network", _) => "the monitor serves a different network than this node",
        ("bad_signature", _) => "the binding signature was not accepted; check the service key",
        _ => "try again later",
    };
    let mut msg = format!("monitor replied {status} {err}").trim_end().to_string();
    if let Some(r) = reason {
        msg = format!("{msg}: {r}");
    }
    format!("{msg} ({hint})")
}

/// Requests and checks the monitor's attestation for this node and `stake`; returns it.
pub async fn request(
    cfg: &Config,
    service_key: &secp256k1::SecretKey,
    service_pubkey: &str,
    stake: &str,
) -> Result<Value, String> {
    let first = cfg.monitors().into_iter().next().ok_or("no monitor configured")?;
    let base = first.url.trim_end_matches('/');
    let monitor = parse_pubkey(&first.pubkey).map_err(|e| format!("monitor key: {e}"))?;

    let info = RpcClient::new(&cfg.node.rpc_url)
        .call("local_node_info", json!([]))
        .await
        .map_err(|e| format!("cannot reach the node: {e}"))?;
    let peer_id = info
        .get("node_id")
        .and_then(Value::as_str)
        .ok_or("the node reported no node_id")?
        .to_string();

    let binding_sig = proto::sign(
        service_key,
        &proto::binding_message(&peer_id, service_pubkey, &cfg.network),
    );
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = http
        .post(format!("{base}/v1/attest"))
        .json(&json!({
            "peer_id": peer_id,
            "service_pubkey": service_pubkey,
            "network": cfg.network,
            "binding_sig": binding_sig,
            "stake": stake,
        }))
        .send()
        .await
        .map_err(|e| format!("monitor request failed: {}", e.without_url()))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if status.as_u16() != 200 {
        let err = body.get("error").and_then(Value::as_str).unwrap_or("");
        let reason = body.get("reason").and_then(Value::as_str);
        return Err(explain_error(status.as_u16(), err, reason));
    }

    check(
        &body,
        &peer_id,
        service_pubkey,
        &cfg.network,
        &monitor,
        now_ms(),
    )?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::SecretKey;

    const PEER: &str = "QmNode";
    const SVC: &str = "0x0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const NOW: u64 = 1_000;

    fn monitor_key() -> (SecretKey, PublicKey) {
        let sk = SecretKey::from_slice(&[7u8; 32]).unwrap();
        (sk, PublicKey::from_secret_key(&Secp256k1::new(), &sk))
    }

    fn sign(sk: &SecretKey, peer: &str, network: &str, expiry: u64) -> Value {
        let pk = hex::decode(&SVC[2..]).unwrap();
        let msg = attestation_message(peer.as_bytes(), &pk, network_id(network).unwrap(), expiry);
        let (rec, compact) = Secp256k1::new()
            .sign_ecdsa_recoverable(&Message::from_digest(msg), sk)
            .serialize_compact();
        let mut sig = compact.to_vec();
        sig.push(rec.to_i32() as u8);
        json!({
            "peer_id": peer,
            "service_pubkey": SVC,
            "network": network,
            "network_id": network_id(network).unwrap(),
            "expiry_ms": expiry,
            "signature": format!("0x{}", hex::encode(sig)),
        })
    }

    fn run_check(att: &Value, mon: &PublicKey) -> Result<(), String> {
        check(att, PEER, SVC, "testnet", mon, NOW)
    }

    #[test]
    fn round_trip() {
        let (sk, pk) = monitor_key();
        let att = sign(&sk, PEER, "testnet", 5_000);
        assert_eq!(run_check(&att, &pk), Ok(()));
        let mut upper = att.clone();
        upper["service_pubkey"] = json!(SVC.to_uppercase().replace("0X", "0x"));
        assert_eq!(run_check(&upper, &pk), Ok(()));
    }

    #[test]
    fn refusals() {
        let (sk, pk) = monitor_key();
        let good = sign(&sk, PEER, "testnet", 5_000);

        let other = sign(&sk, "QmOther", "testnet", 5_000);
        assert!(run_check(&other, &pk).unwrap_err().contains("QmOther"));

        let mut att = good.clone();
        att["service_pubkey"] = json!(format!("0x02{}", "11".repeat(32)));
        assert!(run_check(&att, &pk).unwrap_err().contains("service key"));

        let mainnet = sign(&sk, PEER, "mainnet", 5_000);
        assert!(run_check(&mainnet, &pk).unwrap_err().contains("network"));

        let expired = sign(&sk, PEER, "testnet", NOW);
        assert!(run_check(&expired, &pk).unwrap_err().contains("expired"));
        let mut att = good.clone();
        att["expiry_ms"] = json!("soon");
        assert!(run_check(&att, &pk).unwrap_err().contains("expired"));

        let stranger = PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[9u8; 32]).unwrap(),
        );
        assert!(run_check(&good, &stranger)
            .unwrap_err()
            .contains("monitor key"));

        let mut tampered = good.clone();
        tampered["expiry_ms"] = json!(6_000);
        assert!(run_check(&tampered, &pk)
            .unwrap_err()
            .contains("monitor key"));
        let mut short = good.clone();
        short["signature"] = json!("0x00");
        assert!(run_check(&short, &pk).unwrap_err().contains("monitor key"));
        assert!(run_check(&Value::Null, &pk).is_err());
    }
}
