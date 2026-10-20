// Runs the contract test suite and checks that every required test exists and passes.
// Usage: node acceptance/a8/named-tests.mjs <group> [<group> ...]
import { spawnSync } from 'node:child_process';

const GROUPS = {
  consent: [
    'registry_create_with_consent_signature_ok',
    'registry_create_with_stake_owner_input_ok',
    'registry_create_without_stake_owner_consent_rejected',
    'registry_create_consent_wrong_length_rejected',
    'registry_create_consent_bad_recovery_id_rejected',
    'registry_create_consent_high_s_ok',
    'registry_create_consent_v27_ok',
    'registry_create_consent_by_other_key_rejected',
    'registry_create_consent_for_other_stake_rejected',
    'registry_create_consent_for_other_service_key_rejected',
    'registry_create_consent_for_other_peer_id_rejected',
    'registry_create_consent_for_other_network_rejected',
    'registry_create_consent_without_magic_prefix_rejected',
    'registry_create_consent_in_input_type_rejected',
    'registry_owner_changes_stake_with_consent_ok',
    'registry_owner_changes_stake_without_consent_rejected',
    'registry_owner_changes_owner_without_stake_consent_rejected',
    'registry_owner_changes_owner_with_stake_owner_input_ok',
    'registry_runner_refresh_needs_no_consent_ok',
    'registry_consent_text_vector',
    'registry_consent_neuron_vector',
  ],
  runner_owned: [
    'registry_create_owned_by_runner_ok',
    'registry_runner_owned_refresh_ok',
    'registry_runner_owned_changes_stake_with_consent_ok',
    'registry_runner_owned_changes_stake_without_consent_rejected',
    'registry_runner_owned_destroy_ok',
    'job_tip_to_runner_wallet_ok',
    'job_tip_to_runner_wallet_paying_fee_rejected',
  ],
  harness: ['harness_deploys_all_contracts'],
  registry: [
    'registry_create_with_valid_attestation_ok',
    'registry_create_without_runner_key_rejected',
    'registry_create_with_copied_attestation_rejected',
    'registry_create_without_attestation_rejected',
    'registry_create_wrong_monitor_key_rejected',
    'registry_create_expired_attestation_rejected',
    'registry_create_without_config_cell_dep_rejected',
    'registry_create_attestation_for_other_pubkey_rejected',
    'registry_create_with_mismatched_runner_lock_rejected',
    'registry_runner_refreshes_attestation_ok',
    'registry_runner_updates_prekeys_ok',
    'registry_runner_changes_payout_lock_hash_rejected',
    'registry_runner_changes_service_pubkey_rejected',
    'registry_runner_reduces_capacity_rejected',
    'registry_runner_destroys_cell_rejected',
    'registry_owner_changes_payout_lock_hash_ok',
    'registry_owner_destroys_cell_ok',
    'registry_owner_changes_service_pubkey_without_new_runner_key_rejected',
    'registry_owner_changes_service_pubkey_with_new_runner_key_ok',
    'registry_v1_data_rejected',
    'registry_v2_data_matches_monitor_vector',
  ],
  stake: [
    'registry_create_stake_exactly_min_ok',
    'registry_create_without_stake_dep_rejected',
    'registry_create_stake_dep_after_dep_group_rejected',
    'registry_create_stake_dep_is_dep_group_rejected',
    'registry_create_stake_outpoint_mismatch_rejected',
    'registry_create_stake_not_dao_rejected',
    'registry_create_stake_dao_args_not_empty_rejected',
    'registry_create_stake_withdrawing_rejected',
    'registry_create_stake_below_min_rejected',
    'registry_create_stake_of_other_owner_without_consent_rejected',
    'registry_runner_changes_stake_rejected',
    'registry_runner_refresh_without_stake_dep_ok',
    'registry_owner_changes_stake_with_new_deposit_ok',
    'registry_owner_changes_stake_without_dep_rejected',
  ],
  job: [
    'job_create_underfunded_rejected',
    'job_create_multi_run_non_every_rejected',
    'job_ondemand_run_tip_to_registered_payout_ok',
    'job_tip_to_unregistered_payout_rejected',
    'job_tip_too_small_rejected',
    'job_run_with_v1_registry_dep_rejected',
    'job_at_block_before_trigger_rejected',
    'job_at_block_after_trigger_ok',
    'job_at_timestamp_before_trigger_rejected',
    'job_every_run_recreates_with_runs_minus_one_ok',
    'job_every_run_too_early_rejected',
    'job_every_run_without_recreation_rejected',
    'job_every_run_with_changed_payload_rejected',
    'job_every_run_siphons_capacity_rejected',
    'job_last_run_refunds_remainder_ok',
    'job_last_run_without_refund_rejected',
    'job_refund_after_expiry_ok',
    'job_refund_before_expiry_rejected',
    'job_refund_without_refund_lock_input_rejected',
    'job_run_without_registered_runner_key_rejected',
    'job_run_with_forged_registry_rejected',
    'job_run_with_expired_attestation_rejected',
    'job_run_without_config_dep_rejected',
    'job_run_with_valid_attestation_ok',
  ],
  dual: [
    'dual_lock_without_key_cell_rejected',
    'dual_lock_bad_args_rejected',
    'dual_lock_runner_key_cell_ok',
  ],
};

const groups = process.argv.slice(2);
const wanted = groups.flatMap((g) => {
  if (!GROUPS[g]) { console.error(`unknown group ${g}`); process.exit(2); }
  return GROUPS[g];
});

const r = spawnSync('bash', ['acceptance/a1/cargo.sh', 'test', '--manifest-path', 'contracts/Cargo.toml', '-p', 'csr-contract-tests', '--', '--test-threads=1'],
  { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
const out = `${r.stdout}\n${r.stderr}`;
const passed = [...out.matchAll(/^test (\S+) \.\.\. ok$/gm)].map((m) => m[1]);
const missing = wanted.filter((w) => !passed.some((p) => p === w || p.endsWith(`::${w}`)));
const floatTests = passed.filter((p) => /(^|::)float_/.test(p));
if (r.status !== 0 || missing.length > 0 || floatTests.length > 0) {
  console.log(out.slice(-30000));
  if (r.status !== 0) console.error(`cargo test exited with ${r.status}`);
  if (missing.length) console.error(`required tests missing or not passing:\n  ${missing.join('\n  ')}`);
  if (floatTests.length) console.error(`float lock tests must be gone:\n  ${floatTests.join('\n  ')}`);
  process.exit(1);
}
console.log(`all ${wanted.length} required tests passed (${passed.length} tests passed in total)`);
