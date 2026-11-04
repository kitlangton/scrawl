import { STORAGE_BRIDGE_TEST, verifyBridgeListing, verifyBridgeResult } from './settings/bridge-result.ts';
import { createHash } from 'node:crypto';
import { chmod, mkdir, readFile, readdir, rename, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const frontend = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = path.resolve(frontend, '../../../..');
const receiptDefault = path.join(process.env.XDG_CACHE_HOME || path.join(process.env.HOME || '/tmp', '.cache'),
  'apparent', 'desktop-settings-native.json');


function run(command: string, args: string[], options: { cwd?: string; env?: NodeJS.ProcessEnv; capture?: boolean } = {}) {
  return new Promise<{ code: number; stdout: string; stderr: string }>((resolve, reject) => {
    const child = spawn(command, args, { cwd: options.cwd || root, env: options.env || process.env,
      stdio: options.capture ? ['ignore', 'pipe', 'pipe'] : 'inherit' });
    let stdout = '';
    let stderr = '';
    child.stdout?.on('data', chunk => { stdout += chunk; });
    child.stderr?.on('data', chunk => { stderr += chunk; });
    child.once('error', reject);
    child.once('close', code => resolve({ code: code ?? 1, stdout, stderr }));
  });
}

function sha256(bytes: Buffer | string) {
  return createHash('sha256').update(bytes).digest('hex');
}

async function nativeInputs(directory = root, relative = ''): Promise<Array<[string, string]>> {
  const entries = await readdir(directory, { withFileTypes: true });
  const found: Array<[string, string]> = [];
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    if (entry.name === '.git' || entry.name === 'target' || entry.name === 'node_modules' || entry.name === '.t3') continue;
    const absolute = path.join(directory, entry.name);
    const rel = path.join(relative, entry.name).split(path.sep).join('/');
    if (entry.isDirectory()) {
      found.push(...await nativeInputs(absolute, rel));
    } else if (/\.(rs|c|cc|cpp|cxx|h|hh|hpp|m|mm|swift|metal|plist)$/.test(entry.name) ||
      entry.name === 'Cargo.toml' || entry.name === 'Cargo.lock' || entry.name === 'build.rs' ||
      entry.name === 'rust-toolchain' || entry.name === 'rust-toolchain.toml' ||
      (relative === '.cargo' && entry.name.startsWith('config'))) {
      found.push([rel, sha256(await readFile(absolute))]);
    }
  }
  return found;
}

async function nativeSourceHash() {
  const inputs = await nativeInputs();
  const resources = new Set<string>();
  for (const [file] of inputs.filter(([file]) => file.endsWith('.rs'))) {
    const source = await readFile(path.join(root, file), 'utf8');
    for (const match of source.matchAll(/include_(?:str|bytes)!\(\s*"([^"]+)"/g)) {
      resources.add(path.resolve(root, path.dirname(file), match[1]));
    }
    for (const match of source.matchAll(/env!\("CARGO_MANIFEST_DIR"\),\s*"([^"]+)"/g)) {
      resources.add(path.resolve(root, '.' + match[1]));
    }
    if (file === 'build.rs') {
      for (const match of source.matchAll(/"((?:capture|shared|enablement)\/[^"\n]+)"/g)) {
        const candidate = path.join(root, match[1]);
        if ((await stat(candidate).catch(() => undefined))?.isFile()) resources.add(candidate);
      }
    }
  }
  for (const resource of resources) {
    if (!(await stat(resource).catch(() => undefined))?.isFile()) continue;
    const relative = path.relative(root, resource).split(path.sep).join('/');
    if (relative.startsWith('../')) throw new Error('Native resource is outside the checkout');
    if (!inputs.some(([file]) => file === relative)) inputs.push([relative, sha256(await readFile(resource))]);
  }
  inputs.sort(([a], [b]) => a.localeCompare(b));
  return { hash: sha256(JSON.stringify(inputs)), count: inputs.length };
}

