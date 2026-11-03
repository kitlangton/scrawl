/** Headless Linux CLI installer. Selects the newest published Linux preview, never a Mac release. */
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, lstatSync, mkdirSync, mkdtempSync, readlinkSync, renameSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const releasesURL = 'https://api.github.com/repos/tryapparent/apparent/releases?per_page=100';
export function latestRelease(releases: any[]) {
  const published = releases.filter(r => /^linux-main-v2-[a-f0-9]{40}$/.test(r.tag_name) &&
    r.draft === false && r.prerelease === true && Number.isFinite(Date.parse(r.published_at)));
  published.sort((a, b) => Date.parse(b.published_at) - Date.parse(a.published_at));
  if (!published.length) throw new Error('No published Linux preview found in the latest 100 releases');
  return published[0];
}
export function artifact(feed: any, arch: string) {
  if (feed.schema !== 1 || !/^[a-f0-9]{40}$/.test(feed.source) ||
      feed.release?.tag_name !== `linux-main-v2-${feed.source}` ||
      feed.release.draft !== false || feed.release.prerelease !== true) throw new Error('Invalid Linux publication feed');
  const matches = feed.release.assets.filter((a: any) =>
    new RegExp(`^Apparent-[0-9.]+-linux-${arch}\\.AppImage$`).test(a.name));
  if (matches.length !== 1) throw new Error('Missing or ambiguous architecture asset');
  const image = matches[0];
  const prefix = `https://github.com/tryapparent/apparent/releases/download/${feed.release.tag_name}/`;
  if (image.browser_download_url !== prefix + image.name) throw new Error('Unexpected download origin or release');
  const checksum = /^([a-f0-9]{64})  ([^\r\n]+)\n?$/.exec(feed.checksums?.[image.name] ?? '');
  if (!checksum || checksum[2] !== image.name) throw new Error('Missing or invalid checksum');
  return { source: feed.source, name: image.name, url: image.browser_download_url, sha256: checksum[1] };
}
function run(cmd: string, args: string[], cwd?: string) {
  return execFileSync(cmd, args, { cwd, encoding: 'utf8', timeout: 360000, maxBuffer: 4 * 1024 * 1024 });
}
export async function install() {
  if (process.platform !== 'linux' || !['x64', 'arm64'].includes(process.arch)) throw new Error('This installer requires Linux x86_64 or aarch64');
  const arch = process.arch === 'x64' ? 'x86_64' : 'aarch64';
  const downloadText = (url: string) => run('curl', ['--fail', '--silent', '--show-error', '--location', '--proto', '=https', '--proto-redir', '=https', '--max-time', '60', url]);
  const release = latestRelease(JSON.parse(downloadText(releasesURL)));
  const images = release.assets.filter((a: any) => new RegExp(`^Apparent-[0-9.]+-linux-${arch}\\.AppImage$`).test(a.name));
  if (images.length !== 1) throw new Error('Latest Linux release has a missing or ambiguous architecture asset');
  const name = images[0].name;
  const prefix = `https://github.com/tryapparent/apparent/releases/download/${release.tag_name}/`;
  const sums = release.assets.filter((a: any) => a.name === name + '.sha256' && a.browser_download_url === prefix + a.name);
  if (sums.length !== 1) throw new Error('Latest Linux release is missing its checksum asset');
  const image = artifact({ schema: 1, source: release.tag_name.slice('linux-main-v2-'.length), release,
    checksums: { [name]: downloadText(sums[0].browser_download_url) } }, arch);
  const root = join(homedir(), '.local/share/apparent-cli');
  mkdirSync(root, { recursive: true, mode: 0o700 });
  const stage = mkdtempSync(join(root, '.install-'));
  const destination = join(root, `${image.source}-${arch}-${basename(stage).slice(9)}`);
  try {
    const archive = join(stage, image.name);
    run('curl', ['--fail', '--silent', '--show-error', '--location', '--proto', '=https', '--proto-redir', '=https', '--max-time', '300', '--max-filesize', '600000000', image.url, '--output', archive]);
    const hash = createHash('sha256');
    for await (const chunk of createReadStream(archive)) hash.update(chunk);
    if (hash.digest('hex') !== image.sha256) throw new Error('Checksum mismatch; downloaded code was not executed');
    run('chmod', ['+x', archive]);
    run(archive, ['--appimage-extract'], stage);
    const wrapper = '#!/bin/sh\nroot=$(dirname -- "$(readlink -f -- "$0")")\nexport LD_LIBRARY_PATH="$root/squashfs-root/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\nexec "$root/squashfs-root/usr/bin/apparent" "$@"\n';
    writeFileSync(join(stage, 'apparent'), wrapper, { mode: 0o700 });
    // Exercise the extracted binary, never start the desktop app.
    run(join(stage, 'apparent'), ['--version']);
    run(join(stage, 'apparent'), ['connect', 'login', '--help']);
    rmSync(archive);
    // Publish a fresh complete directory; an existing executable may still be in use.
    renameSync(stage, destination);
    const link = join(homedir(), '.local/bin/apparent');
    mkdirSync(dirname(link), { recursive: true });
    let old;
    try { old = lstatSync(link); } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
    if (old && (!old.isSymbolicLink() || !resolve(dirname(link), readlinkSync(link)).startsWith(root + '/'))) {
      console.log(`Installed ${image.source}. Existing command preserved; run ${join(destination, 'apparent')}`);
      return;
    }
    const temporary = join(stage + '-link');
    try {
      symlinkSync(join(destination, 'apparent'), temporary);
      renameSync(temporary, link);
    } finally { rmSync(temporary, { force: true }); }
    console.log(`Installed ${image.source}: ${link}\nRun: ${link} inspect <capture-id>`);
  } finally { rmSync(stage, { recursive: true, force: true }); }
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  install().catch(error => { console.error(`Apparent installation failed: ${error.message}`); process.exitCode = 1; });
}
