use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_types::prelude::{Builder, Entity};
use csr_contract_tests::*;

/// Loads all five binaries and verifies a trivial transaction using each of them
/// (plus a signed spend of the system lock and a Type ID config cell as cell dep).
#[test]
fn harness_deploys_all_contracts() {
    let mut env = Env::new();
    let key = env.new_key();
    let monitor = env.new_key();
    let (config_op, config_hash) = env.config_cell(&monitor.pubkey, 1_000, monitor.lock.clone());
    assert_ne!(config_hash, [0u8; 32]);

    // system lock: a signed key cell spent into a new key cell
    let input = env.min_cell(10 * CKB, key.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&input, 0)],
        vec![(output(10 * CKB, key.lock.clone(), None), Bytes::new())],
    );
    env.verify_signed(tx, &[&key]).expect("system lock spend");

    // one transaction per contract: lock contracts guard the input, type contracts sit on in/out
    let owner = key.lock_hash();
    let runner = monitor.lock_hash();
    let locks = vec![
        env.float_lock_script(&owner, &runner, 1_000),
        env.dual_lock_script(&owner, &runner),
        env.job_lock_script(&[7u8; 32]),
    ];
    for lock in locks {
        let input = env.min_cell(10 * CKB, lock.clone(), None, Bytes::new());
        // owner key cell so that the real float lock is satisfied
        let owner_cell = env.min_cell(10 * CKB, key.lock.clone(), None, Bytes::new());
        let tx = tx_with(
            vec![env.input(&input, 0), env.input(&owner_cell, 0)],
            vec![(output(10 * CKB, lock, None), Bytes::new()), (output(10 * CKB, key.lock.clone(), None), Bytes::new())],
        )
            .as_advanced_builder()
            .cell_dep(ckb_testtool::ckb_types::packed::CellDep::new_builder().out_point(config_op.clone()).build())
            .build();
        env.verify_signed(tx, &[&key]).expect("lock contract accepts");
    }

    // registry: a valid creation (monitor-signed attestation, config cell as dep, dual-lock output)
    let reg_type = env.registry_type_script(&config_hash);
    let dual = env.dual_lock_script(&owner, &key.lock_hash());
    let fields = RegistryFields {
        peer_id: b"12D3KooWHarness".to_vec(),
        service_pubkey: key.pubkey,
        float_lock_hash: [3u8; 32],
        network: 2,
        prekeys: Vec::new(),
    };
    let data = fields.signed_data(&monitor, 2_000);
    let wallet = env.min_cell(1000 * CKB, key.lock.clone(), None, Bytes::new());
    let cap = min_capacity(&dual, Some(&reg_type), data.len());
    let tx = tx_with(vec![env.input(&wallet, 0)], vec![(output(cap, dual, Some(reg_type)), data)])
        .as_advanced_builder()
        .cell_dep(cell_dep(&config_op))
        .build();
    env.verify_signed(tx, &[&key]).expect("registry creation");

    let always = env.always_success_script(b"");
    let types = vec![env.job_type_script(&[9u8; 32])];
    for type_ in types {
        let input = env.min_cell(10 * CKB, always.clone(), Some(type_.clone()), Bytes::new());
        let tx = tx_with(
            vec![env.input(&input, 0)],
            vec![(output(10 * CKB, always.clone(), Some(type_)), Bytes::new())],
        );
        env.verify(tx).expect("type contract placeholder accepts");
    }
}
