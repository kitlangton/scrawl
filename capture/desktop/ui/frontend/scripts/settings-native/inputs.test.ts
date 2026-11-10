import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, mkdir, writeFile, chmod, rm, symlink, readFile} from 'node:fs/promises';
import path from 'node:path';
import {consumerInput, contract, digest, testName} from './inputs.ts';
import {main} from './cli.ts';
import {verifyInventory, probe} from './provider.ts';

test('the independent surface excludes only the audited consumer owners', () => {
  for (const file of ['record/storage/src/policy.ts', 'record/storage/test/new.test.ts',
    'capture/desktop/ui/frontend/src/storage.tsx', 'capture/desktop/ui/frontend/tests/rust-storage.cjs']) assert.equal(consumerInput(file), true, file);
  for (const file of ['build.rs', 'Cargo.lock', '.cargo/config.toml', 'shared/handoff/config_store.rs',
    'capture/desktop/native/Settings.swift', 'capture/desktop/assets/settings/frog.png',
    'enablement/native-build/build.ts', 'public/site/new-unknown-input.ts', 'connect/hosted.rs',
    'record/storage/native.rs', 'record/storage/Cargo.toml', 'capture/desktop/ui/frontend/new.metal',
    'record/storage/native.S', 'record/storage/native.asm', 'capture/desktop/ui/frontend/native.inc',
    'record/storage/src/unknown.bin', 'capture/desktop/ui/frontend/src/new-resource.dat']) assert.equal(consumerInput(file), false, file);
});

test('provider command parsing never falls back to implicit preparation', async () => {
  for (const args of [[], ['test'], ['provider'], ['prepare', '/unadmitted'], ['unknown', '/tmp']]) {
    await assert.rejects(main(args), /no implicit Cargo build/);
  }
  assert.equal(contract, 'storage-provider-linux-v1');
  assert.equal(testName, 'capture::desktop::workflow::agent_capture::tests::storage_electron_rust_roundtrip');
});

test('a missing receipt fails before any build or provider launch', {skip: process.platform !== 'linux'}, async t => {
  const root = await mkdtemp('/tmp/apparent-dev-sn-missing-');
  t.after(() => rm(root, {recursive: true, force: true}));
  await assert.rejects(main(['provider', root]), (error: unknown) => !!error && typeof error === 'object' &&
    'code' in error && error.code === 'ENOENT' && 'path' in error && error.path === path.join(root, 'receipt.json'));
});

test('prepared files reject byte changes, writable output, unknown members and symlinks', async t => {
  const root = await mkdtemp('/tmp/apparent-dev-sn-unit-');
  t.after(() => rm(root, {recursive: true, force: true}));
  const native = path.join(root, 'native'); await mkdir(native, {mode: 0o700});
  const backend = path.join(native, 'backend');
  await writeFile(backend, 'inert test bytes', {mode: 0o500});
  const inventory = [{name: 'backend', sha256: digest('inert test bytes')}];
  await verifyInventory(native, inventory);
  await chmod(backend, 0o700); await assert.rejects(verifyInventory(native, inventory), /file changed/);
  await writeFile(backend, 'changed bytes'); await chmod(backend, 0o500);
  await assert.rejects(verifyInventory(native, inventory), /file changed/);
  await chmod(backend, 0o700); await writeFile(backend, 'inert test bytes'); await chmod(backend, 0o500);
  const unexpected = path.join(native, 'extra'); await writeFile(unexpected, 'not admitted');
  await assert.rejects(verifyInventory(native, inventory), /inventory changed/); await rm(unexpected);
  await rm(backend); await symlink(path.join(root, 'external'), backend);
  await assert.rejects(verifyInventory(native, inventory), /file changed/);
});

test('a stalled probe kills its owned descendants without a reaped-PGID retry', {skip: process.platform !== 'linux'}, async t => {
  const root = await mkdtemp('/tmp/apparent-dev-sn-timeout-');
  t.after(() => rm(root, {recursive: true, force: true}));
  const pids = path.join(root, 'pids');
  const source = `const {spawn}=require('node:child_process');const fs=require('node:fs');
    const child=spawn(process.execPath,['-e','setTimeout(()=>{},2500)'],{stdio:'ignore'});
    fs.writeFileSync(process.argv[1],JSON.stringify([process.pid,child.pid]));setTimeout(()=>{},2500);`;
  assert.throws(() => probe(process.execPath, ['-e', source, pids], 1), (error: unknown) =>
    !!error && typeof error === 'object' && ('signal' in error && error.signal === 'SIGKILL' || 'status' in error && error.status === 137));
  for (const pid of JSON.parse(await readFile(pids, 'utf8')) as number[]) {
    const stat = await readFile(`/proc/${pid}/stat`, 'utf8').catch(error => { if (error.code === 'ENOENT') return ''; throw error; });
    assert.ok(!stat || /^\d+ \(.+\) [ZX] /.test(stat), 'Timed-out probe descendant is not running');
  }
});
