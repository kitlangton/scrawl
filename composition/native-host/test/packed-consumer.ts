import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { access, lstat, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

process.umask(0o077);
const nativeRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const contractsRoot = resolve(nativeRoot, '../../shared/contracts');
const retainedRoot = process.env.NATIVE_HOST_PACKED_PROOF_DIR
  ? resolve(process.env.NATIVE_HOST_PACKED_PROOF_DIR)
  : undefined;
const root = retainedRoot ?? await mkdtemp(join(tmpdir(), 'apparent-native-host-packed-'));
if (retainedRoot) await mkdir(root, { recursive: false, mode: 0o700 });
const packages = join(root, 'packages');
const consumer = join(root, 'consumer');
const deny = join(root, 'deny-tools');
const logs = join(root, 'logs');
await Promise.all([mkdir(packages, { mode: 0o700 }), mkdir(consumer, { mode: 0o700 }), mkdir(deny, { mode: 0o700 }), mkdir(logs, { mode: 0o700 })]);
const log = async (name: string, value: string) => writeFile(join(logs, name), value, { mode: 0o600 });
const sha256 = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');
const digestFile = async (path: string) => sha256(await readFile(path));
const run = async (name: string, command: string, args: string[], cwd: string, env = process.env, timeout = 180_000) => {
  const result = spawnSync(command, args, { cwd, env, encoding: 'utf8', timeout, maxBuffer: 8 * 1024 * 1024 });
  const transcript = [`$ ${command} ${args.join(' ')}`, `exit=${result.status}`, result.stdout ?? '', result.stderr ?? ''].join('\n');
  await log(name, transcript);
  if (result.error || result.status !== 0) throw new Error(`${name} failed; see ${join(logs, name)}\n${transcript.slice(-6000)}`);
  return result.stdout ?? '';
};
const npm = process.env.npm_execpath
  ? [process.execPath, process.env.npm_execpath]
  : ['npm'];
const npmCommand = npm[0]!;
const npmPrefix = npm.slice(1);
let passed = false;

try {
  await symlink('/bin/false', join(deny, 'cargo'));
  await symlink('/bin/false', join(deny, 'rustc'));
  await symlink('/bin/false', join(deny, 'tsc'));
  await symlink('/bin/false', join(deny, 'esbuild'));
  const blockedPath = `${deny}${process.platform === 'win32' ? ';' : ':'}${process.env.PATH ?? ''}`;
  const installEnv = { ...process.env, PATH: blockedPath, npm_config_cache: join(root, 'npm-cache') };

  for (const [workdir, artifact] of [
    [contractsRoot, 'apparent-app-contracts-0.1.0.tgz'],
    [nativeRoot, 'apparent-native-host-0.1.0.tgz'],
  ] as const) {
    await run(`pack-${artifact}.log`, npmCommand, [...npmPrefix, 'pack', '--pack-destination', packages], workdir);
    await access(join(packages, artifact));
  }

  const contractsTar = join(packages, 'apparent-app-contracts-0.1.0.tgz');
  const nativeTar = join(packages, 'apparent-native-host-0.1.0.tgz');
  const nativeTarEntries = (await run('native-tar-list.log', 'tar', ['-tzf', nativeTar], root)).split(/\r?\n/).filter(Boolean);
  assert.ok(nativeTarEntries.some(name => name === 'package/dist/client.js'));
  assert.ok(nativeTarEntries.some(name => name === 'package/dist/client.d.ts'));
  assert.ok(nativeTarEntries.every(name => !name.startsWith('package/src/')), 'native-host tarball must not require TypeScript source');

  await writeFile(join(consumer, 'package.json'), JSON.stringify({ name: 'native-host-cold-consumer', private: true, type: 'module' }, null, 2));
  await run('consumer-install.log', npmCommand, [...npmPrefix, 'install', '--no-audit', '--no-fund', contractsTar, nativeTar], consumer, installEnv);

  const installedHost = join(consumer, 'node_modules/@apparent/native-host');
  const installedContracts = join(consumer, 'node_modules/@apparent/app-contracts');
  assert.equal((await lstat(installedHost)).isSymbolicLink(), false, 'host package must be installed from its tarball');
  assert.equal((await lstat(installedContracts)).isSymbolicLink(), false, 'contracts package must be installed from its tarball');
  const hostEntries = await readdir(installedHost);
  assert.deepEqual(hostEntries.sort(), ['README.md', 'dist', 'package.json']);
  const hostPackage = JSON.parse(await readFile(join(installedHost, 'package.json'), 'utf8')) as Record<string, unknown>;
  assert.equal(hostPackage.dependencies, undefined, 'runtime package must not have a source-tree dependency');
  assert.equal((hostPackage.peerDependencies as Record<string, string>)['@apparent/app-contracts'], '0.1.0');
  for (const forbidden of ['react', 'react-dom', 'electron', '@types/react', '@apparent/ui']) {
    await assert.rejects(access(join(consumer, 'node_modules', forbidden)), { code: 'ENOENT' }, `${forbidden} must not be installed`);
  }
  await log('installed-tree.log', (await run('installed-tree-list.log', 'npm', ['ls', '--all', '--parseable'], consumer, installEnv)).trim());

  const runtimeSmoke = `
    import assert from 'node:assert/strict';
    import { createNativeHostPort } from '@apparent/native-host';
    import { createNativePenPort } from '@apparent/native-host/pen';
    import { createNativeClipboardPort, MAX_UTF8_TEXT_BYTES, MAX_SERIALIZED_FRAME_BYTES } from '@apparent/native-host/clipboard';
    assert.equal(MAX_UTF8_TEXT_BYTES, 21800);
    assert.equal(MAX_SERIALIZED_FRAME_BYTES, 131072);
    assert.equal(typeof createNativeHostPort().readSnapshot, 'function');
    assert.equal(typeof createNativePenPort().readStrokeColor, 'function');
    const clipboard = createNativeClipboardPort();
    assert.deepEqual(await clipboard.writeText('bad\\0text', { processEpoch: 'epoch', revision: 1 }), { status: 'refused', reason: 'invalid-text' });
    assert.deepEqual(await clipboard.writeText('x'.repeat(21801), { processEpoch: 'epoch', revision: 1 }), { status: 'refused', reason: 'too-large' });
    console.log('plain-node-imports=pass; limits=21800,131072; local-refusals=pass');
  `;
  await run('plain-node-runtime.log', process.execPath, ['--input-type=module', '-e', runtimeSmoke], consumer, installEnv);

  const consumerTypeRoot = join(consumer, 'empty-types');
  await mkdir(consumerTypeRoot, { mode: 0o700 });
  await writeFile(join(consumer, 'consumer.ts'), await readFile(join(nativeRoot, 'test/packed-consumer-types.ts')));
  await writeFile(join(consumer, 'tsconfig.json'), JSON.stringify({
    compilerOptions: {
      strict: true, noEmit: true, target: 'ES2022', module: 'NodeNext', moduleResolution: 'NodeNext',
      skipLibCheck: false, types: [], typeRoots: ['./empty-types'],
    },
    files: ['consumer.ts'],
  }, null, 2));
  const tsc = resolve(nativeRoot, 'node_modules/typescript/bin/tsc');
  await run('strict-consumer-typecheck.log', process.execPath, [tsc, '--project', join(consumer, 'tsconfig.json')], consumer, installEnv);

  const consumerTests = join(consumer, 'test');
  await mkdir(consumerTests, { mode: 0o700 });
  const sourceTests = await readFile(join(nativeRoot, 'test/client.test.ts'), 'utf8');
  const packedTests = sourceTests
    .replace(
      "import { createNativeHostPort, createNativeClipboardPort, createNativePenPort } from '../src/client.ts';",
      "import { createNativeHostPort } from '@apparent/native-host';\nimport { createNativeClipboardPort } from '@apparent/native-host/clipboard';\nimport { createNativePenPort } from '@apparent/native-host/pen';",
    );
  assert.notEqual(packedTests, sourceTests, 'packed test copy must switch to the installed public exports');
  assert.ok(!packedTests.includes("'../src/"), 'packed test copy must not import TypeScript source');
  await writeFile(join(consumerTests, 'client.test.ts'), packedTests);
  await run('compiled-client-socket-tests.log', process.execPath, ['--experimental-strip-types', '--test', join(consumerTests, 'client.test.ts')], consumer, installEnv);

  const sourceFiles = [
    'package.json', 'package-lock.json', 'README.md', 'tsconfig.build.json',
    'src/client.ts', 'src/pen.ts', 'src/clipboard.ts',
    'test/client.test.ts', 'test/packed-consumer-types.ts', 'test/packed-consumer.ts',
  ];
  const sourceSha = Object.fromEntries(await Promise.all(sourceFiles.map(async file => [file, await digestFile(join(nativeRoot, file))])));
  const distFiles = ['client.js', 'client.d.ts', 'pen.js', 'pen.d.ts', 'clipboard.js', 'clipboard.d.ts'];
  const distSha = Object.fromEntries(await Promise.all(distFiles.map(async file => [file, await digestFile(join(nativeRoot, 'dist', file))])));
  const proof = {
    node: process.version,
    sourceFiles: sourceSha,
    buildFiles: distSha,
    packageTars: {
      appContracts: { path: 'packages/apparent-app-contracts-0.1.0.tgz', sha256: await digestFile(contractsTar) },
      nativeHost: { path: 'packages/apparent-native-host-0.1.0.tgz', sha256: await digestFile(nativeTar) },
    },
    publicExports: ['@apparent/native-host', '@apparent/native-host/pen', '@apparent/native-host/clipboard'],
    engineFloor: '>=22.18',
    install: 'normal npm install from paired tarballs; scripts enabled; cargo/rustc/tsc/esbuild denied on PATH',
    runtime: 'plain Node ESM import of root, pen, and clipboard; invalid and oversize refusal; no TS flags',
    types: 'strict fresh consumer compile; no React ambient types',
    socketTests: 'existing 18 tests against the installed compiled package',
  };
  await writeFile(join(root, 'proof.json'), JSON.stringify(proof, null, 2), { mode: 0o600 });
  passed = true;
  console.log(`PACKED_CONSUMER=PASS`);
  console.log(JSON.stringify(proof, null, 2));
  console.log(`PACKED_PROOF_ROOT=${root}`);
} finally {
  if (!retainedRoot && passed) await rm(root, { recursive: true, force: true });
  else console.log(`PACKED_PROOF_RETAINED=${root}`);
}
