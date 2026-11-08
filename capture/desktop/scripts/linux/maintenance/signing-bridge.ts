/** Compatibility publication for legacy Main updaters. No old ancestry enters Main. */
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

export const LEGACY = '1a55d5ebe05c948211ff74356c00b8dbfe58ecc6';
const REPO = 'tryapparent/apparent';
function run(command: string, args: string[]): string {
  return execFileSync(command, args, { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 }).trim();
}
function gh(...args: string[]): string { return run('gh', args); }
function api(path: string): any { return JSON.parse(gh('api', `repos/${REPO}/${path}`)); }
export function sha(value: string): string {
  if (!/^[0-9a-f]{40}$/.test(value)) throw new Error('Expected full lowercase commit SHA');
  return value;
}
export function sourceOfBridge(bridge: any, source: any): string {
  sha(bridge.sha); sha(source.sha); sha(source.tree?.sha);
  if (bridge.parents?.length !== 2 || bridge.parents[0].sha !== LEGACY ||
      bridge.parents[1].sha !== source.sha || bridge.tree?.sha !== source.tree.sha)
    throw new Error('Bridge must have exactly legacy/source parents and the unchanged source tree');
  return source.sha;
}
function inspect(id: string): string {
  const bridge = api(`git/commits/${sha(id)}`);
  if (bridge.sha !== id) throw new Error('Unexpected bridge identity');
  const source = api(`git/commits/${sha(bridge.parents?.[1]?.sha)}`);
  sourceOfBridge(bridge, source);
  if (!bridge.verification?.verified || bridge.author?.email !== '32648751+simonvanlaak@users.noreply.github.com')
    throw new Error('Bridge must have Simon’s correct identity and a verified signature');
  return source.sha;
}
function history(): any[] {
  return JSON.parse(gh('api', `repos/${REPO}/releases?per_page=100`, '--paginate', '--slurp')).flat();
}
export function legacyRevisions(releases: any[]): string[] {
  return releases.filter(r => !r.draft && r.prerelease && /^linux-main-[0-9a-f]{40}$/.test(r.tag_name))
    .map(r => sha(r.tag_name.slice('linux-main-'.length)));
}
function coversLegacy(bridge: string): void {
  for (const id of legacyRevisions(history())) {
    const result = api(`compare/${id}...${bridge}?per_page=1&page=2`);
    if (result.base_commit?.sha !== id || !['ahead', 'identical'].includes(result.status))
      throw new Error(`Bridge does not cover published legacy revision ${id}`);
  }
}
function admit(source: string): void {
  execFileSync('python3', ['enablement/release_gate.py', '--platform', 'linux', '--sha', sha(source)], {
    encoding: 'utf8', env: { ...process.env, GITHUB_REPOSITORY: REPO, GITHUB_OUTPUT: '/dev/null' },
  });
}
function publicSource(source: string): any {
  const release = api(`releases/tags/linux-main-v2-${sha(source)}`);
  if (release.draft || !release.prerelease || !release.published_at || release.target_commitish !== source)
    throw new Error('Canonical v2 release must already be public and identify the verified source');
  return release;
}
function prepare(source: string): void {
  admit(source); publicSource(source);
  if (gh('api', 'user', '--jq', '.login') !== 'simonvanlaak')
    throw new Error('Use Simon’s authenticated account and correct commit identity');
  run('git', ['fetch', 'origin', 'main']);
  run('git', ['fetch', 'origin', `refs/tags/linux-main-${LEGACY}:refs/tags/linux-main-${LEGACY}`]);
  if (run('git', ['rev-parse', 'FETCH_HEAD^{commit}']) !== LEGACY) throw new Error('Legacy tag moved');
  const tree = run('git', ['rev-parse', `${sha(source)}^{tree}`]);
  const bridge = sha(run('git', ['-c', 'user.name=Simon van Laak', '-c', 'user.email=32648751+simonvanlaak@users.noreply.github.com', 'commit-tree', '-S', tree, '-p', LEGACY, '-p', source,
    '-m', `Bridge legacy Linux updates to signed Main ${source}`]));
  // Compare before publishing any reference. Failed preparation leaves only an
  // unreachable signed object; it never changes a branch, index or working tree.
  for (const id of legacyRevisions(history())) {
    run('git', ['fetch', 'origin', `refs/tags/linux-main-${id}`]);
    run('git', ['merge-base', '--is-ancestor', id, bridge]);
  }
  // Push hooks require the exact pushed commit to be checked-out HEAD. Use an
  // owned detached checkout rather than changing the caller's branch or hooks.
  // Trust only Simon's publicly registered signing keys for this operation;
  // do not modify shared Git settings (which may contain stale filesystem paths).
  const directory = mkdtempSync(join(tmpdir(), 'apparent-bridge-checkout-'));
  const checkout = join(directory, 'checkout');
  let added = false;
  try {
    const keys = JSON.parse(gh('api', 'users/simonvanlaak/ssh_signing_keys'));
    if (!Array.isArray(keys) || !keys.length || keys.some(k => !/^ssh-ed25519 [A-Za-z0-9+/=]+$/.test(k.key)))
      throw new Error('Cannot establish Simon’s registered signing trust');
    const allowed = join(directory, 'allowed-signers');
    writeFileSync(allowed, keys.map(k => `32648751+simonvanlaak@users.noreply.github.com ${k.key}\n`).join(''), { mode: 0o600 });
    const config = ['-c', `gpg.ssh.allowedSignersFile=${allowed}`];
    run('git', [...config, 'verify-commit', bridge]);
    run('git', [...config, 'worktree', 'add', '--detach', checkout, bridge]);
    added = true;
    run('git', [...config, '-C', checkout, 'push', 'origin', `HEAD:refs/tags/linux-main-${bridge}`]);
  } finally {
    if (added) run('git', ['worktree', 'remove', checkout]);
    rmSync(directory, { recursive: true, force: true });
  }
  inspect(bridge);
  console.log(`Prepared signed bridge ${bridge}; source ${source}.`);
  console.log(`gh workflow run release-linux.yml --repo ${REPO} --ref main -f bridge_commit=${bridge}`);
}
function digest(path: string): string { return createHash('sha256').update(readFileSync(path)).digest('hex'); }
export function checkAssets(directory: string, assets: any[]): string[] {
  const names = assets.map(a => a.name).sort();
  const images = names.filter(n => /^Apparent-[0-9]+(?:\.[0-9]+)*-linux-(x86_64|aarch64)\.AppImage$/.test(n));
  if (images.length !== 2 || !images.some(n => n.endsWith('-x86_64.AppImage')) ||
      !images.some(n => n.endsWith('-aarch64.AppImage')) ||
      images[0].split('-linux-')[0] !== images[1].split('-linux-')[0] ||
      JSON.stringify(names) !== JSON.stringify(images.flatMap(n => [n, n + '.sha256']).sort()) ||
      JSON.stringify(readdirSync(directory).sort()) !== JSON.stringify(names))
    throw new Error('Expected exactly both same-version AppImages and checksums');
  for (const name of names) {
    const asset = assets.find(a => a.name === name);
    if (asset.state !== 'uploaded' || asset.digest !== `sha256:${digest(join(directory, name))}`)
      throw new Error(`Asset differs from verified v2 release: ${name}`);
  }
  for (const name of images) {
    if (readFileSync(join(directory, name + '.sha256'), 'utf8').trim() !== `${digest(join(directory, name))}  ${name}`)
      throw new Error(`Checksum mismatch: ${name}`);
  }
  return names;
}
function publish(bridge: string): void {
  if (process.env.GITHUB_ACTIONS !== 'true' || process.env.GITHUB_REF !== 'refs/heads/main' ||
      process.env.GITHUB_WORKFLOW !== 'Release (Linux)' || process.env.GITHUB_REPOSITORY !== REPO)
    throw new Error('Publish only inside the serialized trusted-main Linux release workflow');
  const source = inspect(bridge);
  admit(source);
  const canonical = publicSource(source);
  const tag = `linux-main-${bridge}`;
  if (api(`commits/${tag}`).sha !== bridge) throw new Error('Bridge tag moved');
  const matching = history().filter(r => r.tag_name === tag);
  if (matching.length) {
    // Never overwrite public assets, even after a failed acknowledgement.
    if (!matching[0].draft) throw new Error('Bridge already public; inspect it rather than replacing assets');
    throw new Error('Existing bridge draft requires inspection before retry');
  }
  const directory = mkdtempSync(join(tmpdir(), 'apparent-bridge-'));
  let created = false;
  try {
    gh('release', 'download', `linux-main-v2-${source}`, '--repo', REPO, '--dir', directory);
    const names = checkAssets(directory, canonical.assets);
    coversLegacy(bridge);
    created = true; // The remote may accept creation before gh loses its reply.
    gh('release', 'create', tag, ...names.map(n => join(directory, n)), '--repo', REPO,
      '--verify-tag', '--draft', '--prerelease', '--latest=false',
      '--title', 'Linux Main signing migration', '--notes',
      `Compatibility bridge only. Exact unchanged assets from linux-main-v2-${source}. Main retains corrected signed history.`);
    // Tag lookup only returns published releases; authenticated listings include drafts.
    const draft = history().find(r => r.tag_name === tag);
    if (!draft?.draft || draft.target_commitish !== bridge && draft.target_commitish !== 'main')
      throw new Error('Unexpected bridge draft');
    checkAssets(directory, draft.assets);
    coversLegacy(bridge); // A late old publisher must not be silently stranded.
    admit(source);
    if (api(`commits/${tag}`).sha !== bridge) throw new Error('Bridge tag moved before promotion');
    gh('release', 'edit', tag, '--repo', REPO, '--draft=false');
    const published = api(`releases/tags/${tag}`);
    if (published.draft || !published.prerelease || !published.published_at) throw new Error('Bridge publication not acknowledged');
    checkAssets(directory, published.assets);
    console.log(`Published ${tag}; exact verified source ${source}`);
  } finally {
    // Do not delete tags/history or retract a public release after readback failure.
    try {
      if (created) {
        const partial = history().find(r => r.tag_name === tag);
        if (partial?.draft) gh('release', 'delete', tag, '--repo', REPO, '--yes');
      }
    } finally { rmSync(directory, { recursive: true, force: true }); }
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [mode, id] = process.argv.slice(2);
  if (mode === 'prepare') prepare(sha(id));
  else if (mode === 'inspect') console.log(inspect(sha(id)));
  else if (mode === 'publish') publish(sha(id));
  else throw new Error('Usage: node --experimental-strip-types signing-bridge.ts prepare|inspect|publish <SHA>');
}
