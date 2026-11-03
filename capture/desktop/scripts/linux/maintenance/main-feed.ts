/** Mutable metadata only: immutable v2 and legacy release tags/assets stay untouched. */
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
const REPO = 'tryapparent/apparent';
const TAG = 'linux-main-feed';
const LEGACY = 'a5cf47d7dc7d0f89b1552cfb2d5e4b0ab489526a';
const LIMIT = 4 * 1024 * 1024;
const sha = (s: string) => /^[0-9a-f]{40}$/.test(s);
function run(cmd: string, args: string[]): string {
  return execFileSync(cmd, args, { encoding: 'utf8', maxBuffer: LIMIT, timeout: 120000 }).trim();
}
const git = (...args: string[]) => run('git', args);
const gh = (...args: string[]) => run('gh', args);
function requireThat(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
export function manifest(source: string, release: any, checksums: Record<string, string>, ancestors: string[], bridges: {id: string, source: string}[]) {
  requireThat(sha(source) && release.tag_name === `linux-main-v2-${source}` &&
    release.target_commitish === source && release.prerelease && !release.draft && release.published_at,
    'Release must identify the admitted public v2 source');
  requireThat(ancestors.length > 0 && ancestors.length <= 50000 && ancestors.includes(source) &&
    ancestors.every(sha) && new Set(ancestors).size === ancestors.length, 'Invalid complete ancestry');
  requireThat(bridges.length <= 100 && new Set(bridges.map(b => b.id)).size === bridges.length &&
    bridges.every(b => sha(b.id) && sha(b.source) && ancestors.includes(b.source)), 'Invalid bridges');
  requireThat(release.assets.length === 4 && Object.keys(checksums).length === 2, 'Expected exactly both architectures');
  const prefix = `https://github.com/${REPO}/releases/download/${release.tag_name}/`;
  for (const arch of ['x86_64', 'aarch64']) {
    const images = release.assets.filter((a: any) => new RegExp(`^Apparent-[0-9.]+-linux-${arch}\\.AppImage$`).test(a.name));
    requireThat(images.length === 1, 'Missing or ambiguous image');
    const image = images[0];
    const sums = release.assets.filter((a: any) => a.name === image.name + '.sha256');
    requireThat(sums.length === 1, 'Missing or ambiguous checksum');
    const sum = sums[0];
    const text = checksums[image.name];
    const match = /^([0-9a-f]{64})  ([^\r\n]+)\n?$/.exec(text || '');
    requireThat(match && match[2] === image.name && image.digest === `sha256:${match[1]}` &&
      sum.digest === 'sha256:' + createHash('sha256').update(text).digest('hex') &&
      [image, sum].every(a => a.state === 'uploaded' && a.browser_download_url === prefix + a.name),
      'Remote artifact/checksum identity differs');
  }
  const result = { schema: 1, source, release: {
    tag_name: release.tag_name, prerelease: true, draft: false, published_at: release.published_at,
    assets: release.assets.map((a: any) => ({ name: a.name, browser_download_url: a.browser_download_url })),
  }, checksums, ancestors: [...ancestors].sort(), bridges: [...bridges].sort((a,b) => a.id.localeCompare(b.id)) };
  requireThat(Buffer.byteLength(JSON.stringify(result)) <= LIMIT, 'Feed exceeds bound');
  return result;
}
export function mayAdvance(previous: any, source: string, ancestors: string[]): boolean {
  requireThat(previous.schema === 1 && sha(previous.source), 'Malformed previous feed');
  if (previous.source === source) return false;
  requireThat(ancestors.includes(previous.source), 'Refusing stale or diverged feed publication');
  return true;
}
export function history(source: string, git: (...args: string[]) => string = (...args) => run('git', args)) {
  requireThat(sha(source) && git('rev-parse', '--is-shallow-repository') === 'false', 'Full trusted history required');
  const ancestors = git('rev-list', source).split('\n');
  const bridges: {id:string, source:string}[] = [];
  for (const tag of git('tag', '--list', 'linux-main-*').split('\n')) {
    if (!/^linux-main-[0-9a-f]{40}$/.test(tag)) continue;
    const id = tag.slice('linux-main-'.length);
    requireThat(git('rev-parse', `${tag}^{commit}`) === id, 'Legacy tag moved');
    const parents = git('show', '-s', '--format=%P', id).split(' ');
    if (parents.length !== 2 || parents[0] !== LEGACY || !ancestors.includes(parents[1])) continue;
    requireThat(git('rev-parse', `${id}^{tree}`) === git('rev-parse', `${parents[1]}^{tree}`), 'Bridge tree mismatch');
    bridges.push({ id, source: parents[1] });
  }
  requireThat(ancestors.length <= 50000 && ancestors.every(sha), 'Ancestry exceeds bound');
  return { ancestors, bridges };
}
function publish(source: string) {
  requireThat(process.env.GITHUB_REPOSITORY === REPO && process.env.GITHUB_REF === 'refs/heads/main' && sha(source), 'Trusted main publication only');
  requireThat(git('rev-parse', '--is-shallow-repository') === 'false', 'Full trusted history required');
  git('merge-base', '--is-ancestor', source, 'origin/main');
  execFileSync('python3', ['enablement/release_gate.py', '--platform', 'linux', '--sha', source], {
    stdio: 'inherit', env: { ...process.env, GITHUB_OUTPUT: '/dev/null' }, timeout: 120000,
  });
  const release = JSON.parse(gh('api', `repos/${REPO}/releases/tags/linux-main-v2-${source}`));
  requireThat(git('rev-parse', `refs/tags/linux-main-v2-${source}^{commit}`) === source, 'Immutable release tag mismatch');
  const { ancestors, bridges } = history(source);
  const dir = mkdtempSync(join(tmpdir(), 'apparent-main-feed-'));
  try {
    requireThat(release.assets.length === 4 && release.assets.filter((a: any) => a.name.endsWith('.sha256')).every((a: any) => a.size > 0 && a.size <= 4096), 'Checksum assets exceed bounds');
    gh('release', 'download', release.tag_name, '--repo', REPO, '--pattern', '*.sha256', '--dir', dir);
    const checksums: Record<string,string> = {};
    for (const asset of release.assets) if (asset.name.endsWith('.AppImage')) {
      requireThat(/^Apparent-[0-9.]+-linux-(x86_64|aarch64)\.AppImage$/.test(asset.name), 'Invalid image filename');
      checksums[asset.name] = readFileSync(join(dir, asset.name + '.sha256'), 'utf8');
    }
    const feed = manifest(source, release, checksums, ancestors, bridges);
    // Enumerating release tags distinguishes absence from network/auth errors.
    const pages = JSON.parse(gh('api', `repos/${REPO}/releases?per_page=100`, '--paginate', '--slurp'));
    requireThat(Array.isArray(pages) && pages.every(Array.isArray), 'Malformed release history');
    for (const published of pages.flat()) {
      if (published.draft || !published.prerelease || !/^linux-main-v2-[0-9a-f]{40}$/.test(published.tag_name)) continue;
      const id = published.tag_name.slice('linux-main-v2-'.length);
      requireThat(git('rev-parse', `${published.tag_name}^{commit}`) === id, 'Published v2 tag moved');
      if (id !== source) requireThat(git('merge-base', source, id) !== source,
        'A newer immutable v2 release already exists; refusing stale feed');
    }
    const existing = pages.flat().find((r: any) => r.tag_name === TAG);
    if (existing) {
      requireThat(!existing.draft && existing.prerelease, 'Unexpected feed release state');
      if (existing.assets.some((a: any) => a.name === 'main.json')) {
        gh('release', 'download', TAG, '--repo', REPO, '--pattern', 'main.json', '--dir', dir);
        requireThat(statSync(join(dir, 'main.json')).size <= LIMIT, 'Previous feed exceeds bound');
        const previous = JSON.parse(readFileSync(join(dir, 'main.json'), 'utf8'));
        if (!mayAdvance(previous, source, ancestors)) return;
      }
    } else {
      gh('release', 'create', TAG, '--repo', REPO, '--target', source, '--prerelease', '--latest=false',
        '--title', 'Linux Main update metadata', '--notes', 'Mutable metadata; immutable builds remain in their v2 releases.');
    }
    const bytes = JSON.stringify(feed);
    writeFileSync(join(dir, 'main.json'), bytes);
    gh('release', 'upload', TAG, join(dir, 'main.json'), '--repo', REPO, '--clobber');
    rmSync(join(dir, 'main.json'));
    gh('release', 'download', TAG, '--repo', REPO, '--pattern', 'main.json', '--dir', dir);
    requireThat(readFileSync(join(dir, 'main.json'), 'utf8') === bytes, 'Feed publication readback mismatch');
  } finally { rmSync(dir, { recursive: true, force: true }); }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) publish(process.argv[2]);
