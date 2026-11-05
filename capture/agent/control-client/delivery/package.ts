/** Produce the authenticated, target-specific capture-control component archive. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { publishValidatedDirectory } from '../../../../shared/packaging/transaction.ts';

const sha256 = (bytes: Buffer | string) => createHash('sha256').update(bytes).digest('hex');
const targets = ['x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu', 'x86_64-apple-darwin', 'aarch64-apple-darwin'];
function run(command: string, args: string[]) {
  const result = spawnSync(command, args, { encoding: 'utf8', timeout: 60_000, maxBuffer: 4 * 1024 * 1024 });
  assert.equal(result.status, 0, `${command}: ${result.stderr ?? result.error?.message}`);
  return result.stdout.trim();
}
function binaryTarget(bytes: Buffer) {
  if (bytes.length >= 20 && bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
    assert.equal(bytes[4], 2); assert.equal(bytes[5], 1);
    const arch = ({ 62: 'x86_64', 183: 'aarch64' })[bytes.readUInt16LE(18)];
    assert(arch, 'Unsupported ELF architecture'); return `${arch}-unknown-linux-gnu`;
  }
  assert(bytes.length >= 8 && bytes.readUInt32LE(0) === 0xfeedfacf, 'Expected ELF or thin 64-bit Mach-O');
  const arch = ({ 0x1000007: 'x86_64', 0x100000c: 'aarch64' })[bytes.readUInt32LE(4)];
  assert(arch, 'Unsupported Mach-O architecture'); return `${arch}-apple-darwin`;
}

/** Package only the already-tested binary; Cargo invocation belongs to the native producer job. */
export function pack(executable: string, source: string, target: string, destination: string) {
  assert(/^[a-f0-9]{40}$/.test(source) && targets.includes(target), 'Exact source and supported target required');
  assert.equal(run('git', ['rev-parse', 'HEAD']), source, 'Producer must run from its exact committed source');
  assert.equal(run('git', ['status', '--porcelain', '--untracked-files=normal']), '', 'Clean committed source required');
  const stat = lstatSync(executable); assert(stat.isFile() && !stat.isSymbolicLink() && (stat.mode & 0o111), 'Regular executable required');
  const bytes = readFileSync(executable); assert.equal(binaryTarget(bytes), target, 'Executable target mismatch');
  const root = resolve('capture/agent/control-client');
  const inputs = ['Cargo.toml', 'Cargo.lock', 'lib.rs', 'client.rs', 'wire.rs', 'main.rs'].map(path => ({
    path, sha256: sha256(readFileSync(join(root, path))),
  }));
  const manifest = { schema: 1, kind: 'agent-control-client', protocol: 1, executable: 'apparent-capture',
    sourceSha: source, target, inputsSha256: sha256(JSON.stringify(inputs)), inputs,
    executableSha256: sha256(bytes), size: bytes.length };
  const out = resolve(destination); mkdirSync(out, { mode: 0o700 });
  const payload = join(out, 'payload'); mkdirSync(payload, { mode: 0o700 });
  writeFileSync(join(payload, manifest.executable), bytes, { mode: 0o755, flag: 'wx' });
  writeFileSync(join(payload, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n', { mode: 0o644, flag: 'wx' });
  chmodSync(join(payload, manifest.executable), 0o755);
  chmodSync(join(payload, 'manifest.json'), 0o644);
  const archive = join(out, `apparent-capture-${source}-${target}.tar.gz`);
  run('tar', ['-czf', archive, '-C', payload, manifest.executable, 'manifest.json']);
  const digest = sha256(readFileSync(archive));
  writeFileSync(`${archive}.sha256`, `${digest}  ${basename(archive)}\n`, { flag: 'wx' });
  return { archive, digest, manifest };
}

/** Stage only caller-authenticated component bytes into an exclusive app candidate. */
export function stage(archive: string, expectedDigest: string, expectedSource: string, target: string, destination: string) {
  assert(/^[a-f0-9]{64}$/.test(expectedDigest) && /^[a-f0-9]{40}$/.test(expectedSource) && targets.includes(target),
    'Authenticated digest, source and supported target required');
  const snapshot = readFileSync(archive);
  assert.equal(sha256(snapshot), expectedDigest, 'Capture client archive digest mismatch');
  const output = resolve(destination);
  assert(!existsSync(output), 'Capture client destination already exists');
  const parent = lstatSync(dirname(output));
  assert(parent.isDirectory() && !parent.isSymbolicLink() && (parent.mode & 0o022) === 0 &&
    (process.getuid === undefined || parent.uid === process.getuid()), 'Trusted exclusive staging parent required');
  const temporary = mkdtempSync(join(dirname(output), '.capture-client-stage-'));
  chmodSync(temporary, 0o700);
  try {
    const input = join(temporary, 'input.tar.gz');
    writeFileSync(input, snapshot, { mode: 0o600, flag: 'wx' });
    const inventory = run('tar', ['-tzf', input]).trim().split('\n').sort();
    assert.deepEqual(inventory, ['apparent-capture', 'manifest.json'], 'Unexpected capture client archive inventory');
    assert(run('tar', ['-tvzf', input]).trim().split('\n').every(line => line.startsWith('-')), 'Only regular files accepted');
    run('tar', ['-xzf', input, '-C', temporary]);
    const executable = readFileSync(join(temporary, 'apparent-capture'));
    const manifestBytes = readFileSync(join(temporary, 'manifest.json'));
    const manifest = JSON.parse(manifestBytes.toString());
    assert(manifest.schema === 1 && manifest.kind === 'agent-control-client' && manifest.protocol === 1 &&
      manifest.executable === 'apparent-capture' && manifest.target === target &&
      /^[a-f0-9]{40}$/.test(manifest.sourceSha) && /^[a-f0-9]{64}$/.test(manifest.inputsSha256) &&
      Array.isArray(manifest.inputs) && manifest.inputs.length === 6 &&
      manifest.inputs.map((item: any) => item.path).sort().join(',') === 'Cargo.lock,Cargo.toml,client.rs,lib.rs,main.rs,wire.rs' &&
      manifest.inputs.every((item: any) => /^[a-f0-9]{64}$/.test(item.sha256)) &&
      /^[a-f0-9]{64}$/.test(manifest.executableSha256) && Number.isSafeInteger(manifest.size),
      'Invalid capture client manifest');
    assert.equal(manifest.sourceSha, expectedSource, 'Capture client source identity mismatch');
    assert.equal(sha256(JSON.stringify(manifest.inputs)), manifest.inputsSha256, 'Component input digest mismatch');
    assert.equal(binaryTarget(executable), target, 'Capture client executable target mismatch');
    assert.equal(sha256(executable), manifest.executableSha256, 'Capture client executable digest mismatch');
    assert.equal(executable.length, manifest.size, 'Capture client executable size mismatch');
    const candidate = join(temporary, 'candidate');
    mkdirSync(candidate, { mode: 0o700 });
    writeFileSync(join(candidate, 'apparent-capture'), executable, { mode: 0o755, flag: 'wx' });
    writeFileSync(join(candidate, 'manifest.json'), manifestBytes, { mode: 0o644, flag: 'wx' });
    chmodSync(join(candidate, 'apparent-capture'), 0o755);
    chmodSync(join(candidate, 'manifest.json'), 0o644);
    publishValidatedDirectory(candidate, output);
    assert.deepEqual(readdirSync(output).sort(), ['apparent-capture', 'manifest.json']);
    return manifest;
  } finally {
    rmSync(temporary, { recursive: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [operation, ...args] = process.argv.slice(2);
  if (operation === 'pack' && args.length === 4) console.log(JSON.stringify(pack(args[0], args[1], args[2], args[3])));
  else if (operation === 'stage' && args.length === 5) console.log(JSON.stringify(stage(args[0], args[1], args[2], args[3], args[4])));
  else throw new Error('Usage: package.ts pack EXECUTABLE SOURCE_SHA TARGET NEW_OUTPUT | stage ARCHIVE AUTHENTICATED_SHA256 SOURCE_SHA TARGET NEW_DESTINATION');
}
