import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { manifest, mayAdvance } from './main-feed.ts';
const source = 'b'.repeat(40), previous = 'a'.repeat(40);
test('feed step fetches immutable tags created after checkout without accepting moved tags', () => {
  const workflow = readFileSync(new URL('../../../../../.github/workflows/release-linux.yml', import.meta.url), 'utf8');
  const step = workflow.split('- name: Publish admitted immutable artifact identities and complete ancestry')[1]
    .split('- name: Publish unchanged verified assets')[0];
  assert.ok(step.indexOf('git fetch --tags origin') < step.indexOf('node --experimental-strip-types'));
  assert.ok(step.includes('git fetch --tags origin'));
  const dir = mkdtempSync(join(tmpdir(), 'apparent-feed-tags-'));
  const run = (cwd: string, ...args: string[]) => execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  try {
    const root = run(process.cwd(), 'rev-parse', '--show-toplevel');
    const remote = join(dir, 'origin.git'), checkout = join(dir, 'checkout');
    run(dir, 'clone', '--bare', '--shared', root, remote);
    run(remote, 'config', 'uploadpack.allowFilter', 'true');
    run(dir, 'clone', '--filter=blob:none', '--no-checkout', '--no-tags', `file://${remote}`, checkout);
    assert.equal(run(checkout, 'config', '--get', 'remote.origin.partialclonefilter'), 'blob:none');
    assert.equal(run(checkout, 'rev-parse', '--is-shallow-repository'), 'false');
    assert.equal(run(checkout, 'rev-list', 'HEAD'), run(remote, 'rev-list', 'HEAD'));
    // Lazy materialization retains the exact current source tree and file bytes.
    assert.equal(run(checkout, 'rev-parse', 'HEAD^{tree}'), run(remote, 'rev-parse', 'HEAD^{tree}'));
    assert.equal(run(checkout, 'show', 'HEAD:Cargo.toml'), run(remote, 'show', 'HEAD:Cargo.toml'));
    const head = run(checkout, 'rev-parse', 'HEAD'), tag = `refs/tags/linux-main-v2-${head}`;
    assert.throws(() => run(checkout, 'rev-parse', '--verify', `${tag}^{commit}`));
    run(remote, 'update-ref', tag, head); // Simulate the preceding release publisher.
    const fetch = step.match(/^          (git fetch --tags origin)$/m)![1].split(' ');
    run(checkout, ...fetch.slice(1));
    assert.equal(run(checkout, 'rev-parse', `${tag}^{commit}`), head);
    // Refuse an upstream tag move instead of overwriting the checked identity.
    run(remote, 'update-ref', tag, run(checkout, 'rev-parse', 'HEAD^'));
    assert.throws(() => run(checkout, ...fetch.slice(1)));
    assert.equal(run(checkout, 'rev-parse', `${tag}^{commit}`), head);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
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
