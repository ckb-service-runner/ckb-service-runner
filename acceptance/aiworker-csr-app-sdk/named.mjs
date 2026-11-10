// The runner decodes every SDK vector (jobs::tests::js_sdk_vectors_decode must exist and pass).
import { spawnSync } from 'node:child_process';

const name = 'jobs::tests::js_sdk_vectors_decode';
const r = spawnSync('bash', ['acceptance/a1/cargo.sh', 'test', '-p', 'csr', '--bin', 'csr', '--', 'js_sdk_vectors_decode',
  '--test-threads=1'], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
const out = `${r.stdout}\n${r.stderr}`;
if (r.status !== 0 || !new RegExp(`^test ${name.replace(/:/g, ':')} \\.\\.\\. ok$`, 'm').test(out)) {
  console.log(out.slice(-20000));
  console.error(`${name} missing or not passing`);
  process.exit(1);
}
console.log(`${name} passed`);
