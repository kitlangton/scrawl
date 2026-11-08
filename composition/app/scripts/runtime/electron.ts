import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { spawnSync } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { downloadArtifact, type Downloader } from '@electron/get';

export const ELECTRON_VERSION = '45.2.1';
export type ElectronTarget = { platform: 'linux' | 'darwin'; arch: 'x64' | 'arm64' };
const require = createRequire(import.meta.url);
const api = 'https://api.github.com/repos/electron/electron/releases';
const filename = ({ platform, arch }: ElectronTarget) => `electron-v${ELECTRON_VERSION}-${platform}-${arch}.zip`;
const browserUrl = (target: ElectronTarget) => `https://github.com/electron/electron/releases/download/v${ELECTRON_VERSION}/${filename(target)}`;
function target(value: ElectronTarget) {
  assert(['linux', 'darwin'].includes(value.platform) && ['x64', 'arm64'].includes(value.arch), 'Unsupported Electron target');
  return { platform: value.platform, arch: value.arch };
}
function pinnedChecksums(): Record<string, string> {
  assert.equal(require('electron/package.json').version, ELECTRON_VERSION, 'Electron runtime and package pin differ');
  return require('electron/checksums.json');
}

async function metadata(response: Response): Promise<any> {
  assert(response.ok && response.body, `Official Electron release metadata unavailable (${response.status})`);
  const reader = response.body.getReader(), chunks: Uint8Array[] = []; let bytes = 0;
  try {
    for (;;) {
      const next = await reader.read(); if (next.done) break;
      bytes += next.value.length; assert(bytes <= 2 * 1024 * 1024, 'Electron release metadata exceeds bound'); chunks.push(next.value);
    }
  } finally { await reader.cancel(); }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}

/** Public @electron/get transport seam. The cache key and checksum validator
 * remain unchanged; only the same publisher's download transport differs. */
export function releaseApiDownloader(input: ElectronTarget, checksum: string, request: typeof fetch = fetch): Downloader<any> {
  const selected = target(input), name = filename(selected);
  assert.match(checksum, /^[a-f0-9]{64}$/);
  return { async download(url, destination) {
    assert.equal(url, browserUrl(selected), 'Official API transport cannot substitute another URL');
    const release = await metadata(await request(`${api}/tags/v${ELECTRON_VERSION}`, {
      headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'Apparent-build' },
      signal: AbortSignal.timeout(15_000), redirect: 'error',
    }));
    assert.equal(release.tag_name, `v${ELECTRON_VERSION}`); assert.equal(release.draft, false);
    assert(Array.isArray(release.assets) && release.assets.length <= 512);
    const assets = release.assets.filter((asset: any) => asset?.name === name);
    assert.equal(assets.length, 1, 'Electron release asset is missing or ambiguous');
    const asset = assets[0];
    assert(Number.isSafeInteger(asset.id) && asset.id > 0);
    assert(Number.isSafeInteger(asset.size) && asset.size > 0 && asset.size <= 512 * 1024 * 1024);
    assert.equal(asset.digest, `sha256:${checksum}`, 'Official asset and pinned package checksum differ');
    const response = await request(`${api}/assets/${asset.id}`, {
      headers: { Accept: 'application/octet-stream', 'User-Agent': 'Apparent-build' },
      signal: AbortSignal.timeout(120_000),
    });
    assert(response.ok && response.body, `Official Electron asset unavailable (${response.status})`);
    await mkdir(dirname(destination), { recursive: true });
    let bytes = 0;
    await pipeline(Readable.fromWeb(response.body as any), async function* (source) {
      for await (const chunk of source) { bytes += chunk.length; assert(bytes <= asset.size, 'Electron asset exceeds declared size'); yield chunk; }
    }, createWriteStream(destination, { flags: 'wx', mode: 0o600 }));
    assert.equal(bytes, asset.size, 'Electron asset is incomplete');
    // @electron/get subsequently checks actual bytes against the npm-pinned
    // checksum before publishing its normal cache entry. No skip flag/mirror.
  } };
}

export function failedOfficialTarget(stderr: string): ElectronTarget | undefined {
  const match = stderr.match(/HTTPError: Response code 5\d\d[^\r\n]* for https:\/\/github\.com\/electron\/electron\/releases\/download\/v45\.2\.1\/electron-v45\.2\.1-(linux|darwin)-(x64|arm64)\.zip(?:\s|$)/);
  return match ? { platform: match[1] as ElectronTarget['platform'], arch: match[2] as ElectronTarget['arch'] } : undefined;
}

export async function pinnedElectronZip(input: ElectronTarget, apiOnly = false): Promise<string> {
  const selected = target(input), checksums = pinnedChecksums(), checksum = checksums[filename(selected)];
  assert.match(checksum ?? '', /^[a-f0-9]{64}$/);
  const options = { version: ELECTRON_VERSION, artifactName: 'electron', ...selected, checksums,
    cacheRoot: process.env.electron_config_cache, downloadOptions: { signal: AbortSignal.timeout(120_000) } };
  if (!apiOnly) {
    try { return await downloadArtifact(options); }
    catch (error) {
      const response = (error as { response?: Response }).response;
      // Do not hide extraction, checksum, permission, non-5xx or custom-mirror failures.
      if (!response || response.status < 500 || response.status > 599 || response.url !== browserUrl(selected)) throw error;
    }
  }
  return downloadArtifact({ ...options, downloader: releaseApiDownloader(selected, checksum) });
}

/** Keep the upstream installer, its platform/Rosetta selection, extraction and
 * installed fast path. Only a diagnosed official 5xx gets one verified cache warm. */
export async function prepareElectron(): Promise<void> {
  assert(process.platform === 'linux' || process.platform === 'darwin', 'Unsupported Electron host');
  pinnedChecksums();
  const install = () => spawnSync(process.execPath, [require.resolve('electron/install.js')], {
    encoding: 'utf8', timeout: 120_000, maxBuffer: 2 * 1024 * 1024, killSignal: 'SIGKILL',
  });
  const first = install();
  if (!first.error && first.status === 0) return;
  const selected = !first.error && failedOfficialTarget(first.stderr ?? '');
  if (!selected) throw first.error ?? new Error(`Electron preparation failed: ${first.stderr}`);
  console.error('Official Electron download returned 5xx; verifying the same release through its API.');
  await pinnedElectronZip(selected, true);
  const second = install();
  if (second.error || second.status !== 0) throw second.error ?? new Error(`Electron preparation failed: ${second.stderr}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  assert.equal(process.argv.length, 2, 'Electron preparation accepts no override arguments');
  await prepareElectron();
}
