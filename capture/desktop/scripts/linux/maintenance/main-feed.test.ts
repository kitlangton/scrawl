import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { manifest, mayAdvance } from './main-feed.ts';
const source = 'b'.repeat(40), previous = 'a'.repeat(40);
function fixture() {
  const tag = 'linux-main-v2-' + source;
  const checksums: Record<string,string> = {};
  const assets: any[] = [];
  for (const arch of ['aarch64', 'x86_64']) {
    const name = `Apparent-0.3.13-linux-${arch}.AppImage`;
    const hash = createHash('sha256').update(arch).digest('hex');
    checksums[name] = `${hash}  ${name}\n`;
    for (const [filename, digest] of [[name, hash], [name + '.sha256', createHash('sha256').update(checksums[name]).digest('hex')]]) {
      assets.push({ name: filename, digest: 'sha256:' + digest, state: 'uploaded',
        browser_download_url: `https://github.com/tryapparent/apparent/releases/download/${tag}/${filename}` });
    }
  }
  return { tag_name: tag, target_commitish: source, prerelease: true, draft: false, published_at: '2026-11-03T00:00:00Z', assets, checksums };
}
test('binds both artifact and checksum identities to complete ancestry', () => {
  const f = fixture(); const result = manifest(source, f, f.checksums, [source, previous], []);
  assert.equal(result.source, source); assert.equal(result.release.assets.length, 4);
  assert.deepEqual(result.ancestors, [previous, source]);
});
test('rejects artifact, checksum, origin, source and ancestry tampering', () => {
  for (const mutate of [
    (f: any) => f.assets[0].digest = 'sha256:' + '0'.repeat(64),
    (f: any) => f.assets[1].digest = 'sha256:' + '0'.repeat(64),
    (f: any) => f.assets[0].browser_download_url = 'https://evil.invalid/image',
    (f: any) => f.target_commitish = previous,
    (f: any) => f.assets.push(f.assets[0]),
    (f: any) => f.draft = true,
  ]) { const f = fixture(); mutate(f); assert.throws(() => manifest(source, f, f.checksums, [source, previous], [])); }
  const f = fixture();
  for (const ancestors of [[previous], [source, source], [source, 'bad']])
    assert.throws(() => manifest(source, f, f.checksums, ancestors, []));
  assert.throws(() => manifest(source, f, f.checksums, [source], [{id: previous, source: previous}]));
});
test('never regresses or rewrites same-source metadata', () => {
  assert.equal(mayAdvance({schema:1, source: previous}, source, [source, previous]), true);
  assert.equal(mayAdvance({schema:1, source}, source, [source]), false);
  assert.throws(() => mayAdvance({schema:1, source}, previous, [previous]));
  assert.throws(() => mayAdvance({schema:2, source: previous}, source, [source, previous]));
});

test('history requires full Git traversal and authenticates bridge tree and parents', async () => {
  const { history } = await import('./main-feed.ts');
  const bridge = 'f'.repeat(40), tree = 'e'.repeat(40);
  const legacy = 'a5cf47d7dc7d0f89b1552cfb2d5e4b0ab489526a';
  const responses: Record<string,string> = {
    'rev-parse --is-shallow-repository':'false',
    [`rev-list ${source}`]:`${source}\n${previous}`,
    'tag --list linux-main-*':`linux-main-${bridge}`,
    [`rev-parse linux-main-${bridge}^{commit}`]:bridge,
    [`show -s --format=%P ${bridge}`]:`${legacy} ${previous}`,
    [`rev-parse ${bridge}^{tree}`]:tree,
    [`rev-parse ${previous}^{tree}`]:tree,
  };
  const git = (...args: string[]) => { assert.ok(args.join(' ') in responses); return responses[args.join(' ')]; };
  assert.deepEqual(history(source, git), {ancestors:[source,previous],bridges:[{id:bridge,source:previous}]});
  responses[`rev-parse ${bridge}^{tree}`] = 'd'.repeat(40);
  assert.throws(() => history(source, git), /tree mismatch/);
  responses['rev-parse --is-shallow-repository'] = 'true';
  assert.throws(() => history(source, git), /Full trusted history/);
});
