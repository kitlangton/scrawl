/** Build policy for the existing Mac helper/shaders; Cargo retains native execution. */
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { chmodSync, closeSync, copyFileSync, existsSync, lstatSync, mkdtempSync, openSync, readFileSync, readSync, realpathSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const helperSources = [
  'capture/engine/native/Export.swift', 'capture/engine/native/LiveTranscribe.swift',
  'capture/engine/native/Media.swift', 'capture/engine/native/Mirror.swift',
  'capture/engine/native/PlaybackVolume.swift', 'capture/engine/native/ScreenProbe.swift',
  'capture/desktop/native/Onboarding.swift', 'capture/desktop/native/OnboardingBrand.swift',
  'capture/desktop/native/OnboardingDemo.swift', 'capture/desktop/native/PermissionPanel.swift',
  'capture/desktop/native/Settings.swift', 'capture/desktop/native/ShortcutKeys.swift',
  'shared/native/main.swift',
].sort((a, b) => {
  const left = a.slice(a.lastIndexOf('/') + 1), right = b.slice(b.lastIndexOf('/') + 1);
  return left < right ? -1 : left > right ? 1 : 0;
});
const shaders = [
  ['capture/desktop/native/laser_overlay.metal', 'laser_overlay'],
  ['capture/desktop/native/pill/pill.metal', 'pill'],
];
const environmentInputs = ['DEVELOPER_DIR', 'SDKROOT', 'TOOLCHAINS', 'MACOSX_DEPLOYMENT_TARGET',
  'CPATH', 'C_INCLUDE_PATH', 'CPLUS_INCLUDE_PATH', 'LIBRARY_PATH',
  'APPARENT_HELPER_BUNDLE_ID', 'APPARENT_PREVIEW_BUILD'];
const hash = (data: string | Buffer) => createHash('sha256').update(data).digest('hex');
function digest(path: string) {
  const fd = openSync(path, 'r'), value = createHash('sha256'), buffer = Buffer.alloc(65536);
  try {
    let size: number;
    while ((size = readSync(fd, buffer, 0, buffer.length, null)) > 0) value.update(buffer.subarray(0, size));
    return value.digest('hex');
  } finally { closeSync(fd); }
}
type Receipt = { input: string; outputs: Record<string, string> };

function command(args: string[], capture = false): string {
  const result = spawnSync('xcrun', args, { encoding: 'utf8', stdio: capture ? 'pipe' : 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`xcrun ${args[0]} failed (${result.signal ?? result.status})${capture ? ': ' + result.stderr : ''}`);
  return (result.stdout ?? '').trim();
}
function regular(path: string) {
  try { return lstatSync(path).isFile(); } catch { return false; }
}
function watch(path: string) { console.log(`cargo:rerun-if-changed=${path}`); }

export function buildNative(root: string, output: string, architecture: string) {
  root = realpathSync(root);
  output = realpathSync(output);
  const arch = architecture === 'aarch64' ? 'arm64' : architecture;
  if (!['arm64', 'x86_64'].includes(arch)) throw new Error('Unsupported Mac target architecture');
  const previewId = process.env.APPARENT_HELPER_BUNDLE_ID;
  if (previewId !== undefined && previewId !== 'dev.simonvanlaak.apparent.preview.helper') throw new Error('Unexpected helper bundle ID');
  const script = fileURLToPath(import.meta.url);
  watch(script);
  for (const key of environmentInputs) console.log(`cargo:rerun-if-env-changed=${key}`);
  const sourcePaths = [...helperSources, ...shaders.map(([path]) => path), 'shared/native/Info.plist'];
  for (const path of sourcePaths) watch(join(root, path));

  // Discover the actual selected tools/SDK before looking at any receipt. Neither
  // Cargo's root revision nor a cache file is evidence of compatible native inputs.
  const sdk = command(['--sdk', 'macosx', '--show-sdk-path'], true);
  const identity: unknown[] = [root, output, arch, digest(script),
    command(['--sdk', 'macosx', '--show-sdk-version'], true),
    command(['--sdk', 'macosx', '--show-sdk-build-version'], true), sdk,
    environmentInputs.map(key => [key, process.env[key] ?? null])];
  for (const tool of ['swiftc', 'metal', 'metallib']) {
    const path = realpathSync(command(tool === 'swiftc' ? ['--find', tool] : ['--sdk', 'macosx', '--find', tool], true));
    watch(path);
    // metallib has no portable version flag; its exact executable bytes bind it.
    identity.push([tool, path, digest(path), tool === 'metallib' ? null
      : command(tool === 'swiftc' ? [tool, '--version'] : ['--sdk', 'macosx', tool, '--version'], true)]);
  }
  // Swift's driver invokes a frontend and the platform linker separately.
  for (const tool of ['swift-frontend', 'clang', 'ld']) {
    const path = realpathSync(command(['--find', tool], true));
    watch(path);
    identity.push([tool, path, digest(path)]);
  }
  const settings = ['SDKSettings.json', 'SDKSettings.plist'].map(name => join(sdk, name)).filter(regular);
  if (!settings.length) throw new Error('Selected SDK has no settings identity');
  for (const path of settings) { watch(path); identity.push([path, digest(path)]); }
  if (existsSync('/var/db/xcode_select_link')) watch('/var/db/xcode_select_link');

  const manifest = join(output, 'native-build.json');
  let previous: Record<string, Receipt> = {};
  try { previous = JSON.parse(readFileSync(manifest, 'utf8')); } catch { /* Cold/malformed cache rebuilds. */ }
  if (!previous || typeof previous !== 'object' || Array.isArray(previous)) previous = {};
  const next: Record<string, Receipt> = {};
  const stage = mkdtempSync(join(output, '.native-build-'));
  const pending: string[] = [];
  try {
    function artifact(name: string, inputs: string[], flags: string[], outputs: string[], compile: () => void) {
      const input = hash(JSON.stringify([identity, inputs.map(path => [path, digest(join(root, path))]), flags]));
      const old = previous[name];
      const hit = old?.input === input && outputs.every(file => regular(join(output, file))
        && old.outputs?.[file] === digest(join(output, file)))
        && (name !== 'helper' || (lstatSync(join(output, 'apparent-helper')).mode & 0o111) !== 0);
      if (hit) {
        next[name] = old;
        console.log(`native-build ${name}: verified reuse`);
        return;
      }
      console.log(`native-build ${name}: compiling`);
      compile();
      for (const file of outputs) {
        if (!regular(join(stage, file)) || lstatSync(join(stage, file)).size === 0) throw new Error(`Missing native compiler output: ${file}`);
        pending.push(file);
      }
      next[name] = { input, outputs: Object.fromEntries(outputs.map(file => [file, digest(join(stage, file))])) };
    }
    for (const [source, name] of shaders) {
      const flags = ['--sdk', 'macosx', 'metal', '-c', '-mmacosx-version-min=15.0'];
      artifact(name, [source], flags, [`${name}.air`, `${name}.metallib`], () => {
        command([...flags, join(root, source), '-o', join(stage, `${name}.air`)]);
        command(['--sdk', 'macosx', 'metallib', join(stage, `${name}.air`), '-o', join(stage, `${name}.metallib`)]);
      });
    }
    const flags = ['swiftc', '-O', '-swift-version', '5', '-target', `${arch}-apple-macos15.0`];
    artifact('helper', [...helperSources, 'shared/native/Info.plist'], flags, ['apparent-helper'], () => {
      let plist = join(root, 'shared/native/Info.plist');
      if (previewId !== undefined) {
        const source = readFileSync(plist, 'utf8');
        const original = '<string>dev.kitlangton.apparent.helper</string>';
        if (!source.includes(original)) throw new Error('Helper bundle ID changed');
        plist = join(stage, 'preview-helper-Info.plist');
        writeFileSync(plist, source.replaceAll(original, `<string>${previewId}</string>`));
      }
      command([...flags, ...helperSources.map(path => join(root, path)), '-o', join(stage, 'apparent-helper'),
        '-Xlinker', '-sectcreate', '-Xlinker', '__TEXT', '-Xlinker', '__info_plist', '-Xlinker', plist]);
      if (!(lstatSync(join(stage, 'apparent-helper')).mode & 0o111)) throw new Error('Native helper is not executable');
    });
    // No receipt is published for a failed/partial compiler invocation.
    for (const file of pending) renameSync(join(stage, file), join(output, file));
    writeFileSync(join(stage, 'receipt.json'), JSON.stringify(next), { mode: 0o600 });
    renameSync(join(stage, 'receipt.json'), manifest);

    // Installation and runtime lookup expect this companion beside the binary.
    // Recreate it on hits too; never follow a pre-existing destination symlink.
    const profile = resolve(output, '../../..');
    const sibling = mkdtempSync(join(profile, '.native-helper-'));
    try {
      copyFileSync(join(output, 'apparent-helper'), join(sibling, 'apparent-helper'));
      chmodSync(join(sibling, 'apparent-helper'), lstatSync(join(output, 'apparent-helper')).mode & 0o777);
      renameSync(join(sibling, 'apparent-helper'), join(profile, 'apparent-helper'));
    } finally { rmSync(sibling, { recursive: true, force: true }); }
  } finally { rmSync(stage, { recursive: true, force: true }); }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [root, output, arch] = process.argv.slice(2);
  if (!root || !output || !arch || process.argv.length !== 5) throw new Error('Expected manifest directory, Cargo OUT_DIR and target architecture');
  buildNative(root, output, arch);
}
