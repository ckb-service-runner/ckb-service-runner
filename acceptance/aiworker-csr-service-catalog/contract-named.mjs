// Runs the contract tests and checks that every named manifest test exists and passes.
import { spawnSync } from 'node:child_process';

const wanted = [
  'manifest_create_ok', 'manifest_create_with_data_cells_ok', 'manifest_create_without_code_dep_rejected',
  'manifest_create_wrong_code_hash_rejected', 'manifest_create_other_publisher_rejected',
  'manifest_create_wrong_data_hash_rejected', 'manifest_create_bad_version_tag_rejected',
  'manifest_create_name_too_long_rejected', 'manifest_create_description_too_long_rejected',
  'manifest_create_http_url_rejected', 'manifest_create_invalid_utf8_rejected',
  'manifest_create_too_many_data_cells_rejected', 'manifest_create_bad_type_id_rejected',
  'manifest_update_text_ok', 'manifest_update_deprecate_ok', 'manifest_update_code_hash_rejected',
  'manifest_update_args_rejected', 'manifest_update_lock_rejected', 'manifest_two_outputs_rejected',
  'manifest_destroy_ok',
];
const r = spawnSync('cargo', ['test', '--manifest-path', 'contracts/Cargo.toml', '-p', 'csr-contract-tests',
  '--test', 'manifest', '--', '--test-threads=1'], { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
const out = `${r.stdout}\n${r.stderr}`;
const passed = [...out.matchAll(/^test (\S+) \.\.\. ok$/gm)].map((m) => m[1]);
const missing = wanted.filter((w) => !passed.some((p) => p.endsWith(w)));
if (r.status !== 0 || missing.length) {
  console.log(out.slice(-30000));
  if (missing.length) console.error(`required tests missing or not passing:\n  ${missing.join('\n  ')}`);
  process.exit(1);
}
console.log(`all ${wanted.length} required manifest tests passed`);
