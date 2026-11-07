import type { CaptureManifest, NativeManifest, RuntimeFile } from './types.js';
import { DEFAULT_LIMITS } from './types.js';

function fail(message: string): never { throw new Error(message); }
function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
function exactKeys(value: Record<string, unknown>, expected: string[]): boolean {
  return JSON.stringify(Object.keys(value).sort()) === JSON.stringify(expected.slice().sort());
}
function safeRelativeFilePath(value: unknown): value is string {
  if (typeof value !== 'string' || value.length === 0 || value.startsWith('/') || value.includes('\\') || value.includes('\0')) return false;
  return value.split('/').every((part) => part !== '' && part !== '.' && part !== '..');
}

export function validateManifest(value: unknown): NativeManifest {
  if (!isRecord(value) || !Array.isArray(value.capabilities)) {
    fail('native media manifest has an invalid schema');
  }
  const media2 = JSON.stringify(value.capabilities) === JSON.stringify(['media/1', 'media/2']);
  const media1 = JSON.stringify(value.capabilities) === JSON.stringify(['media/1']);
  const hasVisualObservations = Object.hasOwn(value, 'visualObservations');
  const hasRecoveryLaunch = Object.hasOwn(value, 'recoveryLaunch');
  const expectedKeys = [
    'schema', 'sourceSha', 'target', 'rustTriple', 'capabilities', 'launch', 'environment', 'runtimeFiles',
    ...(media2 ? ['media2Launch'] : []),
    ...(media2 && hasVisualObservations ? ['visualObservations'] : []),
    ...(media2 && hasRecoveryLaunch ? ['recoveryLaunch'] : []),
  ];
  if ((!media1 && !media2) || !exactKeys(value, expectedKeys)) fail('native media manifest has an invalid schema');
  if (hasVisualObservations) {
    const profile = value.visualObservations;
    if (!media2 || !isRecord(profile) || !exactKeys(profile, ['schema', 'appliesTo', 'chunkType', 'algorithm']) ||
        profile.schema !== 'apparent.frame-observation-profile/1' || profile.appliesTo !== 'media/2-frame-png' ||
        profile.chunkType !== 'apOB' || profile.algorithm !== 'apparent.visual-luma/1') {
      fail('native visual observation profile rejected');
    }
  }
  if (hasRecoveryLaunch) {
    const recoveryLaunch = value.recoveryLaunch;
    const expectedRecoveryArgv = [
      '--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media',
      '--media-protocol-version', '2', '--recovery-peer-v1', '<private-peer-nonce>',
    ];
    if (!media2 || !isRecord(recoveryLaunch) || !exactKeys(recoveryLaunch, ['schema', 'channel', 'argv', 'startsFreshCapture', 'onInputEof']) ||
        recoveryLaunch.schema !== 'apparent.engine-recovery-peer/1' || recoveryLaunch.channel !== 'media/2' ||
        JSON.stringify(recoveryLaunch.argv) !== JSON.stringify(expectedRecoveryArgv) || recoveryLaunch.startsFreshCapture !== false ||
        recoveryLaunch.onInputEof !== 'retain-admitted-finite-work') {
      fail('native recovery launch metadata rejected');
    }
  }
  if (media2) {
    const media2Launch = value.media2Launch;
    const expectedMedia2Argv = ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2'];
    if (!isRecord(media2Launch) || !exactKeys(media2Launch, ['argv', 'sharesCaptureRuntime']) ||
        JSON.stringify(media2Launch.argv) !== JSON.stringify(expectedMedia2Argv) || media2Launch.sharesCaptureRuntime !== true) {
      fail('native media/2 launch contract rejected');
    }
  }
  const launch = value.launch;
  if (!isRecord(launch) || !exactKeys(launch, ['entry', 'argv', 'worker', 'pathLookup', 'runtimeBuilds'])) {
    fail('native media launch contract has an invalid schema');
  }
  const expectedArgv = ['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1'];
  if (value.schema !== 1 || typeof value.sourceSha !== 'string' || !/^[a-f0-9]{40}$/.test(value.sourceSha) ||
      value.target !== 'linux-x64' || value.rustTriple !== 'x86_64-unknown-linux-gnu' ||
      launch.entry !== 'apparent-engine-frames' || launch.worker !== 'libexec/apparent-engine-frames' ||
      launch.pathLookup !== false || launch.runtimeBuilds !== false || JSON.stringify(launch.argv) !== JSON.stringify(expectedArgv) ||
      !isRecord(value.environment) || !Array.isArray(value.runtimeFiles) || value.runtimeFiles.length > DEFAULT_LIMITS.maxFiles) {
    fail('native media manifest contract rejected');
  }

  const seen = new Set<string>();
  const runtimeFiles: RuntimeFile[] = [];
  for (const raw of value.runtimeFiles) {
    if (!isRecord(raw) || !exactKeys(raw, ['path', 'mode', 'sha256']) || !safeRelativeFilePath(raw.path) ||
        Buffer.byteLength(raw.path) > DEFAULT_LIMITS.maxPathBytes ||
        (raw.mode !== 0o644 && raw.mode !== 0o755) || typeof raw.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(raw.sha256)) {
      fail('native runtime inventory entry is malformed');
    }
    if (seen.has(raw.path)) fail(`duplicate native runtime path: ${raw.path}`);
    seen.add(raw.path);
    runtimeFiles.push({ path: raw.path, mode: raw.mode, sha256: raw.sha256 });
  }
  if (!seen.has(launch.entry) || !seen.has(launch.worker)) fail('native runtime entry or worker is undeclared');
  const entry = runtimeFiles.find((file) => file.path === launch.entry);
  const worker = runtimeFiles.find((file) => file.path === launch.worker);
  if (entry?.mode !== 0o755 || worker?.mode !== 0o755) fail('native entry and worker must be executable files');

  return { ...value, launch: { ...launch }, runtimeFiles } as NativeManifest;
}

