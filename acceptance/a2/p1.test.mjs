import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, setup, runCsr, cleanup } from '../a1/lib.mjs';
import { runService, asciiHex, manifest } from './svc.mjs';

after(cleanup);

test('hello-rust returns hello:<payload>, deterministically', async () => {
  const env = await setup('a2-hello');
  const a = await runService(env, 'hello-rust', '0x01020304');
  assert.equal(a.status, 'ok', JSON.stringify(a));
  assert.equal(a.exit_code, 0);
  assert.equal(a.result_hex, asciiHex('hello:') + '01020304');
  assert.match(a.code_hash, /^0x[0-9a-f]{64}$/);
  assert.ok(a.cycles > 0);
  const b = await runService(env, 'hello-rust', '0x01020304');
  assert.equal(b.cycles, a.cycles, 'same input must use the same cycles');
  assert.equal(b.code_hash, a.code_hash);
  const empty = await runService(env, 'hello-rust', '0x');
  assert.equal(empty.result_hex, asciiHex('hello:'));
});

test('cycle limit', async () => {
  const env = await setup('a2-cycles');
  const r = await runService(env, 'loop', '0x', ['--max-cycles', '1000000']);
  assert.equal(r.status, 'cycles_exceeded');
});

test('wall-time limit stops a loop without syscalls', async () => {
  const env = await setup('a2-wall');
  const r = await runService(env, 'loop', '0x', ['--max-cycles', '1000000000000', '--max-wall-seconds', '1']);
  assert.equal(r.status, 'timeout');
  assert.ok(r.elapsedMs < 15000, `took ${r.elapsedMs} ms`);
});

test('memory limit', async () => {
  const env = await setup('a2-mem');
  assert.equal((await runService(env, 'memhog')).status, 'memory_exceeded');
});

test('unknown syscall', async () => {
  const env = await setup('a2-sys');
  assert.equal((await runService(env, 'badsyscall')).status, 'unknown_syscall');
});

test('oversized result', async () => {
  const env = await setup('a2-big');
  assert.equal((await runService(env, 'bigresult')).status, 'result_too_large');
});

test('invalid ELF', async () => {
  const env = await setup('a2-elf');
  const m = path.join(env.dir, 'bad.toml');
  fs.writeFileSync(m, `name = "bad"\nlanguage = "rust"\nelf = ${JSON.stringify(path.join(ROOT, 'README.md'))}\n`);
  assert.equal((await runService(env, m)).status, 'invalid_elf');
});

test('missing manifest is a CLI error', async () => {
  const env = await setup('a2-cli');
  const r = await runCsr([...env.args, 'service', 'run-local', '--manifest', manifest('does-not-exist'), '--payload', '0x']);
  assert.equal(r.code, 1);
});
