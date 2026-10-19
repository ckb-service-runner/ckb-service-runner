// Attestations signed by the CSR Network Monitor code (csrmon.csrchain.sign_attestation).
// Monitor key 0x4d…4d, other key 0x5e…5e, service key 0x2b…2b, peer id = the mock node's id.
export const V = {
  "service_key": "2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b",
  "service_pubkey": "0x02bb58b5feca505c74edc000d8282fc556e51a1024fc8e7d7e56c6f887c5c8d5f2",
  "monitor_pubkey": "0x0371102fc86b5c576c72f411e083cc03eb83d1b55065406ba2a483208dbb5074ab",
  "other_pubkey": "0x035f64993ccb4044a005e82ba07e78b2815a6a17aa7d5ad62fd59d519778072483",
  "peer_id": "QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
  "ok": {
    "peer_id": "QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    "service_pubkey": "0x02bb58b5feca505c74edc000d8282fc556e51a1024fc8e7d7e56c6f887c5c8d5f2",
    "network": "testnet",
    "network_id": 1,
    "expiry_ms": 4102444800000,
    "signature": "0xe226c07a89916158ccaeb5a5b700f8540552828d712818d20951c4e4b5c053ec64ca88682e2e896b558e860f5122532bda1c550ddca086de3c5da83c0595cc9e01",
    "monitor_pubkey": "0x0371102fc86b5c576c72f411e083cc03eb83d1b55065406ba2a483208dbb5074ab"
  },
  "expired": {
    "peer_id": "QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    "service_pubkey": "0x02bb58b5feca505c74edc000d8282fc556e51a1024fc8e7d7e56c6f887c5c8d5f2",
    "network": "testnet",
    "network_id": 1,
    "expiry_ms": 1000000000000,
    "signature": "0x97d967302334a8b5a504811417ada149c15c9425d3ecbc2a2d6112e62709b37904830214ed7f9827d417ff9e1b74071701f1c79b4294386258576ad2d805b7b100",
    "monitor_pubkey": "0x0371102fc86b5c576c72f411e083cc03eb83d1b55065406ba2a483208dbb5074ab"
  },
  "badsig": {
    "peer_id": "QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    "service_pubkey": "0x02bb58b5feca505c74edc000d8282fc556e51a1024fc8e7d7e56c6f887c5c8d5f2",
    "network": "testnet",
    "network_id": 1,
    "expiry_ms": 4102444800000,
    "signature": "0x107eeea22432049bc6b9c696a2658d1b0cb1dec0798ecd12be4a46847405c5ca0ac9f18c584b9a2018110f406029d0b68dd79d4b01beb9759007d2f338098cc200",
    "monitor_pubkey": "0x0371102fc86b5c576c72f411e083cc03eb83d1b55065406ba2a483208dbb5074ab"
  },
  "otherpeer": {
    "peer_id": "QmSomeOtherNode",
    "service_pubkey": "0x02bb58b5feca505c74edc000d8282fc556e51a1024fc8e7d7e56c6f887c5c8d5f2",
    "network": "testnet",
    "network_id": 1,
    "expiry_ms": 4102444800000,
    "signature": "0x9eed291c3d996f872ba8672324472dac6520abc9e7e72375110f7cff007fcf27660ece140064155a36b094c12f0a87b66e5ed69bb93b67c32d42296c6894a8cd01",
    "monitor_pubkey": "0x0371102fc86b5c576c72f411e083cc03eb83d1b55065406ba2a483208dbb5074ab"
  },
  "mainnet": {
    "peer_id": "QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    "service_pubkey": "0x02bb58b5feca505c74edc000d8282fc556e51a1024fc8e7d7e56c6f887c5c8d5f2",
    "network": "mainnet",
    "network_id": 0,
    "expiry_ms": 4102444800000,
    "signature": "0xcd93904a21bbe67190cd1ccf24225add0ee4bae3fb4bf9e45d3b7fd1eaf414577136fc6d5ce09feb1cade6ea52d990ebea611961ae0d4741c5b4b4d62751441200",
    "monitor_pubkey": "0x0371102fc86b5c576c72f411e083cc03eb83d1b55065406ba2a483208dbb5074ab"
  }
};
