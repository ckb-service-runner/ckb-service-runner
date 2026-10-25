use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_types::prelude::{Builder, Entity, Unpack};
use csr_contract_tests::*;

/// Loads all five binaries and verifies a trivial transaction using each of them
/// (plus a signed spend of the system lock and a `csr-config-type` config cell created by a transaction).
#[test]
fn harness_deploys_all_contracts() {
    let mut env = Env::new();
    let key = env.new_key();
    let monitor = env.new_key();
    for name in CONTRACT_NAMES {
        assert!(!load_contract(name).is_empty(), "{name}");
    }
    assert_ne!(load_job_lock_variant(), load_contract("csr-job-lock"), "the job lock variant is byte-different");

    // config type: create the monitor config cell (Type ID of output 0), then use it as a cell dep
    let wallet = env.min_cell(1000 * CKB, monitor.lock.clone(), None, Bytes::new());
    let input = env.input(&wallet, 0);
    let config_id = type_id(&input, 0);
    let config_type = env.config_type_script(&config_id);
    let data = config_data(&monitor.pubkey, 1_000);
    let cap = min_capacity(&monitor.lock, Some(&config_type), data.len());
    let wallet_cap = min_capacity(&monitor.lock, None, 0) + 1000 * CKB;
    let tx = tx_with(
        vec![input],
        vec![
            (output(cap, monitor.lock.clone(), Some(config_type.clone())), data.clone()),
            (output(wallet_cap - cap - 100_000, monitor.lock.clone(), None), Bytes::new()),
        ],
    );
    env.verify_signed(tx, &[&monitor]).expect("config cell creation");
    let config_op = env.cell(cap, monitor.lock.clone(), Some(config_type), data);

    // system lock: a signed key cell spent into a new key cell
    let input = env.min_cell(10 * CKB, key.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&input, 0)],
        vec![(output(10 * CKB, key.lock.clone(), None), Bytes::new())],
    );
    env.verify_signed(tx, &[&key]).expect("system lock spend");

    // one transaction per contract: the lock contract guards the input, type contracts sit on in/out
    let owner = key.lock_hash();
    let runner = monitor.lock_hash();
    {
        let lock = env.dual_lock_script(&owner, &runner);
        let input = env.min_cell(10 * CKB, lock.clone(), None, Bytes::new());
        // owner key cell so that the dual lock is satisfied
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

    // registry: a valid creation (monitor-signed attestation, DAO deposit of the owner as the first cell dep,
    // config cell as dep, dual-lock output)
    let reg_type = env.registry_type_script(&config_id);
    let dual = env.dual_lock_script(&owner, &key.lock_hash());
    let stake = env.stake(key.lock.clone());
    let fields = RegistryFields {
        peer_id: b"12D3KooWHarness".to_vec(),
        service_pubkey: key.pubkey,
        payout_lock_hash: [3u8; 32],
        network: 2,
        stake: stake.clone(),
        endpoint: b"https://203.0.113.7:8131".to_vec(),
        prekeys: Vec::new(),
        registry_type_hash: reg_type.calc_script_hash().unpack(),
    };
    let data = fields.signed_data(&monitor, 2_000);
    let wallet = env.min_cell(1000 * CKB, key.lock.clone(), None, Bytes::new());
    let cap = min_capacity(&dual, Some(&reg_type), data.len());
    let tx = tx_with(vec![env.input(&wallet, 0)], vec![(output(cap, dual, Some(reg_type)), data)])
        .as_advanced_builder()
        .cell_dep(cell_dep(&stake))
        .cell_dep(cell_dep(&config_op))
        .build();
    env.verify_signed(tx, &[&key]).expect("registry creation");

    // job type + job lock: create an OnDemand job, then refund it after expiry (refund path)
    let job_lock = env.job_lock_script(&[7u8; 32]);
    let job_type = env.job_type_script(&[9u8; 32]);
    let job = JobFields {
        trigger: Trigger::OnDemand,
        tip_shannons: CKB,
        max_fee_shannons: 1_000_000,
        max_cycles: 1_000_000,
        payload: Vec::new(),
        expiry_block: 100,
        refund_lock_hash: key.lock_hash(),
        remaining_runs: 1,
    };
    let cap = job.required_capacity(&job_lock, &job_type);
    let wallet = env.min_cell(1000 * CKB, key.lock.clone(), None, Bytes::new());
    let tx = tx_with(vec![env.input(&wallet, 0)], vec![(output(cap, job_lock.clone(), Some(job_type.clone())), job.data())]);
    env.verify_signed(tx, &[&key]).expect("job creation");
    let job_cell = env.cell(cap, job_lock, Some(job_type), job.data());
    let key_cell = env.min_cell(10 * CKB, key.lock.clone(), None, Bytes::new());
    let tx = tx_with(
        vec![env.input(&job_cell, since::abs_block(100)), env.input(&key_cell, 0)],
        vec![(output(cap, key.lock.clone(), None), Bytes::new())],
    );
    env.verify_signed(tx, &[&key]).expect("job refund");
}
