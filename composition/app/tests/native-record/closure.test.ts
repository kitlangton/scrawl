import assert from 'node:assert/strict';
import { test } from 'node:test';
import { execFileSync } from 'node:child_process';
import { chmod, open, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { readShell } from '../../scripts/artifacts.ts';
import { stageRecordNative } from '../../scripts/record-native.ts';
import { buildShell } from '../../scripts/build.ts';
import { assemble, verifyAssembly } from '../../scripts/assemble.ts';

import { fixture } from './fixture.ts';

test('fixture canonicalizes an aliased OS temporary directory without relaxing artifact admission', async t => {
  const f = await fixture(); t.after(() => rm(f.work, { recursive: true, force: true }));
  const alias = join(f.work, 'temporary-alias');
  await symlink(f.work, alias);
  execFileSync(process.execPath, ['--experimental-strip-types', '--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import {realpath,rm} from 'node:fs/promises';
    import {basename,join} from 'node:path';
    import {fixture} from ${JSON.stringify(new URL('./fixture.ts', import.meta.url).href)};
    import {stageRecordNative} from ${JSON.stringify(new URL('../../scripts/record-native.ts', import.meta.url).href)};
    const f=await fixture();
    try {
      assert.equal(f.work,await realpath(f.work));await stageRecordNative(f.output,f.input);
      const aliased={...f.input,capture:{...f.input.capture,directory:join(process.env.TMPDIR,basename(f.work),'capture')}};
      await assert.rejects(stageRecordNative(f.output,aliased),/must be canonical/);
    }
    finally {await rm(f.work,{recursive:true,force:true});}
  `], { env: { ...process.env, TMPDIR: alias, TMP: alias, TEMP: alias }, stdio: 'pipe', timeout: 10_000 });
});

test('complete native closures preserve manifests, adjacent workers and executable modes; existing output is refused', async t => {
  const f = await fixture(); t.after(() => rm(f.work, { recursive: true, force: true }));
  const modes = await stageRecordNative(f.output, f.input);
  assert.equal(Object.keys(modes).length, 8);
  for (const [path, mode] of Object.entries(modes)) {
    assert.equal((await stat(join(f.output, path))).mode & 0o7777, mode);
    const [, slot, ...source] = path.split('/');
    assert.deepEqual(await readFile(join(f.output, path)), await readFile(join(f.input[slot as 'capture'].directory, ...source)));
  }
  await assert.rejects(stageRecordNative(f.output, f.input), /EEXIST/);
});

test('unadmitted manifest, wrong source/target, changed worker, extra file, symlink and wrong mode fail before staging', async t => {
  for (const change of ['digest', 'source', 'target', 'worker', 'extra', 'symlink', 'mode'] as const) {
    const f = await fixture(); t.after(() => rm(f.work, { recursive: true, force: true }));
    if (change === 'digest') f.input.capture.manifestSha256 = 'b'.repeat(64);
    if (change === 'source') f.input.capture.sourceSha = 'b'.repeat(40);
    if (change === 'target') (f.input as unknown as { target: string }).target = 'macos-x64';
    const worker = join(f.input.capture.directory, 'libexec/worker');
    if (change === 'worker') await writeFile(worker, 'tampered');
    if (change === 'extra') await writeFile(join(f.input.media.directory, 'extra'), 'undeclared');
    if (change === 'symlink') { await rm(worker); await symlink(join(f.input.media.directory, 'libexec/worker'), worker); }
    if (change === 'mode') await chmod(worker, 0o644);
    await assert.rejects(stageRecordNative(f.output, f.input), Error, change);
    await assert.rejects(stat(join(f.output, 'native-record')), { code: 'ENOENT' });
  }
});

test('Record-bound shell and assembled copy seal all native bytes and modes', { skip: process.platform !== 'linux' || process.arch !== 'x64' }, async t => {
  const f = await fixture(); t.after(() => rm(f.work, { recursive: true, force: true }));
  const binding = join(f.work, 'binding.ts');
  await writeFile(binding, 'export function createRecordHostServices() { throw new Error("synthetic build only"); }');
  await buildShell(f.output, undefined, { module: binding, nativeArtifacts: f.input });
  const manifest = await readShell(f.output);
  assert.equal(Object.keys(manifest.nativeFiles!).length, 8);
  const assembly = join(f.work, 'assembly');
  await assemble(f.output, [], assembly);
  await verifyAssembly(assembly);
  const worker = join(assembly, 'native-record/capture/libexec/worker');
  await chmod(worker, 0o644);
  await assert.rejects(verifyAssembly(assembly), /mode mismatch/);
  await chmod(worker, 0o755);
  await writeFile(worker, 'changed');
  await assert.rejects(verifyAssembly(assembly), /checksum mismatch/);
});

test('mode changed after handle validation cannot repair an invalid source snapshot', async t => {
  const f = await fixture(); t.after(() => rm(f.work, { recursive: true, force: true }));
  const worker = join(f.input.capture.directory, 'libexec/worker');
  await chmod(worker, 0o644);
  const inode = (await stat(worker)).ino;
  const probe = await open(worker, 'r');
  const prototype = Object.getPrototypeOf(probe);
  await probe.close();
  const original = prototype.stat;
  let observations = 0;
  // Place the concurrent chmod exactly after the open handle's second stat,
  // before pathname validation. The old separate lstat accepted the new 0755
  // although the buffered bytes were observed from the invalid 0644 snapshot.
  t.mock.method(prototype, 'stat', async function (this: unknown, ...args: unknown[]) {
    const value = await original.apply(this, args);
    if (value.ino === inode && ++observations === 2) await chmod(worker, 0o755);
    return value;
  });
  await assert.rejects(stageRecordNative(f.output, f.input), /changed during read|mode mismatch/);
  assert.equal(observations, 2, 'the actual read/validation boundary was exercised');
  await assert.rejects(stat(join(f.output, 'native-record')), { code: 'ENOENT' });
});