async function runtimeIdentity(backend: string) {
  const listing = await run('ldd', [backend], { capture: true });
  if (listing.code !== 0) throw new Error('Cannot identify prepared backend shared libraries');
  const libraries = new Set<string>();
  for (const line of listing.stdout.split(/\r?\n/)) {
    if (line.includes('not found')) throw new Error('Prepared backend has a missing shared library');
    const file = line.match(/(?:=>\s+|^\s*)(\/.*?)\s+\(0x[0-9a-f]+\)/)?.[1];
    if (file) libraries.add(file);
  }
  for (const entry of await readdir(path.dirname(backend))) {
    if (/\.so(?:\.|$)/.test(entry)) libraries.add(path.join(path.dirname(backend), entry));
  }
  const files: Array<[string, string]> = [];
  for (const file of [...libraries].sort()) files.push([file, sha256(await readFile(file))]);
  return { files, preload: process.env.LD_PRELOAD || '', libraryPath: process.env.LD_LIBRARY_PATH || '' };
}

async function locateHarness(output: string) {
  const executable = output.match(/(?:^|\n)\s*Executable\s+.+?\s+\(([^()\n]+)\)(?:\n|$)/m)?.[1];
  if (!executable) throw new Error('Cargo output did not identify the Rust test executable');
  const resolved = path.resolve(root, executable);
  const info = await stat(resolved);
  if (!info.isFile()) throw new Error(`Rust test executable is not a file: ${resolved}`);
  return resolved;
}

async function prepareNative() {
  if (process.platform !== 'linux') throw new Error('The real Rust/Electron Settings bridge lane requires Linux');
  const before = await nativeSourceHash();
  const built = await run('cargo', ['test', '--offline', 'storage_electron_rust_roundtrip', '--no-run'], { capture: true });
  process.stdout.write(built.stdout);
  process.stderr.write(built.stderr);
  if (built.code !== 0) throw new Error(`Cargo preparation failed (${built.code})`);
  const backend = await locateHarness(`${built.stdout}\n${built.stderr}`);
  const listing = await run(backend, ['--list'], { capture: true });
  if (listing.code !== 0) throw new Error(`Prepared Rust harness could not list tests (${listing.code}): ${listing.stderr}`);
  const testName = STORAGE_BRIDGE_TEST;
  verifyBridgeListing(testName, listing.code, listing.stdout);
  const rustc = await run('rustc', ['-Vv'], { capture: true });
  if (rustc.code !== 0) throw new Error(`Could not record rustc version (${rustc.code})`);
  const inputs = await nativeSourceHash();
  if (inputs.hash !== before.hash) throw new Error("Native inputs changed during backend preparation; prepare again from a stable checkout");
  const manifest = {
    schema: 2,
    runtime: await runtimeIdentity(backend),
    createdAt: new Date().toISOString(),
    sourceCommit: (await run('git', ['rev-parse', 'HEAD'], { capture: true })).stdout.trim(),
    platform: process.platform,
    architecture: process.arch,
    rustc: rustc.stdout.trim(),
    nativeInputHash: inputs.hash,
    nativeInputCount: inputs.count,
    backend: path.relative(root, backend).split(path.sep).join('/'),
    backendSha256: sha256(await readFile(backend)),
    testName,
    buildCommand: 'cargo test --offline storage_electron_rust_roundtrip --no-run',
  };
  const output = process.argv[3] ? path.resolve(process.argv[3]) : receiptDefault;
  await mkdir(path.dirname(output), { recursive: true, mode: 0o700 });
  const temporary = `${output}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify(manifest, null, 2)}\n`, { mode: 0o600, flag: 'wx' });
  await rename(temporary, output);
  await chmod(output, 0o600);
  console.log(`Prepared native Settings bridge backend: ${backend}`);
  console.log(`Private compatibility receipt: ${output}`);
  console.log(`Native inputs: ${inputs.count} files, sha256 ${inputs.hash}`);
}

