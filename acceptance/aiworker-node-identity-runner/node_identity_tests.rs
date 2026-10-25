// Acceptance tests for node-identity-runner, written by the spec author.
// Included from runner/src/identity.rs with #[cfg(test)] #[path = ...] mod aiworker_node_identity_tests;
use super::*;

const VECTORS: [(&str, &str, &str); 3] = [
    ("0101010101010101010101010101010101010101010101010101010101010101",
     "031b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f",
     "Qmecbv6CRqPqSyZUvQ8SQErHYAbq4f2Pucq8ofs2MsdZnG"),
    ("0707070707070707070707070707070707070707070707070707070707070707",
     "02989c0b76cb563971fdc9bef31ec06c3560f3249d6ee9e5d83c57625596e05f6f",
     "QmS7sXpV62pY1jHiAkiVkftGxJvimJZTLdLuzerj7yafdS"),
    ("4242424242424242424242424242424242424242424242424242424242424242",
     "0324653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c",
     "QmZdgEgD3Gc5y6DcejPXiy6r9VVbu6Vf3jNVcnUCJ5iKmp"),
];
const SERVICE_PK9: &str = "0x03acd484e2f0c7f65309ad178a9f559abde09796974c57e714c35f110dfc27ccbe";
const P2P_SIG: &str = "0x5d7a192e210313cf1e33abef3505f216bf473b367520941251dbf0809088e31d543c7af4ceb44e2d77858816790e3fada41567f727b3c1c040f10e679e9b2ac9";
const SERVICE_SIG: &str = "0xda44c7262b1339864c03ee6cbc29247ebc4417c93d70fecdf4eafa3db6bda74827bfbab7df07788261bda7cdc3c07c95d3ab575b6a8f7e23f6975b280c648069";

#[test]
fn peer_id_is_derived_like_ckb() {
    for (_, pk, peer) in VECTORS {
        let pk: [u8; 33] = hex::decode(pk).unwrap().try_into().unwrap();
        assert_eq!(peer_id_from_pubkey(&pk), peer);
    }
}

#[test]
fn p2p_key_file_formats() {
    let raw = [7u8; 32];
    let k = parse_p2p_key(&raw).unwrap();
    assert_eq!(hex::encode(k.public_key_bytes()), VECTORS[1].1);
    let text = format!("{}\n", VECTORS[1].0);
    assert_eq!(parse_p2p_key(text.as_bytes()).unwrap().peer_id(), VECTORS[1].2);
    assert!(parse_p2p_key(&[7u8; 31]).is_err());
    assert!(parse_p2p_key(b"not a key").is_err());
    assert!(parse_p2p_key(&[0u8; 32]).is_err(), "zero is not a valid secret key");
}

#[test]
fn identity_proof_matches_the_shared_vector() {
    let p2p = parse_p2p_key(&[7u8; 32]).unwrap();
    let mut s = [0u8; 32];
    s[31] = 9;
    let service = secp256k1::SecretKey::from_slice(&s).unwrap();
    let msg = identity_message(VECTORS[1].2, SERVICE_PK9, "testnet", 1_792_281_600);
    assert_eq!(msg, format!("CSR-NODE-IDENTITY-v1\n{}\n{SERVICE_PK9}\ntestnet\n1792281600", VECTORS[1].2));
    let proof = identity_proof(&p2p, &service, SERVICE_PK9, "testnet", 1_792_281_600);
    assert_eq!(proof["peer_id"], VECTORS[1].2);
    assert_eq!(proof["p2p_pubkey"], format!("0x{}", VECTORS[1].1));
    assert_eq!(proof["p2p_sig"], P2P_SIG);
    assert_eq!(proof["service_sig"], SERVICE_SIG);
    assert_eq!(proof["ts"], 1_792_281_600);
}

#[test]
fn monitor_p2p_address_is_split() {
    assert_eq!(
        parse_monitor_p2p("/ip4/203.0.113.7/tcp/8115/p2p/QmS7sXpV62pY1jHiAkiVkftGxJvimJZTLdLuzerj7yafdS"),
        Some(("QmS7sXpV62pY1jHiAkiVkftGxJvimJZTLdLuzerj7yafdS".to_string(), "/ip4/203.0.113.7/tcp/8115".to_string()))
    );
    assert_eq!(
        parse_monitor_p2p("/dns4/node.example/tcp/8115/p2p/QmS7sXpV62pY1jHiAkiVkftGxJvimJZTLdLuzerj7yafdS").map(|x| x.1),
        Some("/dns4/node.example/tcp/8115".to_string())
    );
    assert_eq!(parse_monitor_p2p("/ip4/203.0.113.7/tcp/8115"), None);
    assert_eq!(parse_monitor_p2p("http://x"), None);
    assert_eq!(parse_monitor_p2p(""), None);
}
