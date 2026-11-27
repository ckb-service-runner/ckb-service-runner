// csr-07-security, phase 4: build provenance workflow and release.sh --ci-dir (S7).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const sha = (f) => crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex');

test('release-build workflow builds from the tag and attests the binaries', () => {
  const w = read('.github', 'workflows', 'release-build.yml');
  assert.match(w, /^name:\s*release-build/m);
  assert.match(w, /tags:\s*\[?\s*['"]?v\*/);
  assert.match(w, /workflow_dispatch/);
  assert.match(w, /runs-on:\s*ubuntu-22\.04/);
  assert.match(w, /runs-on:\s*windows-latest/);
  assert.match(w, /id-token:\s*write/);
  assert.match(w, /attestations:\s*write/);
  assert.match(w, /actions\/attest-build-provenance@v\d+/);
  assert.ok((w.match(/actions\/attest-build-provenance@/g) || []).length >= 2, 'one attestation step per job');
  assert.match(w, /cargo build --release -p csr --locked/);
  assert.match(w, /actions\/upload-artifact@v\d+/);
  assert.match(w, /csr-linux-x86_64-release/);
  assert.match(w, /csr-windows-x86_64-release/);
  assert.match(w, /csr\.exe/);
  assert.match(w, /Cargo\.toml/);
  assert.match(w, /GITHUB_REF_NAME|github\.ref_name/);
  // No secrets: the release signing key is offline.
  assert.doesNotMatch(w, /secrets\./);
});

test('windows.yml keeps working', () => {
  const w = read('.github', 'workflows', 'windows.yml');
  assert.match(w, /csr-windows-x86_64/);
  assert.match(w, /cargo test -p csr --bins/);
});

function runRelease(args) {
  return spawnSync('bash', ['scripts/release.sh', ...args], { encoding: 'utf8', cwd: ROOT });
}

test('release.sh --ci-dir takes the CI files and does not build', () => {
  const ver = read('runner', 'Cargo.toml').match(/^version = "([^"]+)"/m)[1];
  const arch = spawnSync('uname', ['-m'], { encoding: 'utf8' }).stdout.trim();
  fs.mkdirSync(path.join(ROOT, '.acc-tmp'), { recursive: true });
  const dir = fs.mkdtempSync(path.join(ROOT, '.acc-tmp', 'ci-'));
  const bin = path.join(dir, `csr-${ver}-${arch}-linux`);
  const tgz = path.join(dir, `csr-${ver}-linux-x86_64.tar.gz`);
  fs.writeFileSync(bin, 'fake binary built by CI');
  fs.writeFileSync(tgz, 'fake tarball built by CI');
  const r = runRelease([ver, '--ci-dir', dir]);
  try {
    assert.equal(r.status, 0, `${r.stdout}${r.stderr}`);
    const sums = fs.readFileSync(path.join(ROOT, 'dist', 'SHA256SUMS'), 'utf8');
    assert.ok(sums.includes(`${sha(bin)}  csr-${ver}-${arch}-linux`), sums);
    assert.ok(sums.includes(`${sha(tgz)}  csr-${ver}-linux-x86_64.tar.gz`), sums);
    assert.match(r.stdout, /gh attestation verify/);
    assert.equal(fs.readFileSync(path.join(ROOT, 'dist', `csr-${ver}-${arch}-linux`), 'utf8'), 'fake binary built by CI');
  } finally {
    for (const f of [`csr-${ver}-${arch}-linux`, `csr-${ver}-linux-x86_64.tar.gz`, 'SHA256SUMS', 'SHA256SUMS.sig']) {
      fs.rmSync(path.join(ROOT, 'dist', f), { force: true });
    }
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('release.sh --ci-dir fails when a file is missing', () => {
  const ver = read('runner', 'Cargo.toml').match(/^version = "([^"]+)"/m)[1];
  fs.mkdirSync(path.join(ROOT, '.acc-tmp'), { recursive: true });
  const dir = fs.mkdtempSync(path.join(ROOT, '.acc-tmp', 'ci-'));
  try {
    const r = runRelease([ver, '--ci-dir', dir]);
    assert.notEqual(r.status, 0);
    assert.match(r.stdout + r.stderr, /missing|not found|ci-dir/i);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('release.sh keeps the local build path and the version checks', () => {
  const s = read('scripts', 'release.sh');
  assert.match(s, /scripts\/package\.sh|package\.sh/);
  assert.match(s, /runner\/Cargo\.toml/);
  assert.match(s, /--ci-dir/);
  assert.match(s, /SHA256SUMS/);
  assert.doesNotMatch(s, /openssl pkeyutl -sign[^\n]*\.key[^\n]*\n[^\n]*&&/, 'the script signs nothing');
});

test('release docs describe the CI flow and the verification', () => {
  const d = read('docs', 'release.md');
  assert.match(d, /release-build/);
  assert.match(d, /--ci-dir/);
  assert.match(d, /gh attestation verify/);
  assert.match(d, /SHA256SUMS/);
  assert.match(d, /offline/i);
});

test('changelog has an Unreleased section', () => {
  const c = read('CHANGELOG.md');
  assert.match(c, /^## Unreleased/m);
  const sec = c.split(/^## Unreleased/m)[1].split(/^## /m)[0];
  assert.match(sec, /NT SERVICE|service account/i);
  assert.match(sec, /provenance/i);
});
