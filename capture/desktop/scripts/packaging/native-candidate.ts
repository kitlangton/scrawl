// Seal an installed, never-launched CI app for separate owned native verification.
import {execFileSync, spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {createReadStream, lstatSync, mkdirSync, readdirSync, readlinkSync, realpathSync, rmSync, writeFileSync} from 'node:fs';
import {basename, isAbsolute, join, relative, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import assert from 'node:assert/strict';

export function sourceSnapshot(root: string, expected: string) {
  const git = (...args: string[]) => execFileSync('git', args, {cwd: root, encoding: 'utf8'}).trim();
  assert.match(expected, /^[0-9a-f]{40}$/, 'Expected an exact checkout commit');
  const commit = git('rev-parse', 'HEAD');
  assert.equal(commit, expected, 'Checkout differs from dispatched source');
  assert.equal(git('status', '--porcelain', '--untracked-files=normal'), '', 'Source tree changed');
  return {commit, tree: git('rev-parse', 'HEAD^{tree}')};
}

export async function fingerprint(path: string) {
  const stat = lstatSync(path);
  assert.ok(stat.isFile(), 'Artifact input must be a regular file');
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return {sha256: hash.digest('hex'), bytes: stat.size};
}

export function requireArm64(host: {platform: string; node: string; machine: string; hardware: string}) {
  assert.deepEqual(host, {platform: 'darwin', node: 'arm64', machine: 'arm64', hardware: '1'},
    'Requires actual ARM64 hardware and an ARM64 Node runtime');
}

export function requireAdHoc(description: string) {
  assert.match(description, /^Signature=adhoc$/m, 'Expected an ad-hoc test candidate');
  assert.doesNotMatch(description, /^Authority=/m, 'Candidate must not use release signing');
}

type Entry = {kind: 'file'; sha256: string; bytes: number; mode: number} | {kind: 'link'; target: string};
export async function inventory(app: string): Promise<Record<string, Entry>> {
  const root = realpathSync(app);
  const entries: Record<string, Entry> = {};
  async function visit(directory: string) {
    for (const name of readdirSync(directory).sort()) {
      const path = join(directory, name), key = relative(root, path), stat = lstatSync(path);
      if (stat.isSymbolicLink()) {
        const target = readlinkSync(path), resolved = relative(root, realpathSync(path));
        assert.ok(!isAbsolute(target) && resolved !== '..' && !resolved.startsWith('../'),
          `Packaged link escapes the app: ${key}`);
        entries[key] = {kind: 'link', target};
      } else if (stat.isDirectory()) await visit(path);
      else {
        assert.ok(stat.isFile(), `Unexpected packaged file type: ${key}`);
        entries[key] = {kind: 'file', ...await fingerprint(path), mode: stat.mode & 0o777};
      }
    }
  }
  await visit(root);
  return entries;
}

async function main() {
  const output = (command: string, args: string[]) => execFileSync(command, args, {encoding: 'utf8'}).trim();
  const host = {platform: process.platform, node: process.arch, machine: output('uname', ['-m']),
    hardware: output('sysctl', ['-n', 'hw.optional.arm64'])};
  requireArm64(host);
  assert.equal(process.env.GITHUB_ACTIONS, 'true');
  assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted');
  const root = process.cwd();
  const source = sourceSnapshot(root, process.env.GITHUB_SHA ?? '');
  const temporary = process.env.RUNNER_TEMP;
  assert.ok(temporary);
  const app = join(temporary, 'Apparent Native Candidate.app');
  assert.ok(lstatSync(app).isDirectory(), 'Expected the newly packaged app');
  const executables = ['Contents/MacOS/apparent', 'Contents/MacOS/apparent-helper',
    'Contents/Resources/settings/runtime/Electron.app/Contents/MacOS/Electron',
    'Contents/Resources/settings/runtime/Electron.app/Contents/Frameworks/Electron Framework.framework/Versions/A/Electron Framework'];
  for (const relative of executables) {
    assert.equal(output('lipo', ['-archs', join(app, relative)]), 'arm64', relative);
  }
  execFileSync('codesign', ['--verify', '--deep', '--strict', app]);
  const signature = spawnSync('codesign', ['-d', '--verbose=2', app], {encoding: 'utf8'});
  assert.equal(signature.status, 0, signature.stderr);
  requireAdHoc(signature.stderr);
  const out = join(temporary, 'apparent-native-candidate');
  mkdirSync(out, {mode: 0o700});
  const archive = join(out, 'Apparent-Native-Candidate-arm64.zip');
  const files: Record<string, {sha256: string; bytes: number}> = {};
  for (const relative of executables) {
    files[relative] = await fingerprint(join(app, relative));
  }
  // Includes every shipped owner worker/policy byte without importing an owner
  // or requiring a not-yet-adopted package on other candidate branches.
  const contents = await inventory(app);
  execFileSync('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', app, archive]);
  const inspection = join(out, 'archive-inspection');
  mkdirSync(inspection, {mode: 0o700});
  execFileSync('ditto', ['-x', '-k', archive, inspection]);
  const restored = join(inspection, basename(app));
  assert.deepEqual(await inventory(restored), contents, 'Archive contents differ from the packaged app');
  execFileSync('codesign', ['--verify', '--deep', '--strict', restored]);
  rmSync(inspection, {recursive: true});
  assert.deepEqual(sourceSnapshot(root, source.commit), source);
  const manifest = {schema: 1, source, repository: process.env.GITHUB_REPOSITORY,
    run: {id: process.env.GITHUB_RUN_ID, attempt: process.env.GITHUB_RUN_ATTEMPT},
    architecture: 'arm64', host, signing: 'ad-hoc', nativeAcceptance: 'pending', releaseAdmission: false,
    compiler: execFileSync('rustc', ['-Vv'], {encoding: 'utf8'}).trim(),
    app: basename(app), artifact: {name: basename(archive), ...await fingerprint(archive)}, files, contents};
  writeFileSync(join(out, 'candidate.json'), JSON.stringify(manifest, null, 2) + '\n', {mode: 0o600});
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await main();
