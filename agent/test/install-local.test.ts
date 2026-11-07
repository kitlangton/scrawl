import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { chmodSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { parseInstallArguments } from '../dist/install/arguments.js';
import { installLocal } from '../dist/install/core.js';
import { hashRegularFile, inventory, validateFreshPrefix } from '../dist/install/filesystem.js';
import { makeLauncher } from '../dist/install/launcher.js';
import { validateCaptureManifest, validateManifest } from '../dist/install/manifest.js';
import { DEFAULT_LIMITS, type InstallerDependencies, type InstallerInput, type NativeManifest, type RuntimeFile } from '../dist/install/types.js';
import { makeNpmInvocation } from '../dist/install/runtime.js';

const sha256 = (bytes: Buffer | string) => createHash('sha256').update(bytes).digest('hex');
const executable = '#!/bin/sh\nexit 0\n';
const agentTar = Buffer.from('synthetic Agent npm archive bytes');

async function fixtureRoot(parent: string) {
  const root = join(parent, 'native fixture');
  await mkdir(join(root, 'libexec'), { recursive: true, mode: 0o700 });
  const files: RuntimeFile[] = [];
  const add = async (path: string, bytes: string, mode: number) => {
    const full = join(root, path);
    await mkdir(dirname(full), { recursive: true, mode: 0o700 });
    await writeFile(full, bytes, { mode });
    await chmod(full, mode);
    files.push({ path, mode, sha256: sha256(bytes) });
  };
  await add('apparent-engine-frames', executable, 0o755);
  await add('libexec/apparent-engine-frames', executable, 0o755);
  const manifest: NativeManifest = {
    schema: 1,
    sourceSha: 'a'.repeat(40),
    target: 'linux-x64',
    rustTriple: 'x86_64-unknown-linux-gnu',
    capabilities: ['media/1'],
    launch: {
      entry: 'apparent-engine-frames',
      argv: ['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1'],
      worker: 'libexec/apparent-engine-frames',
      pathLookup: false,
      runtimeBuilds: false,
    },
    environment: { testFixture: true },
    runtimeFiles: files,
  };
  const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  await writeFile(join(root, 'manifest.json'), manifestBytes, { mode: 0o600 });
  return { root, manifest, manifestBytes, manifestSha256: sha256(manifestBytes), files };
}

async function fixtureInputs(parent: string, prefixName = 'private prefix') {
  const archive = join(parent, 'agent.tgz');
  await writeFile(archive, agentTar, { mode: 0o600 });
  const native = await fixtureRoot(parent);
  const prefix = join(parent, prefixName);
  const input: InstallerInput = {
    agentPath: archive,
    agentSha256: sha256(agentTar),
    nativeRoot: native.root,
    manifestSha256: native.manifestSha256,
    prefix,
  };
  return { input, archive, native, prefix };
}

function fixtureDependencies(overrides: Partial<InstallerDependencies> = {}): InstallerDependencies {
  const installNpm = overrides.installNpm ?? ((snapshotPath, installRoot, privateHome) => {
    const agentRoot = join(installRoot, 'node_modules/@apparent/agent');
    const sdkRoot = join(agentRoot, 'node_modules/@apparent/engine-sdk');
    mkdirSync(join(installRoot, 'node_modules/@apparent'), { recursive: true, mode: 0o700 });
    mkdirSync(join(agentRoot, 'dist'), { recursive: true, mode: 0o700 });
    mkdirSync(join(sdkRoot, 'dist/src'), { recursive: true, mode: 0o700 });
    writeFileSync(join(agentRoot, 'package.json'), JSON.stringify({ name: '@apparent/agent', version: '0.1.0', type: 'module', bin: { 'apparent-agent': './dist/cli.js' }, dependencies: { '@apparent/engine-sdk': 'file:vendor/apparent-engine-sdk-0.3.2.tgz' } }), { mode: 0o600 });
    writeFileSync(join(agentRoot, 'dist/cli.js'), 'process.stdout.write(JSON.stringify(process.argv.slice(2)) + "\\n");', { mode: 0o700 });
    writeFileSync(join(sdkRoot, 'package.json'), JSON.stringify({ name: '@apparent/engine-sdk', version: '0.3.2', exports: {
      './media': { import: './dist/src/media.js', types: './dist/src/media.d.ts' },
      './capture': { import: './dist/src/capture.js', types: './dist/src/capture.d.ts' },
    } }), { mode: 0o600 });
    writeFileSync(join(sdkRoot, 'dist/src/media.js'), '', { mode: 0o600 });
    writeFileSync(join(sdkRoot, 'dist/src/media.d.ts'), '', { mode: 0o600 });
    writeFileSync(join(sdkRoot, 'dist/src/capture.js'), '', { mode: 0o600 });
    writeFileSync(join(sdkRoot, 'dist/src/capture.d.ts'), '', { mode: 0o600 });
    assert.equal(sha256(readFileSync(snapshotPath)), sha256(agentTar));
    assert.ok(privateHome.startsWith(dirname(installRoot)));
  });
  return {
    installNpm,
    smokeCli: overrides.smokeCli ?? ((cli) => {
      const result = spawnSync(process.execPath, [cli, '--version'], { encoding: 'utf8' });
      assert.equal(result.status, 0, result.stderr);
      assert.deepEqual(JSON.parse(result.stdout), ['--version']);
    }),
    handshake: overrides.handshake ?? (async () => ({
      reply: { channel: 'media', protocolVersion: 1, implementation: 'native', capabilities: { decodeFrames: true } },
      closeOutcome: 'eof-exited',
    })),
    captureHandshake: overrides.captureHandshake ?? (async () => ({
      reply: { channel: 'capture', protocolVersion: 2, implementation: 'native', readiness: 'unavailable' },
      closeOutcome: 'eof-exited',
    })),
  };
}

async function fixtureCaptureRoot(parent: string) {
  const root = join(parent, 'capture fixture');
  const files: RuntimeFile[] = [];
  const add = async (path: string) => {
    const full = join(root, path);
    await mkdir(dirname(full), { recursive: true, mode: 0o700 });
    await writeFile(full, executable, { mode: 0o755 });
    await chmod(full, 0o755);
    files.push({ path, mode: 0o755, sha256: sha256(executable) });
  };
  await add('apparent-engine-capture-launch');
  await add('libexec/apparent-engine-capture');
  const manifest = {
    schema: 1,
    sourceSha: 'b'.repeat(40),
    recoveryLaunch: { schema: 'apparent.engine-recovery-peer/1', channel: 'capture/2', argv: [
      '--runtime-directory', '<private-runtime-dir>', '--capture-channel', 'capture', '--capture-protocol-version', '2',
      '--recovery-peer-v1', '<private-peer-nonce>',
    ], startsFreshCapture: false, onInputEof: 'retain-admitted-finite-work' },
    eligibleForReuse: true,
    target: 'linux-x64',
    rustTriple: 'x86_64-unknown-linux-gnu',
    capabilities: ['capture/2', 'x11-display', 'x11-area', 'retained-materials', 'authenticated-publication'],
    launch: { entry: 'apparent-engine-capture-launch', argv: ['--runtime-directory', '<private-runtime-dir>', '--capture-channel', 'capture', '--capture-protocol-version', '2'],
      worker: 'libexec/apparent-engine-capture', pathLookup: false, runtimeBuilds: false, shutdownTimeoutMs: 40000 },
    environment: { architecture: 'x86_64' },
    runtimeFiles: files,
  };
  const bytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  await writeFile(join(root, 'manifest.json'), bytes, { mode: 0o600 });
  return { root, manifest, bytes, sha256: sha256(bytes) };
}

async function withTemp<T>(run: (directory: string) => Promise<T>): Promise<T> {
  const directory = await mkdtemp(join(tmpdir(), 'apparent-agent-installer-test-'));
  try { return await run(directory); } finally { await rm(directory, { recursive: true, force: true }); }
}

async function expectInstallFailure(input: InstallerInput, dependencies: InstallerDependencies, pattern: RegExp, limits = DEFAULT_LIMITS) {
  await assert.rejects(installLocal(input, dependencies, limits), pattern);
  await assert.rejects(lstat(input.prefix), { code: 'ENOENT' });
}

test('argument parser rejects absent, malformed, unknown, duplicate and invalid digest options', () => {
  assert.throws(() => parseInstallArguments(['--agent', '/tmp/a']), /missing required option/);
  assert.throws(() => parseInstallArguments(['agent', '/tmp/a']), /unknown or malformed option/);
  assert.throws(() => parseInstallArguments(['--agent', '/tmp/a', '--agent', '/tmp/b']), /duplicate option/);
  assert.throws(() => parseInstallArguments(['--agent', '--agent-sha256']), /missing value/);
  assert.throws(() => parseInstallArguments(['--unknown', 'x']), /unknown or malformed option/);
  assert.throws(() => parseInstallArguments([
    '--agent', '/tmp/a', '--agent-sha256', 'x', '--native-root', '/tmp/n', '--manifest-sha256', '0'.repeat(64), '--prefix', '/tmp/p',
  ]), /invalid Agent SHA-256/);
  assert.throws(() => parseInstallArguments([
    '--agent', '/tmp/a', '--agent-sha256', 'a'.repeat(64), '--native-root', '/tmp/n', '--manifest-sha256', 'b'.repeat(64),
    '--prefix', '/tmp/p', '--capture-root', '/tmp/c',
  ]), /must be supplied together/);
});

test('Capture/2 manifest is exact, validated before prefix creation, and installed with the shared private runtime', async () => withTemp(async (directory) => {
  const { input: baseInput, prefix, native } = await fixtureInputs(directory, 'capture-prefix');
  const dualMedia = structuredClone(native.manifest) as any;
  dualMedia.capabilities = ['media/1', 'media/2'];
  dualMedia.media2Launch = { argv: ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2'], sharesCaptureRuntime: true };
  const dualMediaBytes = Buffer.from(`${JSON.stringify(dualMedia, null, 2)}\n`);
  await writeFile(join(native.root, 'manifest.json'), dualMediaBytes, { mode: 0o600 });
  const input = { ...baseInput, manifestSha256: sha256(dualMediaBytes) };
  const capture = await fixtureCaptureRoot(directory);
  assert.equal(validateCaptureManifest(capture.manifest).runtimeFiles.length, 2);
  const acceptedInput = { ...input, captureRoot: capture.root, captureManifestSha256: capture.sha256 };
  const observations: unknown[] = [];
  const deps = fixtureDependencies({
    async captureHandshake(entry, runtime, shutdownTimeoutMs) {
      observations.push({ entry, runtime, shutdownTimeoutMs });
      return { reply: { channel: 'capture', protocolVersion: 2, implementation: 'native', readiness: 'unavailable' }, closeOutcome: 'eof-exited' };
    },
  });
  await installLocal(acceptedInput, deps);
  assert.deepEqual(observations, [{ entry: join(prefix, 'native-capture/apparent-engine-capture-launch'), runtime: join(prefix, 'runtime'), shutdownTimeoutMs: 40000 }]);
  assert.equal((await lstat(join(prefix, 'runtime'))).mode & 0o777, 0o700);
  const launcher = await readFile(join(prefix, 'apparent-agent'), 'utf8');
  assert.match(launcher, /APPARENT_ENGINE_CAPTURE_EXECUTABLE=/);
  assert.match(launcher, new RegExp(`APPARENT_ENGINE_MEDIA2_BIN='${prefix}/native-media/apparent-engine-frames'`));
  assert.deepEqual(JSON.parse(await readFile(join(prefix, 'native-capture/manifest.json'), 'utf8')), capture.manifest);
  assert.equal(JSON.parse(await readFile(join(prefix, 'install-receipt.json'), 'utf8')).captureReadiness, 'unavailable');

  for (const mutate of [
    (value: any) => { value.extra = true; },
    (value: any) => { value.launch.argv[0] = '--wrong'; },
    (value: any) => { value.recoveryLaunch.startsFreshCapture = true; },
    (value: any) => { value.capabilities.push('unknown'); },
  ]) {
    const bad = structuredClone(capture.manifest);
    mutate(bad);
    assert.throws(() => validateCaptureManifest(bad));
    const bytes = Buffer.from(`${JSON.stringify(bad)}\n`);
    await writeFile(join(capture.root, 'manifest.json'), bytes, { mode: 0o600 });
    const rejected = { ...acceptedInput, prefix: join(directory, `rejected-${Math.random()}`), captureManifestSha256: sha256(bytes) };
    await assert.rejects(installLocal(rejected, fixtureDependencies()), /capture/);
    await assert.rejects(lstat(rejected.prefix), { code: 'ENOENT' });
  }
}));

test('Capture/2 symlink inventory is rejected before prefix creation and uncertain close never writes a receipt', async () => withTemp(async (directory) => {
  const { input, prefix } = await fixtureInputs(directory, 'capture-symlink-prefix');
  const capture = await fixtureCaptureRoot(directory);
  await rm(join(capture.root, 'libexec/apparent-engine-capture'));
  await symlink(join(capture.root, 'apparent-engine-capture-launch'), join(capture.root, 'libexec/apparent-engine-capture'));
  const badInput = { ...input, captureRoot: capture.root, captureManifestSha256: capture.sha256 };
  await assert.rejects(installLocal(badInput, fixtureDependencies()), /symlink/);
  await assert.rejects(lstat(prefix), { code: 'ENOENT' });

  const valid = await fixtureCaptureRoot(join(directory, 'valid'));
  const nextPrefix = join(directory, 'capture-close-prefix');
  const dependencies = fixtureDependencies({ async captureHandshake() {
    return { reply: { channel: 'capture', protocolVersion: 2, implementation: 'native', readiness: 'unavailable' }, closeOutcome: 'signal-requested-after-timeout' };
  } });
  await assert.rejects(installLocal({ ...input, prefix: nextPrefix, captureRoot: valid.root, captureManifestSha256: valid.sha256 }, dependencies), /handshake or confirmed close rejected/);
  assert.ok((await lstat(nextPrefix)).isDirectory());
  await assert.rejects(lstat(join(nextPrefix, 'install-receipt.json')), { code: 'ENOENT' });
}));

test('Capture/2 source mutation after authenticated inventory fails closed without a receipt', async () => withTemp(async (directory) => {
  const { input, prefix } = await fixtureInputs(directory, 'capture-mutation-prefix');
  const capture = await fixtureCaptureRoot(directory);
  const defaultInstall = fixtureDependencies().installNpm;
  const dependencies = fixtureDependencies({ installNpm(snapshot, root, home) {
    defaultInstall(snapshot, root, home);
    writeFileSync(join(capture.root, 'apparent-engine-capture-launch'), '#!/bin/sh\nexit 7\n');
  } });
  await assert.rejects(installLocal({ ...input, captureRoot: capture.root, captureManifestSha256: capture.sha256 }, dependencies), /changed since inventory/);
  assert.ok((await lstat(prefix)).isDirectory());
  await assert.rejects(lstat(join(prefix, 'install-receipt.json')), { code: 'ENOENT' });
}));

test('archive digest mismatch is checked against actual bytes before any prefix mutation', async () => withTemp(async (directory) => {
  const { input, prefix } = await fixtureInputs(directory);
  await expectInstallFailure({ ...input, agentSha256: '0'.repeat(64) }, fixtureDependencies(), /Agent tarball SHA-256 mismatch/);
  await assert.rejects(lstat(prefix), { code: 'ENOENT' });
  const manifestPrefix = join(directory, 'bad manifest digest');
  await expectInstallFailure({ ...input, prefix: manifestPrefix, manifestSha256: '0'.repeat(64) }, fixtureDependencies(), /native manifest SHA-256 mismatch/);
}));

test('small synthetic fixture installs into a private prefix and shell metacharacters remain arguments', async () => withTemp(async (directory) => {
  const marker = join(directory, 'shell-injected');
  const prefix = join(directory, "prefix with spaces ' ;$(false) `echo nope`");
  const { input, archive, native } = await fixtureInputs(directory, prefix.slice(directory.length + 1));
  const archiveBefore = await readFile(archive);
  const manifestBefore = await readFile(join(native.root, 'manifest.json'));
  const observations: string[] = [];
  const dependencies = fixtureDependencies({
    installNpm(snapshot, root, home) {
      observations.push(snapshot, root, home);
      fixtureDependencies().installNpm(snapshot, root, home);
    },
    smokeCli(cli) {
      const result = spawnSync(process.execPath, [cli, '--version'], { encoding: 'utf8' });
      assert.equal(result.status, 0);
      assert.deepEqual(JSON.parse(result.stdout), ['--version']);
    },
  });
  await installLocal(input, dependencies);
  assert.ok(observations[0].startsWith(join(prefix, 'inputs') + '/'));
  assert.notEqual(observations[0], archive);
  for (const path of [prefix, ...observations.slice(1), join(prefix, 'private-home'), join(prefix, 'inputs'), join(prefix, 'npm-cache'), join(prefix, 'runtime'), join(prefix, 'native-media')]) {
    const info = await lstat(path);
    assert.ok(info.isDirectory());
    assert.equal(info.mode & 0o077, 0);
  }

  const launcher = await readFile(join(prefix, 'apparent-agent'), 'utf8');
  assert.equal(launcher, makeLauncher(process.execPath, join(prefix, 'apparent-agent-private/node_modules/@apparent/agent/dist/cli.js'),
    join(prefix, 'native-media/apparent-engine-frames'), join(prefix, 'runtime')));
  const markerArg = `$(touch ${marker})`;
  const arg = "apostrophe ' and ; ` touch ${marker}";
  const run = spawnSync(join(prefix, 'apparent-agent'), ['--version', markerArg, arg], { encoding: 'utf8' });
  assert.equal(run.status, 0, run.stderr);
  assert.deepEqual(JSON.parse(run.stdout), ['--version', markerArg, arg]);
  await assert.rejects(lstat(marker), { code: 'ENOENT' });
  assert.deepEqual(await readFile(archive), archiveBefore);
  assert.deepEqual(await readFile(join(native.root, 'manifest.json')), manifestBefore);
  assert.equal(JSON.parse(await readFile(join(prefix, 'install-receipt.json'), 'utf8')).sdkVersion, '0.3.2');
  assert.equal(await readdir(join(prefix, 'native-media/libexec')).then((entries) => entries.includes('apparent-engine-frames')), true);
}));

test('prefix validation rejects shared-writable, symlinked, missing-parent and existing destinations before mutation', async () => withTemp(async (directory) => {
  const shared = join(directory, 'shared');
  await mkdir(shared, { mode: 0o700 });
  await chmod(shared, 0o777);
  await assert.rejects(validateFreshPrefix(join(shared, 'new-prefix')), /shared-writable/);
  const target = join(directory, 'target');
  await mkdir(target, { mode: 0o700 });
  await symlink(target, join(directory, 'linked-parent'));
  await assert.rejects(validateFreshPrefix(join(directory, 'linked-parent/prefix')), /canonical/);
  await assert.rejects(validateFreshPrefix(join(directory, 'absent-parent/prefix')), /ENOENT/);
  const occupied = join(directory, 'occupied');
  const { input: occupiedInput } = await fixtureInputs(directory, 'occupied');
  await mkdir(occupied, { mode: 0o700 });
  await writeFile(join(occupied, 'foreign'), 'keep');
  await assert.rejects(validateFreshPrefix(occupied), /already exists/);
  await assert.rejects(installLocal(occupiedInput, fixtureDependencies()), /already exists/);
  assert.equal(await readFile(join(occupied, 'foreign'), 'utf8'), 'keep');
}));

test('manifest and tree inventory reject malformed, unknown, duplicate, unsafe, tampered and undeclared entries', async () => withTemp(async (directory) => {
  const base = await fixtureRoot(directory);
  const good = JSON.parse(base.manifestBytes.toString());
  for (const change of [
    (value: any) => { value.extra = true; },
    (value: any) => { value.runtimeFiles = {}; },
    (value: any) => { value.runtimeFiles[1].path = '../escape'; },
    (value: any) => { value.runtimeFiles[1].path = value.runtimeFiles[0].path; },
    (value: any) => { value.runtimeFiles[0].mode = 0o777; },
    (value: any) => { value.runtimeFiles[0].sha256 = 'bad'; },
    (value: any) => { value.launch.argv = ['--anything']; },
    (value: any) => { value.launch.pathLookup = true; },
    (value: any) => { value.target = 'linux-arm64'; },
  ]) {
    const mutation = structuredClone(good);
    change(mutation);
    assert.throws(() => validateManifest(mutation));
  }

  const { input, native } = await fixtureInputs(directory);
  const rejectRoot = async (pattern: RegExp) => {
    const manifestBytes = await readFile(join(native.root, 'manifest.json'));
    const nextInput = { ...input, manifestSha256: sha256(manifestBytes), prefix: join(directory, `reject-${Math.random()}`) };
    await assert.rejects(installLocal(nextInput, fixtureDependencies(), pattern));
    await assert.rejects(lstat(nextInput.prefix), { code: 'ENOENT' });
  };
  await writeFile(join(native.root, 'apparent-engine-frames'), 'tampered');
  await rejectRoot(/exactly match manifest/);
  await writeFile(join(native.root, 'apparent-engine-frames'), executable, { mode: 0o755 });
  await chmod(join(native.root, 'apparent-engine-frames'), 0o755);
  await rm(join(native.root, 'libexec/apparent-engine-frames'));
  await rejectRoot(/exactly match manifest/);
  await writeFile(join(native.root, 'libexec/apparent-engine-frames'), executable, { mode: 0o755 });
  await chmod(join(native.root, 'libexec/apparent-engine-frames'), 0o755);
  await writeFile(join(native.root, 'extra.so'), 'not declared');
  await rejectRoot(/exactly match manifest/);
  await rm(join(native.root, 'extra.so'));
  await chmod(join(native.root, 'apparent-engine-frames'), 0o600);
  await rejectRoot(/invalid mode/);
  await chmod(join(native.root, 'apparent-engine-frames'), 0o755);
  await rm(join(native.root, 'libexec/apparent-engine-frames'));
  await symlink(join(native.root, 'apparent-engine-frames'), join(native.root, 'libexec/apparent-engine-frames'));
  await rejectRoot(/symlink/);
  await rm(join(native.root, 'libexec/apparent-engine-frames'));
  execFileSync('mkfifo', [join(native.root, 'fifo')]);
  await rejectRoot(/special file/);
}));

test('dual media manifest preserves the exact media/1 install path and rejects partial or altered media/2 declarations before writes', async () => withTemp(async (directory) => {
  const base = await fixtureRoot(directory);
  const dual = structuredClone(base.manifest) as any;
  dual.capabilities = ['media/1', 'media/2'];
  dual.media2Launch = {
    argv: ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2'],
    sharesCaptureRuntime: true,
  };
  assert.deepEqual(validateManifest(dual).capabilities, ['media/1', 'media/2']);

  const dualBytes = Buffer.from(`${JSON.stringify(dual, null, 2)}\n`);
  await writeFile(join(base.root, 'manifest.json'), dualBytes, { mode: 0o600 });
  const archive = join(directory, 'agent.tgz');
  await writeFile(archive, agentTar, { mode: 0o600 });
  const prefix = join(directory, 'dual-prefix');
  await installLocal({ agentPath: archive, agentSha256: sha256(agentTar), nativeRoot: base.root,
    manifestSha256: sha256(dualBytes), prefix }, fixtureDependencies());
  const launcher = await readFile(join(prefix, 'apparent-agent'), 'utf8');
  assert.match(launcher, /apparent-engine-frames/);
  assert.match(launcher, /runtime/);
  assert.doesNotMatch(launcher, /media-protocol-version.*2|capture-runtime-dir/);
  assert.deepEqual(JSON.parse(await readFile(join(prefix, 'native-media/manifest.json'), 'utf8')), dual);
  const receipt = JSON.parse(await readFile(join(prefix, 'install-receipt.json'), 'utf8'));
  assert.equal(receipt.nativeEntry, join(prefix, 'native-media/apparent-engine-frames'));
  assert.deepEqual(receipt.claims, [
    'local Agent consumer installed',
    'native media/1 handshake and confirmed close passed; no codec claim',
    'does not imply current-main artifact admission or global routing',
  ]);

  const malformed = [
    (value: any) => { value.capabilities = ['media/2', 'media/1']; },
    (value: any) => { delete value.media2Launch; },
    (value: any) => { value.media2Launch.argv = ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1']; },
    (value: any) => { value.media2Launch.argv.push('--extra'); },
    (value: any) => { value.media2Launch.sharesCaptureRuntime = false; },
    (value: any) => { value.media2Launch.extra = true; },
    (value: any) => { value.extra = true; },
    (value: any) => { value.capabilities = ['media/1', 'media/2', 'media/3']; },
    (value: any) => { value.capabilities = ['media/1']; value.media2Launch = dual.media2Launch; },
  ];
  for (const [index, change] of malformed.entries()) {
    const value = structuredClone(dual);
    change(value);
    const badPrefix = join(directory, `malformed-${index}`);
    const bytes = Buffer.from(`${JSON.stringify(value, null, 2)}\n`);
    await writeFile(join(base.root, 'manifest.json'), bytes, { mode: 0o600 });
    await assert.rejects(installLocal({ agentPath: archive, agentSha256: sha256(agentTar), nativeRoot: base.root,
      manifestSha256: sha256(bytes), prefix: badPrefix }, fixtureDependencies()));
    await assert.rejects(lstat(badPrefix), { code: 'ENOENT' });
  }
}));

test('dual media manifest accepts only the exact additive visual observation profile before installation writes', async () => withTemp(async (directory) => {
  const base = await fixtureRoot(directory);
  const dual = structuredClone(base.manifest) as any;
  dual.capabilities = ['media/1', 'media/2'];
  dual.media2Launch = {
    argv: ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2'],
    sharesCaptureRuntime: true,
  };
  dual.visualObservations = {
    schema: 'apparent.frame-observation-profile/1',
    appliesTo: 'media/2-frame-png',
    chunkType: 'apOB',
    algorithm: 'apparent.visual-luma/1',
  };
  assert.deepEqual(validateManifest(dual).visualObservations, dual.visualObservations);

  const archive = join(directory, 'agent.tgz');
  await writeFile(archive, agentTar, { mode: 0o600 });
  const reject = async (value: any, index: number) => {
    const bytes = Buffer.from(`${JSON.stringify(value, null, 2)}\n`);
    await writeFile(join(base.root, 'manifest.json'), bytes, { mode: 0o600 });
    const prefix = join(directory, `invalid-profile-${index}`);
    await assert.rejects(installLocal({ agentPath: archive, agentSha256: sha256(agentTar), nativeRoot: base.root,
      manifestSha256: sha256(bytes), prefix }, fixtureDependencies()));
    await assert.rejects(lstat(prefix), { code: 'ENOENT' });
  };
  const malformed = [
    (value: any) => { value.visualObservations = { ...value.visualObservations, schema: 'apparent.frame-observation-profile/2' }; },
    (value: any) => { value.visualObservations = { ...value.visualObservations, appliesTo: 'media/1-frame-png' }; },
    (value: any) => { value.visualObservations = { ...value.visualObservations, chunkType: 'apXX' }; },
    (value: any) => { value.visualObservations = { ...value.visualObservations, algorithm: 'apparent.visual-luma/2' }; },
    (value: any) => { delete value.visualObservations.algorithm; },
    (value: any) => { value.visualObservations.extra = true; },
    (value: any) => { value.visualObservations.schema = 1; },
    (value: any) => { value.visualObservations = null; },
    (value: any) => { value.visualObservations = 'apparent.frame-observation-profile/1'; },
    (value: any) => { value.capabilities = ['media/1']; delete value.media2Launch; },
  ];
  for (const [index, change] of malformed.entries()) {
    const value = structuredClone(dual);
    change(value);
    assert.throws(() => validateManifest(value));
    await reject(value, index);
  }
}));

test('dual media manifest accepts only exact additive recovery launch metadata without enabling recovery', async () => withTemp(async (directory) => {
  const base = await fixtureRoot(directory);
  const dual = structuredClone(base.manifest) as any;
  dual.capabilities = ['media/1', 'media/2'];
  dual.media2Launch = {
    argv: ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2'],
    sharesCaptureRuntime: true,
  };
  dual.visualObservations = {
    schema: 'apparent.frame-observation-profile/1',
    appliesTo: 'media/2-frame-png',
    chunkType: 'apOB',
    algorithm: 'apparent.visual-luma/1',
  };
  dual.recoveryLaunch = {
    schema: 'apparent.engine-recovery-peer/1',
    channel: 'media/2',
    argv: ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2', '--recovery-peer-v1', '<private-peer-nonce>'],
    startsFreshCapture: false,
    onInputEof: 'retain-admitted-finite-work',
  };
  assert.deepEqual(validateManifest(dual).recoveryLaunch, dual.recoveryLaunch);

  const archive = join(directory, 'agent.tgz');
  await writeFile(archive, agentTar, { mode: 0o600 });
  const prefix = join(directory, 'recovery-prefix');
  const bytes = Buffer.from(`${JSON.stringify(dual, null, 2)}\n`);
  await writeFile(join(base.root, 'manifest.json'), bytes, { mode: 0o600 });
  await installLocal({ agentPath: archive, agentSha256: sha256(agentTar), nativeRoot: base.root,
    manifestSha256: sha256(bytes), prefix }, fixtureDependencies());
  const launcher = await readFile(join(prefix, 'apparent-agent'), 'utf8');
  assert.match(launcher, /apparent-engine-frames/);
  assert.doesNotMatch(launcher, /recovery-peer-v1|private-peer-nonce|media-protocol-version.*2|capture-runtime-dir/);
  assert.deepEqual(JSON.parse(await readFile(join(prefix, 'native-media/manifest.json'), 'utf8')), dual);
  const receipt = JSON.parse(await readFile(join(prefix, 'install-receipt.json'), 'utf8'));
  assert.equal(receipt.nativeEntry, join(prefix, 'native-media/apparent-engine-frames'));
  assert.deepEqual(receipt.claims, [
    'local Agent consumer installed',
    'native media/1 handshake and confirmed close passed; no codec claim',
    'does not imply current-main artifact admission or global routing',
  ]);

  const malformed = [
    (value: any) => { value.recoveryLaunch.schema = 'apparent.engine-recovery-peer/2'; },
    (value: any) => { value.recoveryLaunch.channel = 'capture/2'; },
    (value: any) => { value.recoveryLaunch.argv = ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1']; },
    (value: any) => { value.recoveryLaunch.argv.push('--extra'); },
    (value: any) => { value.recoveryLaunch.startsFreshCapture = true; },
    (value: any) => { value.recoveryLaunch.onInputEof = 'cancel-admitted-work'; },
    (value: any) => { delete value.recoveryLaunch.onInputEof; },
    (value: any) => { value.recoveryLaunch.extra = true; },
    (value: any) => { value.recoveryLaunch.schema = 1; },
    (value: any) => { value.recoveryLaunch.argv = null; },
    (value: any) => { value.recoveryLaunch = null; },
    (value: any) => { value.capabilities = ['media/1']; delete value.media2Launch; },
  ];
  for (const [index, change] of malformed.entries()) {
    const value = structuredClone(dual);
    change(value);
    const badPrefix = join(directory, `invalid-recovery-${index}`);
    const invalidBytes = Buffer.from(`${JSON.stringify(value, null, 2)}\n`);
    await writeFile(join(base.root, 'manifest.json'), invalidBytes, { mode: 0o600 });
    assert.throws(() => validateManifest(value));
    await assert.rejects(installLocal({ agentPath: archive, agentSha256: sha256(agentTar), nativeRoot: base.root,
      manifestSha256: sha256(invalidBytes), prefix: badPrefix }, fixtureDependencies()));
    await assert.rejects(lstat(badPrefix), { code: 'ENOENT' });
  }
}));

test('native inventory aggregate, file-count, entry-count and per-file bounds fail before prefix creation', async () => withTemp(async (directory) => {
  const { input, native } = await fixtureInputs(directory);
  const noPrefix = async (limits: typeof DEFAULT_LIMITS, pattern: RegExp, suffix: string) => {
    const next = { ...input, prefix: join(directory, suffix) };
    await assert.rejects(installLocal(next, fixtureDependencies(), limits), pattern);
    await assert.rejects(lstat(next.prefix), { code: 'ENOENT' });
  };
  await noPrefix({ ...DEFAULT_LIMITS, maxFiles: 1 }, /file-count limit/, 'limit-files');
  await noPrefix({ ...DEFAULT_LIMITS, maxEntries: 1 }, /entry-count limit/, 'limit-entries');
  await noPrefix({ ...DEFAULT_LIMITS, maxFileBytes: 4 }, /file exceeds size limit/, 'limit-file-bytes');
  await noPrefix({ ...DEFAULT_LIMITS, maxTotalBytes: 12 }, /aggregate size limit/, 'limit-total-bytes');
  const sparse = join(native.root, 'sparse.dat');
  await writeFile(sparse, '');
  await import('node:fs/promises').then(({ truncate }) => truncate(sparse, DEFAULT_LIMITS.maxFileBytes + 1));
  await assert.rejects(inventory(native.root, DEFAULT_LIMITS), /file exceeds size limit/);
}));

test('test fixture npm call pins offline/no-script controls and uses isolated HOME, config and cwd', () => {
  const invocation = makeNpmInvocation('/private/prefix/agent.tgz', '/private/prefix/install', '/private/prefix/home', '/private/prefix/cache', '/usr/bin');
  assert.ok(invocation.args.includes('--offline'));
  assert.ok(invocation.args.includes('--ignore-scripts'));
  assert.equal(invocation.cwd, '/private/prefix/install');
  assert.equal(invocation.env.HOME, '/private/prefix/home');
  assert.equal(invocation.env.npm_config_userconfig, '/private/prefix/home/user.npmrc');
  assert.equal(invocation.env.npm_config_globalconfig, '/private/prefix/home/global.npmrc');
  assert.notEqual(invocation.env.npm_config_userconfig, invocation.env.npm_config_globalconfig);
  assert.equal(invocation.env.npm_config_offline, 'true');
  assert.equal(invocation.env.npm_config_ignore_scripts, 'true');
  assert.equal(invocation.env.TMPDIR, '/private/prefix/npm-tmp');
  assert.equal(invocation.umask, 0o077);
  assert.ok(invocation.args.includes('--umask=077'));
  assert.equal('NPM_TOKEN' in invocation.env, false);
});

test('native source path replacement after inventory fails closed without a success receipt', async () => withTemp(async (directory) => {
  const { input, native, prefix } = await fixtureInputs(directory);
  const entry = join(native.root, 'apparent-engine-frames');
  const replacement = join(native.root, 'entry-backup');
  const dependencies = fixtureDependencies({
    installNpm(snapshot, installRoot, home) {
      fixtureDependencies().installNpm(snapshot, installRoot, home);
      renameSync(entry, replacement);
      writeFileSync(entry, executable, { mode: 0o755 });
      chmodSync(entry, 0o755);
    },
  });
  await assert.rejects(installLocal(input, dependencies), /changed since inventory/);
  assert.ok((await lstat(prefix)).isDirectory());
  await assert.rejects(lstat(join(prefix, 'install-receipt.json')), { code: 'ENOENT' });
}));

test('Agent archive is snapshotted under the private prefix before npm and source remains unchanged', async () => withTemp(async (directory) => {
  const { input, archive, prefix } = await fixtureInputs(directory);
  let snapshot = '';
  const dependencies = fixtureDependencies({
    installNpm(snapshotPath, installRoot, home) {
      snapshot = snapshotPath;
      assert.deepEqual(readFileSync(snapshotPath), agentTar);
      fixtureDependencies().installNpm(snapshotPath, installRoot, home);
    },
  });
  await installLocal(input, dependencies);
  assert.ok(snapshot.startsWith(join(prefix, 'inputs')));
  assert.deepEqual(await readFile(archive), agentTar);
}));

test('Agent or SDK metadata rejection leaves partial private prefix without receipt', async () => withTemp(async (directory) => {
  for (const kind of ['agent', 'sdk'] as const) {
    const { input, prefix } = await fixtureInputs(directory, `bad-${kind}`);
    const dependencies = fixtureDependencies({
      installNpm(snapshotPath, installRoot, home) {
        fixtureDependencies().installNpm(snapshotPath, installRoot, home);
        const path = kind === 'agent'
          ? join(installRoot, 'node_modules/@apparent/agent/package.json')
          : join(installRoot, 'node_modules/@apparent/agent/node_modules/@apparent/engine-sdk/package.json');
        const pkg = JSON.parse(readFileSync(path, 'utf8'));
        if (kind === 'agent') pkg.version = '9.9.9';
        else pkg.exports = {};
        writeFileSync(path, JSON.stringify(pkg));
      },
    });
    await assert.rejects(installLocal(input, dependencies), kind === 'agent' ? /Agent package metadata mismatch/ : /SDK media export\/version mismatch/);
    assert.ok((await lstat(prefix)).isDirectory());
    await assert.rejects(lstat(join(prefix, 'install-receipt.json')), { code: 'ENOENT' });
  }
}));

test('test-double handshake and uncertain worker shutdown never publish success receipt', async () => withTemp(async (directory) => {
  const cases = [
    { name: 'double', result: { reply: { channel: 'media', protocolVersion: 1, implementation: 'test-double', capabilities: { decodeFrames: true } }, closeOutcome: 'eof-exited' }, error: /handshake rejected/ },
    { name: 'close', result: { reply: { channel: 'media', protocolVersion: 1, implementation: 'native', capabilities: { decodeFrames: true } }, closeOutcome: 'term-requested-after-timeout' }, error: /close was uncertain/ },
  ];
  for (const item of cases) {
    const { input, prefix } = await fixtureInputs(directory, `failed-${item.name}`);
    const dependencies = fixtureDependencies({ async handshake() { return item.result; } });
    await assert.rejects(installLocal(input, dependencies), item.error);
    assert.ok((await lstat(prefix)).isDirectory());
    await assert.rejects(lstat(join(prefix, 'install-receipt.json')), { code: 'ENOENT' });
  }
}));

test('stable patch releases in reviewed SDK lines install and record the actual SDK version', async () => withTemp(async (directory) => {
  for (const legacy of [false, true]) for (const version of ['0.2.1', '0.3.0', '0.3.1', '0.3.2', '0.3.17', '0.4.0', '0.4.1', '0.4.27']) {
    const { input, prefix } = await fixtureInputs(directory, `sdk-${version}-${legacy}`);
    const dependencies = fixtureDependencies({
      installNpm(snapshot, root, home) {
        fixtureDependencies().installNpm(snapshot, root, home);
        const path = join(root, 'node_modules/@apparent/agent/node_modules/@apparent/engine-sdk/package.json');
        const pkg = JSON.parse(readFileSync(path, 'utf8'));
        pkg.version = version;
        writeFileSync(path, JSON.stringify(pkg));
        const agentPath = join(root, 'node_modules/@apparent/agent/package.json');
        const agent = JSON.parse(readFileSync(agentPath, 'utf8'));
        agent.dependencies['@apparent/engine-sdk'] = legacy ? 'file:../engine/sdk' : `file:vendor/apparent-engine-sdk-${version}.tgz`;
        writeFileSync(agentPath, JSON.stringify(agent));
      },
    });
    await installLocal(input, dependencies);
    const receipt = JSON.parse(await readFile(join(prefix, 'install-receipt.json'), 'utf8'));
    assert.equal(receipt.sdkVersion, version);
  }
}));

test('unrecognized SDK declarations and mismatched published versions fail without a receipt', async () => withTemp(async (directory) => {
  for (const [index, source] of ['file:vendor/apparent-engine-sdk-0.3.1.tgz', 'file:../../engine/sdk', 'https://example.invalid/sdk.tgz'].entries()) {
    const { input, prefix } = await fixtureInputs(directory, `sdk-source-${index}`);
    const dependencies = fixtureDependencies({ installNpm(snapshot, root, home) {
      fixtureDependencies().installNpm(snapshot, root, home);
      const path = join(root, 'node_modules/@apparent/agent/package.json');
      const agent = JSON.parse(readFileSync(path, 'utf8'));
      agent.dependencies['@apparent/engine-sdk'] = source;
      writeFileSync(path, JSON.stringify(agent));
    } });
    await assert.rejects(installLocal(input, dependencies), /Agent package metadata mismatch/);
    await assert.rejects(lstat(join(prefix, 'install-receipt.json')), { code: 'ENOENT' });
  }
}));

test('unreviewed SDK versions still fail without a success receipt', async () => withTemp(async (directory) => {
  for (const version of ['0.5.0', '0.1.9', '1.0.0', '0.4.0-next', '0.4.1+unreviewed', '0.3.01', '0.3.9007199254740992']) {
    const { input, prefix } = await fixtureInputs(directory, `sdk-${version}`);
    const dependencies = fixtureDependencies({
      installNpm(snapshot, root, home) {
        fixtureDependencies().installNpm(snapshot, root, home);
        const path = join(root, 'node_modules/@apparent/agent/node_modules/@apparent/engine-sdk/package.json');
        const pkg = JSON.parse(readFileSync(path, 'utf8'));
        pkg.version = version;
        writeFileSync(path, JSON.stringify(pkg));
      },
    });
    await assert.rejects(installLocal(input, dependencies), /SDK media export\/version mismatch/);
    await assert.rejects(lstat(join(prefix, 'install-receipt.json')), { code: 'ENOENT' });
  }
}));

test('hashing refuses symlinks and non-regular or changing files using bounded no-follow reads', async () => withTemp(async (directory) => {
  const file = join(directory, 'regular');
  await writeFile(file, 'bounded bytes');
  assert.equal(await hashRegularFile(file, 100), sha256('bounded bytes'));
  await assert.rejects(hashRegularFile(file, 3), /size limit/);
  const link = join(directory, 'link');
  await symlink(file, link);
  await assert.rejects(hashRegularFile(link, 100), /regular no-follow/);
  const fifo = join(directory, 'fifo');
  execFileSync('mkfifo', [fifo]);
  await assert.rejects(hashRegularFile(fifo, 100), /regular no-follow/);
}));

test('normal root import is side-effect-free', async () => {
  const result = await import('../dist/install-local.js');
  assert.equal(typeof result.installLocal, 'function');
  assert.equal(typeof result.parseInstallArguments, 'function');
});

test('npm-style symlinked installer entry executes argument validation', async () => withTemp(async (directory) => {
  const entry = join(directory, 'apparent-agent-install-local');
  await symlink(new URL('../dist/install-local.js', import.meta.url), entry);
  const result = spawnSync(process.execPath, [entry], { encoding: 'utf8', timeout: 10_000 });
  assert.equal(result.status, 1, result.stderr);
  assert.equal(result.stdout, '');
  assert.match(result.stderr, /missing required option/);
}));