async function readVerifiedReceipt() {
  const receipt = path.resolve(process.argv[3] || receiptDefault);
  const manifest = JSON.parse(await readFile(receipt, 'utf8'));
  if (manifest.schema !== 2) throw new Error('Unsupported native bridge receipt schema');
  if (manifest.platform !== process.platform || manifest.architecture !== process.arch)
    throw new Error('Native bridge receipt is for a different platform or architecture');
  if (process.platform !== 'linux') throw new Error('The real Rust/Electron Settings bridge lane requires Linux');
  if (typeof manifest.backend !== 'string' || !/^target\/debug\/deps\/apparent-[a-f0-9]+$/.test(manifest.backend))
    throw new Error('Receipt does not identify the expected in-checkout Rust test executable');
  const inputs = await nativeSourceHash();
  if (manifest.nativeInputHash !== inputs.hash)
    throw new Error('Native source/build inputs changed; prepare a new backend receipt and rerun native integration gates');
  if (manifest.nativeInputCount !== inputs.count)
    throw new Error('Native input set changed; prepare a new backend receipt');
  const backend = path.resolve(root, manifest.backend);
  if (!(await stat(backend)).isFile() || sha256(await readFile(backend)) !== manifest.backendSha256)
    throw new Error('Prepared Rust test backend is missing or changed; prepare a new backend receipt');
  if (JSON.stringify(await runtimeIdentity(backend)) !== JSON.stringify(manifest.runtime))
    throw new Error('Prepared native runtime libraries or loader environment changed; prepare a new receipt');
  const listing = await run(backend, ['--list'], { capture: true });
  verifyBridgeListing(manifest.testName, listing.code, listing.stdout);
  return { manifest, backend, receipt };
}

async function runSettings() {
  const { manifest, backend, receipt } = await readVerifiedReceipt();
  console.log(`Using unchanged prebuilt Rust backend ${manifest.backend} (${manifest.backendSha256})`);
  console.log(`Verified native input hash ${manifest.nativeInputHash}; receipt ${receipt}`);
  const dist = process.env.ELECTRON_OVERRIDE_DIST_PATH;
  if (!dist || !path.isAbsolute(dist) || path.resolve(dist).includes(`${path.sep}node_modules${path.sep}`))
    throw new Error('Set ELECTRON_OVERRIDE_DIST_PATH to an absolute preinstalled Electron dist outside node_modules; the check never downloads Electron');
  const expectedElectron = path.join(dist, 'electron');
  const expectedInfo = await stat(expectedElectron).catch(() => undefined);
  if (!expectedInfo?.isFile()) throw new Error(`Preinstalled Electron executable is missing: ${expectedElectron}`);
  const electron = await run('node', ['-p', "require('electron')"], { cwd: frontend, capture: true });
  if (electron.code !== 0) throw new Error(`Could not resolve installed Electron executable: ${electron.stderr}`);
  const electronPath = electron.stdout.trim();
  if (!path.isAbsolute(electronPath)) throw new Error(`Electron did not resolve to an absolute path: ${electronPath}`);
  if (path.resolve(electronPath) !== path.resolve(expectedElectron))
    throw new Error(`Electron resolved outside the required preinstalled distribution: ${electronPath}`);

  for (const [command, args, cwd] of [
    ['npm', ['run', 'build'], frontend],
    ['npm', ['test'], frontend],
    ['xvfb-run', ['-a', 'npm', 'run', 'test:electron'], frontend],
  ] as const) {
    const result = await run(command, [...args], { cwd });
    if (result.code !== 0) throw new Error(`${command} ${args.join(' ')} failed (${result.code})`);
  }
  const beforeBridge = await readVerifiedReceipt();
  if (JSON.stringify(beforeBridge.manifest) !== JSON.stringify(manifest))
    throw new Error('Prepared receipt changed during Settings checks');
  const bridge = await run(backend, [manifest.testName, '--ignored', '--exact', '--test-threads=1'], {
    env: { ...process.env, APPARENT_STORAGE_TEST_ELECTRON: electronPath }, capture: true,
  });
  process.stdout.write(bridge.stdout);
  process.stderr.write(bridge.stderr);
  verifyBridgeResult(bridge.code, bridge.stdout);
  const afterBridge = await readVerifiedReceipt();
  if (JSON.stringify(afterBridge.manifest) !== JSON.stringify(manifest))
    throw new Error('Prepared receipt changed during bridge execution');
  console.log(`Settings checks passed using the prepared real Rust listener and Electron ${electronPath}`);
}

const action = process.argv[2];
try {
  if (action === 'prepare-native') await prepareNative();
  else if (action === 'test') await runSettings();
  else throw new Error('Usage: settings-check.ts <prepare-native [receipt-path] | test [receipt-path]>');
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
}
