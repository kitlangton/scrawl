import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { cacheKey, installTool, installTools, resolveTools } from './packaging-tools.ts';

const bytes = Buffer.from('synthetic executable');
const hash = createHash('sha256').update(bytes).digest('hex');
const tool = { name: 'linuxdeploy', url: 'https://example.invalid/tool', sha256: hash };
const response = (body: unknown) => new Response(JSON.stringify(body));
function release(arch: string, digest = `sha256:${hash}`) {
  return { assets: [{ name: `linuxdeploy-${arch}.AppImage`, digest,
    browser_download_url: `https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${arch}.AppImage` }] };
}
async function fixture(run: (directory: string) => Promise<void>) {
  const directory = await mkdtemp(join(tmpdir(), 'apparent-packaging-tools-'));
  try { await run(directory); } finally { await rm(directory, { recursive: true, force: true }); }
}

test('official digests and architecture determine cache identity', async () => {
  const x64 = await resolveTools('x64', async () => response(release('x86_64')), '');
  const arm = await resolveTools('arm64', async () => response(release('aarch64')), '');
  assert.notEqual(cacheKey(x64), cacheKey(arm));
  assert.equal(cacheKey(x64), cacheKey(await resolveTools('x64', async () => response(release('x86_64')), '')));
  const moved = await resolveTools('x64', async () => response(release('x86_64', `sha256:${'a'.repeat(64)}`)), '');
  assert.notEqual(cacheKey(x64), cacheKey(moved));
});
test('resolution refuses missing checksums, wrong origins, duplicate assets and unsupported architecture', async () => {
  await assert.rejects(resolveTools('ppc64'), /Unsupported/);
  for (const body of [release('x86_64', ''), { assets: [] },
    { assets: [...release('x86_64').assets, ...release('x86_64').assets] },
    { assets: [{ ...release('x86_64').assets[0], browser_download_url: 'https://example.invalid/tool' }] }]) {
    await assert.rejects(resolveTools('x64', async () => response(body), ''));
  }
  await assert.rejects(resolveTools('x64', async () => new Response('', { status: 503 }), ''), /HTTP 503/);
});
test('manifest validation precedes download or cache access', async () => {
  const manifest = await resolveTools('x64', async () => response(release('x86_64')), '');
  await fixture(async directory => {
    let requests = 0;
    const request = async () => { requests++; return new Response(bytes); };
    await assert.rejects(installTools({ ...manifest, arch: 'aarch64' }, directory, request, 'x64'));
    await assert.rejects(installTools({ ...manifest, tools: [manifest.tools[0], { ...manifest.tools[1], sha256: hash }] }, directory, request, 'x64'));
    assert.equal(requests, 0);
    assert.deepEqual(await readdir(directory), []);
  });
});
test('cold download and warm reuse produce verified executable bytes without another request', async () => {
  await fixture(async directory => {
    let requests = 0;
    const request = async () => { requests++; return new Response(bytes); };
    await installTool(tool, directory, request);
    await installTool(tool, directory, request);
    assert.equal(requests, 1);
    assert.deepEqual(await readFile(join(directory, tool.name)), bytes);
  });
});
test('corrupt cache is repaired, but a mismatched download never replaces it', async () => {
  await fixture(async directory => {
    const file = join(directory, tool.name);
    await writeFile(file, 'corrupt');
    await assert.rejects(installTool(tool, directory, async () => new Response('wrong download')), /SHA-256 mismatch/);
    assert.equal(await readFile(file, 'utf8'), 'corrupt');
    await installTool(tool, directory, async () => new Response(bytes));
    assert.deepEqual(await readFile(file), bytes);
    assert.deepEqual(await readdir(directory), [tool.name]);
  });
});
test('failed downloads and symlink cache entries cannot become executable tools', async () => {
  await fixture(async directory => {
    await assert.rejects(installTool(tool, directory, async () => new Response('', { status: 404 })), /HTTP 404/);
    assert.deepEqual(await readdir(directory), []);
    const other = join(directory, 'other');
    await writeFile(other, bytes);
    await symlink(other, join(directory, tool.name));
    await assert.rejects(installTool(tool, directory), /not a regular file/);
  });
});
