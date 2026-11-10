// Runs the runner's unit tests for the catalog and checks that every named test exists and passes.
import { spawnSync } from 'node:child_process';

const wanted = [
  'aiworker_service_id_tests::single_binary_service_id_is_the_code_hash',
  'aiworker_service_id_tests::composite_service_id_with_args_and_data',
  'aiworker_service_id_tests::composite_service_id_with_data_only',
  'catalog::tests::decode_rejects_oversized_fields',
  'catalog::tests::decode_rejects_non_https_links',
  'catalog::tests::scan_upserts_and_marks_removed',
  'catalog::tests::cut_off_list_never_marks_removed',
  'catalog::tests::first_scan_marks_all_seen',
  'catalog::tests::new_manifest_after_first_scan_is_unseen',
  'catalog::tests::update_of_added_service_is_not_a_notice',
  'catalog::tests::min_csr_above_version_is_not_runnable',
  'catalog::tests::unknown_lists_job_service_ids_without_manifest',
  'registry::tests::add_from_manifest_keeps_existing_status',
  'registry::tests::add_from_manifest_approve_updates_allowed_services',
  'keeper::tests::composite_service_gets_data_cells_and_args',
  'keeper::tests::single_binary_service_unchanged',
  'keeper::tests::dead_data_cell_fails_the_run',
];
const r = spawnSync('bash', ['acceptance/a1/cargo.sh', 'test', '-p', 'csr', '--bin', 'csr', '--', '--test-threads=1'],
  { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
const out = `${r.stdout}\n${r.stderr}`;
const passed = [...out.matchAll(/^test (\S+) \.\.\. ok$/gm)].map((m) => m[1]);
const missing = wanted.filter((w) => !passed.some((p) => p.endsWith(w)));
if (r.status !== 0 || missing.length) {
  console.log(out.slice(-30000));
  if (r.status !== 0) console.error(`cargo test exited with ${r.status}`);
  if (missing.length) console.error(`required tests missing or not passing:\n  ${missing.join('\n  ')}`);
  process.exit(1);
}
console.log(`all ${wanted.length} required runner tests passed`);
