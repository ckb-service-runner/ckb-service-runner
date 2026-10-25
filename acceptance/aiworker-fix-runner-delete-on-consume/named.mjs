// Runs the runner's unit tests and checks that every named acceptance test exists and passes.
// Usage: node <this file> <module filter> <test name> [<test name> ...]
import { spawnSync } from 'node:child_process';

const [filter, ...wanted] = process.argv.slice(2);
const r = spawnSync('bash', ['acceptance/a1/cargo.sh', 'test', '-p', 'csr', '--bin', 'csr', '--', filter, '--test-threads=1'],
  { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
const out = `${r.stdout}\n${r.stderr}`;
const passed = [...out.matchAll(/^test (\S+) \.\.\. ok$/gm)].map((m) => m[1]);
const missing = wanted.filter((w) => !passed.some((p) => p.endsWith(`${filter}::${w}`)));
if (r.status !== 0 || missing.length > 0) {
  console.log(out.slice(-30000));
  if (r.status !== 0) console.error(`cargo test exited with ${r.status}`);
  if (missing.length) console.error(`required tests missing or not passing:\n  ${missing.join('\n  ')}`);
  process.exit(1);
}
console.log(`all ${wanted.length} required tests passed`);
