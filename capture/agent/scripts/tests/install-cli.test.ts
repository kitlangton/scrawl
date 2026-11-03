import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, readlinkSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { artifact, latestRelease } from '../install-cli.ts';
const source = 'a'.repeat(40);
const name = 'Apparent-0.3.13-linux-x86_64.AppImage';
const archive = '#!/bin/sh\nmkdir -p squashfs-root/usr/bin squashfs-root/usr/lib\nprintf \'#!/bin/sh\\necho synthetic-cli\\n\' > squashfs-root/usr/bin/apparent\nchmod +x squashfs-root/usr/bin/apparent\n';
function feed(revision = source) {
  return { schema: 1, source: revision, release: { tag_name: `linux-main-v2-${revision}`, draft: false, prerelease: true,
    published_at: '2026-11-03T00:00:00Z', assets: [{ name, browser_download_url: `https://github.com/tryapparent/apparent/releases/download/linux-main-v2-${revision}/${name}` }, { name: name + '.sha256', browser_download_url: `https://github.com/tryapparent/apparent/releases/download/linux-main-v2-${revision}/${name}.sha256` }] },
    checksums: { [name]: `${createHash('sha256').update(archive).digest('hex')}  ${name}\n` } };
}
test('selects an exact published Linux architecture and checksum', () => {
  assert.equal(artifact(feed(), 'x86_64').source, source);
  assert.equal(latestRelease([{...feed().release, tag_name: 'mac-main-new'}, {...feed().release, published_at:'2026-01-01'}, feed().release]).published_at, feed().release.published_at);
  assert.throws(() => latestRelease([]));
  assert.throws(() => artifact(feed(), 'aarch64'));
  for (const change of [
    (f: any) => { f.release.tag_name = 'latest'; },
    (f: any) => { f.release.assets[0].browser_download_url = 'https://example.org/payload'; },
    (f: any) => { f.release.assets.push(f.release.assets[0]); },
    (f: any) => { f.checksums[name] = '0'.repeat(64) + '  other.AppImage'; },
    (f: any) => { f.release.draft = true; },
  ]) { const f = feed(); change(f); assert.throws(() => artifact(f, 'x86_64')); }
});
test('fresh setup, latest feed refresh, checksum refusal and unrelated command preservation', () => {
  if (process.platform !== 'linux' || process.arch !== 'x64') return;
  const root = mkdtempSync(join(tmpdir(), 'apparent-installer-test-'));
  try {
    const home = join(root, 'home'), bin = join(root, 'bin');
    mkdirSync(home); mkdirSync(bin);
    writeFileSync(join(root, 'feed.json'), JSON.stringify([feed().release]));
    writeFileSync(join(root, 'image'), archive);
    writeFileSync(join(root, 'sum'), feed().checksums[name]);
    writeFileSync(join(bin, 'curl'), '#!/bin/sh\ncase "$*" in\n *api.github.com/repos/*) cat "$FIXTURE_ROOT/feed.json";;\n *.sha256*) cat "$FIXTURE_ROOT/sum";;\n *) for arg do if [ "$previous" = "--output" ]; then cp "$FIXTURE_ROOT/image" "$arg"; fi; previous=$arg; done;;\nesac\n', { mode: 0o700 });
    const installer = fileURLToPath(new URL('../install-cli.ts', import.meta.url));
    const env = { ...process.env, HOME: home, PATH: `${bin}:${process.env.PATH}`, FIXTURE_ROOT: root };
    const install = () => spawnSync(process.execPath, ['--experimental-strip-types', installer], { env, encoding: 'utf8', timeout: 30000 });
    let result = install(); assert.equal(result.status, 0, result.stderr);
    const link = join(home, '.local/bin/apparent');
    const first = readlinkSync(link);
    assert.match(first, new RegExp(source));
    assert.equal(execFileSync(link, ['--version'], { encoding: 'utf8' }).trim(), 'synthetic-cli');
    writeFileSync(join(root, 'feed.json'), JSON.stringify([feed('b'.repeat(40)).release]));
    result = install(); assert.equal(result.status, 0, result.stderr);
    assert.match(readlinkSync(link), /b{40}/);
    const current = readlinkSync(link);
    writeFileSync(join(root, 'image'), archive + '# tampered\n');
    result = install(); assert.notEqual(result.status, 0); assert.match(result.stderr, /Checksum mismatch/);
    assert.equal(readlinkSync(link), current);
    writeFileSync(join(root, 'image'), archive);
    rmSync(link); writeFileSync(link, 'unrelated command', { mode: 0o700 });
    result = install(); assert.equal(result.status, 0, result.stderr);
    assert.equal(readFileSync(link, 'utf8'), 'unrelated command');
    assert.match(result.stdout, /Existing command preserved/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
