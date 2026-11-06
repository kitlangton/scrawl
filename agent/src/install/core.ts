import { createHash } from 'node:crypto';
import { chmod, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { copyInventoryFile, createPrivateDirectory, hashRegularFile, inventory, readRegularFile, sameInventorySnapshot, snapshotRegularFile, validateFreshPrefix } from './filesystem.js';
import { sameInventory, validateManifest } from './manifest.js';
import { makeLauncher } from './launcher.js';
import type { InstallerDependencies, InstallerInput, InventoryLimits } from './types.js';
import { DEFAULT_LIMITS, MAX_AGENT_ARCHIVE_BYTES, MAX_MANIFEST_BYTES } from './types.js';

const AGENT_VERSION = '0.1.0';
// Capture-only patch releases must not invalidate the independent media/1 consumer.
// New minor lines and prereleases require a separate compatibility assessment.
function supportedMediaSdkVersion(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  const match = /^0\.(?:2|3)\.(0|[1-9][0-9]*)$/.exec(value);
  return match !== null && Number.isSafeInteger(Number(match[1]));
}
const SHA256 = /^[a-f0-9]{64}$/;
const PRIVATE_MODE = 0o700;

function fail(message: string): never { throw new Error(message); }

function canonicalAbsolute(path: string, name: string): string {
  if (!isAbsolute(path) || resolve(path) !== path) fail(`${name} must be a canonical absolute path`);
  return path;
}

function equalHash(left: string, right: string): boolean { return SHA256.test(left) && SHA256.test(right) && left === right; }

function validatePackageMetadata(agent: unknown, sdk: unknown) {
  if (!agent || typeof agent !== 'object' || Array.isArray(agent)) fail('installed Agent package metadata is invalid');
  const agentPackage = agent as Record<string, any>;
  if (agentPackage.name !== '@apparent/agent' || agentPackage.version !== AGENT_VERSION ||
      agentPackage.bin?.['apparent-agent'] !== './dist/cli.js') fail('installed Agent package metadata mismatch');
  if (!sdk || typeof sdk !== 'object' || Array.isArray(sdk)) fail('installed Engine SDK package metadata is invalid');
  const sdkPackage = sdk as Record<string, any>;
  const mediaExport = sdkPackage.exports?.['./media'];
  if (sdkPackage.name !== '@apparent/engine-sdk' || !supportedMediaSdkVersion(sdkPackage.version) ||
      !mediaExport || mediaExport.import !== './dist/src/media.js' || mediaExport.types !== './dist/src/media.d.ts') {
    fail('installed SDK media export/version mismatch');
  }
  // Older admitted Agent archives bundled the SDK from the workspace. New ones
  // bundle a published tarball; its declared version must match the installed SDK.
  const sdkSource = agentPackage.dependencies?.['@apparent/engine-sdk'];
  if (sdkSource !== 'file:../engine/sdk' && sdkSource !== `file:vendor/apparent-engine-sdk-${sdkPackage.version}.tgz`)
    fail('installed Agent package metadata mismatch');
  return { agentPackage, sdkPackage, mediaImport: mediaExport.import.slice(2) };
}

export async function installLocal(
  input: InstallerInput,
  dependencies: InstallerDependencies,
  limits: InventoryLimits = DEFAULT_LIMITS,
): Promise<void> {
  const agentPath = canonicalAbsolute(input.agentPath, 'Agent archive path');
  const nativeRoot = canonicalAbsolute(input.nativeRoot, 'native root');
  const prefix = canonicalAbsolute(input.prefix, 'prefix');
  if (process.platform !== 'linux' || process.arch !== 'x64') fail('requires Linux x64');
  const [nodeMajor, nodeMinor] = process.versions.node.split('.').map(Number);
  if (nodeMajor < 22 || (nodeMajor === 22 && nodeMinor < 13)) fail('requires Node.js 22.13+');
  if (!SHA256.test(input.agentSha256) || !SHA256.test(input.manifestSha256)) fail('trusted SHA-256 values are required');

  const initialArchiveHash = await hashRegularFile(agentPath, MAX_AGENT_ARCHIVE_BYTES);
  if (!equalHash(initialArchiveHash, input.agentSha256)) fail('Agent tarball SHA-256 mismatch');

  const manifestPath = join(nativeRoot, 'manifest.json');
  const rawManifest = await readRegularFile(manifestPath, MAX_MANIFEST_BYTES);
  const manifestHash = createHash('sha256').update(rawManifest).digest('hex');
  if (!equalHash(manifestHash, input.manifestSha256)) fail('native manifest SHA-256 mismatch');
  const manifest = validateManifest(JSON.parse(rawManifest.toString('utf8')));
  const sourceInventory = await inventory(nativeRoot, limits);
  if (!sameInventory(sourceInventory, manifest.runtimeFiles)) fail('native root inventory does not exactly match manifest');

  // Validate every existing ancestor before creating the prefix or any staging file.
  await validateFreshPrefix(prefix);
  try {
    await createPrivateDirectory(prefix);
    await createPrivateDirectory(join(prefix, 'apparent-agent-private'));
    const installRoot = join(prefix, 'apparent-agent-private');
    const privateHome = join(prefix, 'private-home');
    const inputRoot = join(prefix, 'inputs');
    await createPrivateDirectory(join(prefix, 'npm-cache'));
    await createPrivateDirectory(join(prefix, 'npm-tmp'));
    await createPrivateDirectory(privateHome);
    await createPrivateDirectory(inputRoot);
    const snapshotPath = join(inputRoot, 'agent.tgz');
    await snapshotRegularFile(agentPath, snapshotPath, input.agentSha256, MAX_AGENT_ARCHIVE_BYTES);
    if (await hashRegularFile(snapshotPath, MAX_AGENT_ARCHIVE_BYTES) !== input.agentSha256) fail('Agent snapshot verification failed');
    await writeFile(join(privateHome, 'user.npmrc'), '', { mode: 0o600, flag: 'wx' });
    await writeFile(join(privateHome, 'global.npmrc'), '', { mode: 0o600, flag: 'wx' });

    dependencies.installNpm(snapshotPath, installRoot, privateHome);
    const agentPackagePath = join(installRoot, 'node_modules/@apparent/agent/package.json');
    const sdkPackagePath = join(installRoot, 'node_modules/@apparent/agent/node_modules/@apparent/engine-sdk/package.json');
    const agentPackage = JSON.parse((await readRegularFile(agentPackagePath, MAX_MANIFEST_BYTES)).toString('utf8'));
    const sdkPackage = JSON.parse((await readRegularFile(sdkPackagePath, MAX_MANIFEST_BYTES)).toString('utf8'));
    const { mediaImport, sdkPackage: validatedSdk } = validatePackageMetadata(agentPackage, sdkPackage);
    const cliPath = join(installRoot, 'node_modules/@apparent/agent/dist/cli.js');
    const mediaModule = join(dirname(sdkPackagePath), mediaImport);
    await readRegularFile(cliPath, MAX_MANIFEST_BYTES);
    await readRegularFile(mediaModule, MAX_MANIFEST_BYTES);
    const entry = join(prefix, 'native-media/apparent-engine-frames');
    const runtime = join(prefix, 'runtime');
    const nativeDestination = join(prefix, 'native-media');

    await createPrivateDirectory(nativeDestination);
    for (const item of sourceInventory) await copyInventoryFile(nativeRoot, nativeDestination, item, limits);
    const copiedManifest = join(nativeDestination, 'manifest.json');
    await snapshotRegularFile(manifestPath, copiedManifest, input.manifestSha256, MAX_MANIFEST_BYTES);
    await chmod(copiedManifest, 0o600);
    const sourceAfterCopy = await inventory(nativeRoot, limits);
    if (!sameInventory(sourceAfterCopy, manifest.runtimeFiles) || !sameInventorySnapshot(sourceInventory, sourceAfterCopy)) {
      fail('native source changed during installation');
    }
    if (!sameInventory(await inventory(nativeDestination, limits), sourceInventory)) fail('copied native inventory changed');
    await createPrivateDirectory(runtime);

    const launcherPath = join(prefix, 'apparent-agent');
    const launcher = makeLauncher(process.execPath, cliPath, entry, runtime);
    await writeFile(launcherPath, launcher, { mode: PRIVATE_MODE, flag: 'wx' });
    await chmod(launcherPath, PRIVATE_MODE);
    dependencies.smokeCli(cliPath, privateHome, installRoot);
    const result = await dependencies.handshake(mediaModule, entry, runtime);
    if (result.reply.channel !== 'media' || result.reply.protocolVersion !== 1 ||
        result.reply.implementation !== 'native' || result.reply.capabilities?.decodeFrames !== true) {
      fail('native media handshake rejected');
    }
    if (result.closeOutcome !== 'already-exited' && result.closeOutcome !== 'eof-exited') {
      fail('native worker close was uncertain');
    }

    const receipt = {
      schema: 1,
      kind: 'offline-local-agent-install',
      sourceSha: manifest.sourceSha,
      target: manifest.target,
      agentTarballSha256: input.agentSha256,
      nativeManifestSha256: input.manifestSha256,
      agentVersion: AGENT_VERSION,
      sdkVersion: validatedSdk.version,
      platform: 'linux-x64',
      cliPath,
      launcher: launcherPath,
      nativeEntry: entry,
      nativeWorker: join(nativeDestination, manifest.launch.worker),
      runtime,
      claims: [
        'local Agent consumer installed',
        'native media/1 handshake and confirmed close passed; no codec claim',
        'does not imply current-main artifact admission or global routing',
      ],
    };
    await writeFile(join(prefix, 'install-receipt.json'), JSON.stringify(receipt, null, 2) + '\n', {
      mode: 0o600,
      flag: 'wx',
    });
  } catch (error) {
    throw new Error(`Installation incomplete; private partial prefix retained at ${prefix}: ${(error as Error).message}`);
  }
}
