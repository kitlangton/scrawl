import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { manifest, mayAdvance } from './main-feed.ts';
const source = 'b'.repeat(40), previous = 'a'.repeat(40);
test('publisher reuses immutable comparisons while refreshing publication authority', () => {
  execFileSync('python3', ['-c', `
import importlib.util, json
spec = importlib.util.spec_from_file_location('publisher', 'capture/desktop/scripts/linux/maintenance/publish-main-preview.py')
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)
old, candidate, newer, main = [letter * 40 for letter in 'abcd']
calls = []
published = [old]
def fake_gh(*args):
    calls.append(args)
    endpoint = args[1]
    if endpoint.endswith('/git/ref/heads/main'):
        return json.dumps({'object': {'sha': main}})
    if endpoint.endswith('/releases?per_page=100'):
        return json.dumps([[{'tag_name': 'linux-main-v2-' + sha, 'draft': False, 'prerelease': True} for sha in published]])
    pair = endpoint.split('/compare/')[1].split('...')
    return json.dumps({'status': 'ahead' if pair[0] < pair[1] else 'behind'})
publisher.gh = fake_gh
assert publisher.publication_needed('example/repo', candidate)
assert publisher.publication_needed('example/repo', candidate)
comparisons = [call for call in calls if '/compare/' in call[1]]
assert len(comparisons) == 3
assert len([call for call in calls if '/git/ref/' in call[1]]) == 2
assert len([call for call in calls if '/releases?' in call[1]]) == 2
published.append(newer)
assert not publisher.publication_needed('example/repo', candidate)
main = 'e' * 40
assert not publisher.publication_needed('example/repo', candidate)
assert any(candidate + '...' + main in call[1] for call in calls)
`], { stdio: 'pipe' });
});
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
    // A shared local clone cannot fetch missing blobs from its donor's promisor.
    // Hydrate this excluded test input before making the fixture's bare remote.
    const cargoManifest = run(root, 'show', 'HEAD:Cargo.toml');
    const remote = join(dir, 'origin.git'), checkout = join(dir, 'checkout');
    run(dir, 'clone', '--bare', '--shared', root, remote);
    run(remote, 'config', 'uploadpack.allowFilter', 'true');
    run(dir, 'clone', '--filter=blob:none', '--no-checkout', '--no-tags', `file://${remote}`, checkout);
    const sparseBlocks = [...workflow.matchAll(/sparse-checkout-cone-mode: false\n          sparse-checkout: \|\n((?:            .*\n)+)/g)];
    assert.equal(sparseBlocks.length, 2, 'admission and publication retain bounded checkouts');
    const patterns = sparseBlocks.map(block => block[1].trim().split('\n').map(line => line.trim()));
    const smokeInputs = ['/capture/desktop/scripts/linux/appimage/smoke-cpu.sh',
      '/capture/desktop/scripts/linux/appimage/smoke-cpu.test.ts'];
    assert.deepEqual(patterns[1], ['/.github/workflows/release-linux.yml',
      '/capture/desktop/scripts/linux/maintenance/', '/enablement/release_gate.py']);
    assert.deepEqual(patterns[0].filter(path => !smokeInputs.includes(path)), patterns[1]);
    for (const path of smokeInputs) assert.ok(patterns[0].includes(path));
    run(checkout, 'sparse-checkout', 'set', '--no-cone', ...patterns[0]);
    run(checkout, 'checkout', 'HEAD');
    for (const file of ['enablement/release_gate.py', '.github/workflows/release-linux.yml',
      'capture/desktop/scripts/linux/maintenance/main-feed.ts',
      'capture/desktop/scripts/linux/maintenance/publish-main-preview.py']) {
      assert.equal(readFileSync(join(checkout, file), 'utf8'), run(remote, 'show', `HEAD:${file}`) + '\n');
    }
    assert.equal(existsSync(join(checkout, 'public/site')), false);
    execFileSync(process.execPath, ['--experimental-strip-types', '--test',
      'capture/desktop/scripts/linux/maintenance/signing-bridge.test.ts',
      'capture/desktop/scripts/linux/appimage/smoke-cpu.test.ts'], { cwd: checkout, stdio: 'pipe' });
    execFileSync('python3', ['enablement/release_gate.py', '--help'], { cwd: checkout, stdio: 'pipe' });
    run(checkout, 'sparse-checkout', 'set', '--no-cone', ...patterns[1]);
    assert.equal(existsSync(join(checkout, 'capture/desktop/scripts/linux/appimage/smoke-cpu.sh')), false);
    assert.ok(existsSync(join(checkout, 'capture/desktop/scripts/linux/maintenance/main-feed.ts')));
    assert.equal(run(checkout, 'config', '--get', 'remote.origin.partialclonefilter'), 'blob:none');
    assert.equal(run(checkout, 'rev-parse', '--is-shallow-repository'), 'false');
    assert.equal(run(checkout, 'rev-list', 'HEAD'), run(remote, 'rev-list', 'HEAD'));
    // Lazy materialization retains the exact current source tree and file bytes.
    assert.equal(run(checkout, 'rev-parse', 'HEAD^{tree}'), run(remote, 'rev-parse', 'HEAD^{tree}'));
    assert.equal(run(checkout, 'show', 'HEAD:Cargo.toml'), cargoManifest);
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