export function validateCaptureManifest(value: unknown): CaptureManifest {
  const expectedRecoveryArgv = ['--runtime-directory', '<private-runtime-dir>', '--capture-channel', 'capture',
    '--capture-protocol-version', '2', '--recovery-peer-v1', '<private-peer-nonce>'];
  const expectedLaunchArgv = ['--runtime-directory', '<private-runtime-dir>', '--capture-channel', 'capture', '--capture-protocol-version', '2'];
  if (!isRecord(value) || !exactKeys(value, ['schema', 'sourceSha', 'recoveryLaunch', 'eligibleForReuse', 'target', 'rustTriple', 'capabilities', 'launch', 'environment', 'runtimeFiles'])) {
    fail('native capture manifest has an invalid schema');
  }
  const recovery = value.recoveryLaunch;
  if (!isRecord(recovery) || !exactKeys(recovery, ['schema', 'channel', 'argv', 'startsFreshCapture', 'onInputEof']) ||
      recovery.schema !== 'apparent.engine-recovery-peer/1' || recovery.channel !== 'capture/2' ||
      JSON.stringify(recovery.argv) !== JSON.stringify(expectedRecoveryArgv) || recovery.startsFreshCapture !== false ||
      recovery.onInputEof !== 'retain-admitted-finite-work') fail('native capture recovery contract rejected');
  const launch = value.launch;
  if (!isRecord(launch) || !exactKeys(launch, ['entry', 'argv', 'worker', 'pathLookup', 'runtimeBuilds', 'shutdownTimeoutMs']) ||
      launch.entry !== 'apparent-engine-capture-launch' || JSON.stringify(launch.argv) !== JSON.stringify(expectedLaunchArgv) ||
      launch.worker !== 'libexec/apparent-engine-capture' || launch.pathLookup !== false || launch.runtimeBuilds !== false ||
      launch.shutdownTimeoutMs !== 40000) fail('native capture launch contract rejected');
  if (value.schema !== 1 || typeof value.sourceSha !== 'string' || !/^[a-f0-9]{40}$/.test(value.sourceSha) ||
      value.eligibleForReuse !== true || value.target !== 'linux-x64' || value.rustTriple !== 'x86_64-unknown-linux-gnu' ||
      !Array.isArray(value.capabilities) || JSON.stringify(value.capabilities) !== JSON.stringify([
        'capture/2', 'x11-display', 'x11-area', 'retained-materials', 'authenticated-publication',
      ]) || !isRecord(value.environment) || !Array.isArray(value.runtimeFiles) ||
      value.runtimeFiles.length > DEFAULT_LIMITS.maxFiles) fail('native capture manifest contract rejected');

  const seen = new Set<string>();
  const runtimeFiles: RuntimeFile[] = [];
  for (const raw of value.runtimeFiles) {
    if (!isRecord(raw) || !exactKeys(raw, ['path', 'mode', 'sha256']) || !safeRelativeFilePath(raw.path) ||
        Buffer.byteLength(raw.path) > DEFAULT_LIMITS.maxPathBytes || (raw.mode !== 0o644 && raw.mode !== 0o755) ||
        typeof raw.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(raw.sha256)) fail('native capture inventory entry is malformed');
    if (seen.has(raw.path)) fail(`duplicate native capture path: ${raw.path}`);
    seen.add(raw.path);
    runtimeFiles.push({ path: raw.path, mode: raw.mode, sha256: raw.sha256 });
  }
  if (!seen.has(launch.entry) || !seen.has(launch.worker)) fail('native capture entry or worker is undeclared');
  if (runtimeFiles.find((file) => file.path === launch.entry)?.mode !== 0o755 ||
      runtimeFiles.find((file) => file.path === launch.worker)?.mode !== 0o755) fail('native capture entry and worker must be executable');
  return { ...value, launch: { ...launch }, runtimeFiles } as CaptureManifest;
}

export function sameInventory(actual: RuntimeFile[], expected: RuntimeFile[]): boolean {
  const sort = (files: RuntimeFile[]) => files.slice().sort((left, right) => left.path.localeCompare(right.path));
  const project = (files: RuntimeFile[]) => sort(files).map(({ path, mode, sha256 }) => ({ path, mode, sha256 }));
  return JSON.stringify(project(actual)) === JSON.stringify(project(expected));
}
