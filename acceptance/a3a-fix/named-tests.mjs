// Runs the contract test suite and checks that every required test exists and passes.
// Usage: node acceptance/a3a-fix/named-tests.mjs <group> [<group> ...]
import { spawnSync } from 'node:child_process';

const GROUPS = {
  harness: ['harness_deploys_all_contracts'],
  float: [
    'float_runner_tip_deposit_ok',
    'float_runner_merge_two_floats_ok',
    'float_runner_redirect_to_other_lock_rejected',
    'float_runner_two_float_outputs_rejected',
    'float_runner_changed_args_rejected',
    'float_runner_without_runner_input_rejected',
    'float_owner_withdraw_all_ok',
    'float_without_any_key_input_rejected',
    'float_runner_adds_type_script_rejected',
    'float_runner_sets_data_rejected',
    'float_runner_capacity_loss_rejected',
    'float_runner_no_job_drain_rejected',
  ],
  registry: [
    'registry_create_with_valid_attestation_ok',
    'registry_create_without_attestation_rejected',
    'registry_create_wrong_monitor_key_rejected',
    'registry_create_expired_attestation_rejected',
    'registry_create_without_config_cell_dep_rejected',
    'registry_create_attestation_for_other_pubkey_rejected',
    'registry_create_with_mismatched_runner_lock_rejected',
    'registry_runner_refreshes_attestation_ok',
    'registry_runner_updates_prekeys_ok',
    'registry_runner_changes_float_lock_hash_rejected',
    'registry_runner_changes_service_pubkey_rejected',
    'registry_runner_reduces_capacity_rejected',
    'registry_runner_destroys_cell_rejected',
    'registry_owner_changes_float_lock_hash_ok',
    'registry_owner_destroys_cell_ok',
    'registry_create_without_runner_key_rejected',
    'registry_create_with_copied_attestation_rejected',
    'registry_owner_changes_service_pubkey_without_new_runner_key_rejected',
    'registry_owner_changes_service_pubkey_with_new_runner_key_ok',
  ],
  job: [
    'job_create_underfunded_rejected',
    'job_create_multi_run_non_every_rejected',
    'job_ondemand_run_tip_to_registered_float_ok',
    'job_tip_to_unregistered_float_rejected',
    'job_tip_too_small_rejected',
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
if (r.status !== 0 || missing.length > 0) {
  console.log(out.slice(-30000));
  if (r.status !== 0) console.error(`cargo test exited with ${r.status}`);
  if (missing.length) console.error(`required tests missing or not passing:\n  ${missing.join('\n  ')}`);
  process.exit(1);
}
console.log(`all ${wanted.length} required tests passed (${passed.length} tests passed in total)`);
