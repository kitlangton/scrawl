import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { downloadArtifact } from '@electron/get';
import { ELECTRON_VERSION, failedOfficialTarget, releaseApiDownloader } from '../../scripts/runtime/electron.ts';

const name = `electron-v${ELECTRON_VERSION}-linux-x64.zip`;
const canonical = `https://github.com/electron/electron/releases/download/v${ELECTRON_VERSION}/${name}`;
const bytes = Buffer.from('synthetic archive bytes; never extracted or executed');
const sha = createHash('sha256').update(bytes).digest('hex');
function transport(change?: (value: any) => void, body: Buffer = bytes) {
  const requests: string[] = [];
  const fetcher = (async (input: string | URL | Request) => {
    const url = String(input); requests.push(url);
    if (url.endsWith(`/tags/v${ELECTRON_VERSION}`)) {
      const value = { tag_name: `v${ELECTRON_VERSION}`, draft: false,
        assets: [{ id: 42, name, size: bytes.length, digest: `sha256:${sha}`, url: 'https://untrusted.invalid/ignored' }] };
      change?.(value); return new Response(JSON.stringify(value));
    }
    assert.equal(url, 'https://api.github.com/repos/electron/electron/releases/assets/42');
    return new Response(body.toString());
  }) as typeof fetch;
  return { requests, downloader: releaseApiDownloader({ platform: 'linux', arch: 'x64' }, sha, fetcher) };
}

test('real public downloader validates API bytes and warms the ordinary upstream installer cache key', async t => {
  const root = await mkdtemp(join(tmpdir(), 'electron-api-cache-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const f = transport();
  const options = { version: ELECTRON_VERSION, platform: 'linux', arch: 'x64', artifactName: 'electron', cacheRoot: root, checksums: { [name]: sha } };
  const zip = await downloadArtifact({ ...options, downloader: f.downloader });
  assert.deepEqual(await readFile(zip), bytes); assert.equal(f.requests.length, 2);
  // No custom transport: this is the ordinary upstream cache lookup/validator.
  const again = await downloadArtifact(options);
  assert.equal(again, zip); assert.equal(f.requests.length, 2);
});

test('real public checksum validation refuses same-size wrong bytes despite matching metadata claims', async t => {
  const root = await mkdtemp(join(tmpdir(), 'electron-api-wrong-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const f = transport(undefined, Buffer.alloc(bytes.length, 'x'));
  await assert.rejects(downloadArtifact({ version: ELECTRON_VERSION, platform: 'linux', arch: 'x64', artifactName: 'electron',
    cacheRoot: root, checksums: { [name]: sha }, downloader: f.downloader }), /checksum/i);
});

test('ambiguous/version/digest/size metadata and noncanonical URLs refuse before asset download', async t => {
  const root = await mkdtemp(join(tmpdir(), 'electron-api-input-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const invalid = [
    (v: any) => { v.tag_name = 'v99.0.0'; }, (v: any) => { v.draft = true; },
    (v: any) => { v.assets.push(v.assets[0]); }, (v: any) => { v.assets[0].digest = `sha256:${'a'.repeat(64)}`; },
    (v: any) => { v.assets[0].size = 1024 ** 3; }, (v: any) => { v.assets[0].id = 'other/path'; },
  ];
  for (const [index, change] of invalid.entries()) {
    const f = transport(change); await assert.rejects(f.downloader.download(canonical, join(root, String(index)), undefined));
    assert.equal(f.requests.length, 1);
  }
  const f = transport();
  await assert.rejects(f.downloader.download('https://other.invalid/archive', join(root, 'foreign'), undefined));
  assert.equal(f.requests.length, 0);
});

test('actual transport rejects truncated and oversized asset bodies', async t => {
  const root = await mkdtemp(join(tmpdir(), 'electron-api-length-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const [index, body] of [bytes.subarray(0, bytes.length - 1), Buffer.concat([bytes, Buffer.from('x')])].entries()) {
    const f = transport(undefined, body); await assert.rejects(f.downloader.download(canonical, join(root, String(index)), undefined), /incomplete|exceeds/);
  }
});

test('only official pinned 5xx installer failures select a supported cache-warming target', () => {
  assert.deepEqual(failedOfficialTarget(`HTTPError: Response code 504 (Gateway Time-out) for ${canonical}\n at vendor`), { platform: 'linux', arch: 'x64' });
  for (const value of [canonical, `HTTPError: Response code 404 (Not Found) for ${canonical}`,
    `HTTPError: Response code 504 for ${canonical}.evil`, `HTTPError: Response code 504 for ${canonical.replace('github.com', 'other.invalid')}`,
    `HTTPError: Response code 504 for ${canonical.replace('45.2.1', '44.0.0')}`, 'ChecksumMismatchError: invalid bytes']) {
    assert.equal(failedOfficialTarget(value), undefined);
  }
});
