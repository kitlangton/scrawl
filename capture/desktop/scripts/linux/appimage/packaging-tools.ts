import { createHash } from 'node:crypto';
import { chmod, lstat, mkdir, mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { join, resolve as absolute } from 'node:path';
import { fileURLToPath } from 'node:url';

const pluginCommit = '432af639725bc8b99df12351e86bb2e8a6b3585f';
const plugin = {
  name: 'linuxdeploy-plugin-gstreamer.sh',
  url: `https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gstreamer/${pluginCommit}/linuxdeploy-plugin-gstreamer.sh`,
  sha256: 'd1cf90701047aa9583c274bb66dd82db352255374fbfb8f3c4c89a05fe23bce0',
};
type Tool = { name: string; url: string; sha256: string };
type Manifest = { schema: 1; arch: string; tools: Tool[] };
type Fetch = typeof fetch;
const digest = (bytes: Uint8Array | string) => createHash('sha256').update(bytes).digest('hex');
const architecture = (arch: string) => {
  if (arch === 'x64') return 'x86_64';
  if (arch === 'arm64') return 'aarch64';
  throw new Error(`Unsupported Linux packaging architecture: ${arch}`);
};

// Resolve the moving upstream release once, before cache lookup. The cache key
// and installer then use the same official digest, even if upstream moves later.
export async function resolveTools(arch = process.arch, request: Fetch = fetch, token = process.env.GH_TOKEN): Promise<Manifest> {
  const target = architecture(arch);
  const headers: Record<string, string> = { Accept: 'application/vnd.github+json' };
  if (token) headers.Authorization = `Bearer ${token}`;
  const response = await request('https://api.github.com/repos/linuxdeploy/linuxdeploy/releases/tags/continuous', {
    headers, signal: AbortSignal.timeout(30_000),
  });
  if (!response.ok) throw new Error(`Cannot resolve official linuxdeploy release: HTTP ${response.status}`);
  const release = await response.json() as { assets: { name: string; browser_download_url: string; digest: string }[] };
  const name = `linuxdeploy-${target}.AppImage`;
  const assets = release.assets.filter(asset => asset.name === name);
  if (assets.length !== 1) throw new Error(`Expected exactly one official ${name} asset`);
  const asset = assets[0];
  if (!/^sha256:[a-f0-9]{64}$/.test(asset.digest)) throw new Error(`Missing official SHA-256 for ${name}`);
  const manifest: Manifest = { schema: 1, arch: target, tools: [
    { name: 'linuxdeploy', url: asset.browser_download_url, sha256: asset.digest.slice(7) }, plugin,
  ] };
  validate(manifest, arch);
  return manifest;
}

function validate(manifest: Manifest, arch: string) {
  const target = architecture(arch);
  if (process.platform !== 'linux' || manifest.schema !== 1 || manifest.arch !== target || manifest.tools?.length !== 2) {
    throw new Error('Invalid packaging tool manifest or platform');
  }
  const [deploy, gst] = manifest.tools;
  if (deploy.name !== 'linuxdeploy' || deploy.url !== `https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${target}.AppImage`
      || !/^[a-f0-9]{64}$/.test(deploy.sha256)
      || gst.name !== plugin.name || gst.url !== plugin.url || gst.sha256 !== plugin.sha256) {
    throw new Error('Packaging tool manifest must identify official pinned assets');
  }
}

export function cacheKey(manifest: Manifest) {
  validate(manifest, manifest.arch === 'x86_64' ? 'x64' : 'arm64');
  return digest(JSON.stringify(manifest));
}

export async function installTools(manifest: Manifest, directory: string, request: Fetch = fetch, arch = process.arch) {
  validate(manifest, arch);
  for (const tool of manifest.tools) await installTool(tool, directory, request);
}

export async function installTool(tool: Tool, directory: string, request: Fetch = fetch) {
  await mkdir(directory, { recursive: true });
  const destination = join(directory, tool.name);
  try {
    const stat = await lstat(destination);
    if (!stat.isFile()) throw new Error(`Packaging cache entry is not a regular file: ${tool.name}`);
    if (digest(await readFile(destination)) === tool.sha256) {
      await chmod(destination, 0o755);
      console.log(`Verified cached packaging tool: ${tool.name}`);
      return;
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
  }
  const response = await request(tool.url, { signal: AbortSignal.timeout(120_000) });
  if (!response.ok) throw new Error(`Cannot download ${tool.name}: HTTP ${response.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (digest(bytes) !== tool.sha256) throw new Error(`SHA-256 mismatch for ${tool.name}`);
  const temporary = await mkdtemp(join(directory, '.download-'));
  try {
    const staged = join(temporary, tool.name);
    await writeFile(staged, bytes, { mode: 0o755, flag: 'wx' });
    await rename(staged, destination);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
  console.log(`Downloaded and verified packaging tool: ${tool.name}`);
}

async function main() {
  const [command, manifestPath, destination] = process.argv.slice(2);
  if (!manifestPath || !['resolve', 'install'].includes(command) || (command === 'install' && !destination)) {
    throw new Error('Usage: packaging-tools.ts resolve <manifest> [github-output] | install <manifest> <tools-directory>');
  }
  if (command === 'resolve') {
    const manifest = await resolveTools();
    await writeFile(manifestPath, JSON.stringify(manifest) + '\n');
    if (destination) {
      await writeFile(destination, `key=${cacheKey(manifest)}\n`, { flag: 'a' });
    }
  } else {
    await installTools(JSON.parse(await readFile(manifestPath, 'utf8')), destination);
  }
}
if (process.argv[1] && absolute(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
