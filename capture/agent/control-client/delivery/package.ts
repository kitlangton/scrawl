/** Produce the authenticated, target-specific capture-control component archive. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { lstatSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

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
  const archive = join(out, `apparent-capture-${source}-${target}.tar.gz`);
  run('tar', ['-czf', archive, '-C', payload, manifest.executable, 'manifest.json']);
  const digest = sha256(readFileSync(archive));
  writeFileSync(`${archive}.sha256`, `${digest}  ${basename(archive)}\n`, { flag: 'wx' });
  return { archive, digest, manifest };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [executable, source, target, destination] = process.argv.slice(2);
  assert(executable && source && target && destination, 'Usage: package.ts EXECUTABLE SOURCE_SHA TARGET NEW_OUTPUT');
  console.log(JSON.stringify(pack(executable, source, target, destination)));
}
