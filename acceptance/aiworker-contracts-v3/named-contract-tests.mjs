// Runs the contract test suite and checks that every required test exists and passes.
// Usage: node <this file> <group> [<group> ...]
import { spawnSync } from 'node:child_process';

const GROUPS = {
  registry: [
    'registry_v3_create_ok',
    'registry_v3_data_matches_vector',
    'registry_v2_data_rejected',
    'registry_attest_v1_signature_rejected',
    'registry_attest_v2_other_stake_rejected',
    'registry_attest_v2_other_registry_type_rejected',
    'registry_attestation_lifetime_over_cap_rejected',
    'registry_attestation_network_mismatch_rejected',
    'registry_runner_refresh_v3_ok',
    'registry_runner_changes_endpoint_ok',
    'registry_owner_changes_peer_id_without_consent_rejected',
    'registry_owner_changes_network_without_consent_rejected',
    'registry_owner_changes_service_pubkey_without_consent_rejected',
    'registry_owner_changes_peer_id_with_consent_ok',
    'registry_runner_owned_short_args_ok',
    'registry_type_args_length_rejected',
    'registry_short_config_data_rejected',
  ],
  config: [
    'config_create_ok',
    'config_clock_forward_ok',
    'config_clock_same_ok',
    'config_clock_backward_rejected',
    'config_monitor_key_change_rejected',
    'config_destroy_rejected',
    'config_second_cell_same_type_rejected',
  ],
  job: [
    'job_lock_attestation_lifetime_over_cap_rejected',
    'job_lock_refund_at_input_index_ok',
    'job_lock_refund_at_other_index_rejected',
    'job_two_lock_versions_share_refund_rejected',
    'job_lock_args_length_rejected',
    'job_lock_bad_job_data_rejected',
    'job_lock_short_config_data_rejected',
    'job_type_args_length_rejected',
    'job_type_two_new_jobs_one_group_rejected',
  ],
  harness: ['harness_deploys_all_contracts'],
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
