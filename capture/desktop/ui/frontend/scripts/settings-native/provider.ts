import {execFileSync, spawn} from 'node:child_process';
import {chmod, copyFile, lstat, mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile} from 'node:fs/promises';
import {release} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createRequire} from 'node:module';
import {contract, digest, nativeInputs, testName} from './inputs.ts';

export const frontend = fileURLToPath(new URL('../..', import.meta.url));
export const root = path.resolve(frontend, '../../../..');
export type FileHash = {name: string; sha256: string};
type Receipt = {schema: 1; contract: string; root: string; platform: string; arch: string; release: string;
  qualified: true; qualifier: string; sourceCommit: string; rustc: string; inputs: {hash: string; count: number}; files: FileHash[];
  systemLibraries: FileHash[]; timeoutSha256: string; lddSha256: string};
const runtimeLibrary = /^lib(?:transcribe|ggml(?:-[a-z0-9]+)*)\.so(?:\.\d+)*$/;
const environment = () => {
  const env = {...process.env}; for (const key of Object.keys(env)) if (key.startsWith('LD_')) delete env[key]; return env;
};
export function run(command: string, args: string[], cwd = root, env = environment()) {
  return new Promise<void>((resolve, reject) => {
    const child = spawn(command, args, {cwd, env, stdio: 'inherit'});
    child.once('error', reject);
    child.once('close', (code, signal) => code === 0 ? resolve() : reject(Error(`${command} failed (${signal ?? code})`)));
  });
}
function linux() { if (process.platform !== 'linux') throw Error('Prepared Storage provider execution is qualified on Linux only; existing Mac integration/native gates remain required'); }
async function hashFile(file: string) { return digest(await readFile(file)); }
export function probe(command: string, args: string[], seconds = 5) {
  return execFileSync('/usr/bin/timeout', ['--signal=KILL', `${seconds}s`, command, ...args],
    {cwd: root, env: environment(), encoding: 'utf8', maxBuffer: 32 * 1024 * 1024});
}
async function qualifierHash() {
  const entries: Array<[string, string]> = [];
  async function visit(directory: string, relative = '') {
    for (const name of (await readdir(directory)).sort()) {
      const file = path.join(directory, name), key = path.posix.join(relative, name);
      const info = await lstat(file);
      if (info.isSymbolicLink()) throw Error('Qualifier inputs must not be symbolic links');
      if (info.isDirectory()) await visit(file, key);
      else if (info.isFile()) entries.push([key, await hashFile(file)]);
      else throw Error('Unsupported qualifier input');
    }
  }
  await visit(fileURLToPath(new URL('.', import.meta.url)));
  return digest(JSON.stringify(entries));
}
async function systemLibraries(native: string, files: FileHash[]): Promise<FileHash[]> {
  const paths = new Set<string>();
  for (const file of files) {
    const linked = probe('/usr/bin/ldd', [path.join(native, file.name)]);
    if (linked.includes('not found')) throw Error('Prepared provider has unresolved runtime libraries');
    for (const link of linked.match(/\/[^\s()]+/g) ?? []) {
      const resolved = await realpath(link);
      if (resolved.startsWith(native + '/')) continue;
      if (!/^\/(?:usr\/)?lib(?:64)?\//.test(resolved)) throw Error('Provider still depends on a build/cache path');
      paths.add(resolved);
    }
  }
  if (!paths.size) throw Error('No provider system-library closure discovered');
  const result: FileHash[] = [];
  for (const name of [...paths].sort()) result.push({name, sha256: await hashFile(name)});
  return result;
}
export async function verifyInventory(native: string, files: FileHash[]) {
  if (!Array.isArray(files) || !files.some(file => file.name === 'backend') || files.length > 64) throw Error('Invalid provider inventory');
  if (!(await lstat(native)).isDirectory() || (await lstat(native)).isSymbolicLink()) throw Error('Unsafe provider native directory');
  if (JSON.stringify((await readdir(native)).sort()) !== JSON.stringify(files.map(file => file.name).sort())) throw Error('Provider inventory changed');
  for (const file of files) {
    if (file.name !== 'backend' && !runtimeLibrary.test(file.name)) throw Error('Unexpected provider file');
    const target = path.join(native, file.name);
    const info = await lstat(target);
    if (!info.isFile() || info.isSymbolicLink() || (info.mode & 0o222) || await hashFile(target) !== file.sha256) throw Error('Prepared provider file changed');
  }
}
async function privateDirectory(directory: string) {
  const info = await lstat(directory);
  if (!info.isDirectory() || info.isSymbolicLink() || (info.mode & 0o077)) throw Error('Provider directory must be a private regular directory');
}
async function validate(directory: string, receipt: Receipt) {
  linux();
  await privateDirectory(directory);
  if (receipt.schema !== 1 || receipt.qualified !== true || receipt.qualifier !== await qualifierHash() || receipt.contract !== contract || receipt.platform !== process.platform ||
      receipt.arch !== process.arch || receipt.release !== release() || receipt.root !== await realpath(root)) throw Error('Incompatible provider receipt; prepare this checkout/host explicitly');
  const inputs = await nativeInputs(root);
  if (JSON.stringify(inputs) !== JSON.stringify(receipt.inputs)) throw Error('Native input closure changed; prepare a new provider and retain affected native gates');
  if (!Array.isArray(receipt.systemLibraries) || receipt.systemLibraries.length > 256) throw Error('Invalid provider inventory');
  await verifyInventory(path.join(directory, 'native'), receipt.files);
  if (await hashFile('/usr/bin/timeout') !== receipt.timeoutSha256 || await hashFile('/usr/bin/ldd') !== receipt.lddSha256) throw Error('Provider probe tool changed');
  if (JSON.stringify(await systemLibraries(path.join(directory, 'native'), receipt.files)) !== JSON.stringify(receipt.systemLibraries)) throw Error('Provider system-library closure changed');
  const listed = probe(path.join(directory, 'native/backend'), [testName, '--ignored', '--exact', '--list']).split('\n').filter(line => line.endsWith(': test'));
  if (JSON.stringify(listed) !== JSON.stringify([testName + ': test'])) throw Error('Prepared backend does not expose exactly the required ignored provider test');
}
export async function verify(directory: string): Promise<Receipt> {
  linux();
  await privateDirectory(directory);
  const file = path.join(directory, 'receipt.json');
  const receiptInfo = await lstat(file);
  if (!receiptInfo.isFile() || receiptInfo.isSymbolicLink() || (receiptInfo.mode & 0o077) || receiptInfo.size > 1024 * 1024) throw Error('Provider receipt must be a bounded private file');
  const receipt = JSON.parse(await readFile(file, 'utf8')) as Receipt;
  await validate(directory, receipt);
  return receipt;
}

async function runBackend(directory: string) {
  const electron = createRequire(path.join(frontend, 'package.json'))('electron') as string;
  // A short private root avoids Linux/macOS Unix socket pathname limits. No GUI is launched.
  const temporary = await mkdtemp('/tmp/apparent-dev-sp-');
  await chmod(temporary, 0o700);
  try {
    // GNU timeout retains its own process-group leader. A deadline kills the
    // whole owned group in one signal; never retry a reaped PID/PGID ourselves.
    await run('/usr/bin/timeout', ['--signal=KILL', '60s', path.join(directory, 'native/backend'), testName, '--exact', '--ignored', '--nocapture', '--test-threads=1'], root,
      {...environment(), TMPDIR: temporary, APPARENT_STORAGE_TEST_ELECTRON: electron});
    await rm(temporary, {recursive: true});
  } catch (error) { console.error(`Failed provider run temporary root retained: ${temporary}`); throw error; }
}
export async function execute(directory: string) {
  await verify(directory);
  await runBackend(directory);
  await verify(directory);
}

export async function prepare() {
  linux();
  const qualifier = await qualifierHash();
  await run('node', ['--experimental-strip-types', 'scripts/build-storage-policy.ts'], frontend);
  if (!probe('/usr/bin/timeout', ['--version']).startsWith('timeout (GNU coreutils)')) throw Error('The local provider requires GNU timeout');
  const timeoutSha256 = await hashFile('/usr/bin/timeout'), lddSha256 = await hashFile('/usr/bin/ldd');
  const inputs = await nativeInputs(root);
  const output = probe('cargo', ['test', '--offline', '-j', process.env.T3_BUILD_JOBS || '3', '--bin', 'apparent', '--no-run', '--message-format=json'], 900);
  const artifacts = output.split('\n').filter(Boolean).map(line => JSON.parse(line)).filter(message =>
    message.reason === 'compiler-artifact' && message.profile.test && message.target.name === 'apparent' && message.executable);
  if (artifacts.length !== 1) throw Error('Cargo did not identify one root test backend');
  const backend: string = artifacts[0].executable;
  const dependencies = path.dirname(backend);
  // Refuse to qualify a native build that embeds the supposedly independent TS surface.
  for (const file of (await readdir(dependencies)).filter(file => file.endsWith('.d'))) {
    const text = await readFile(path.join(dependencies, file), 'utf8');
    const words = text.replaceAll('\\\n', ' ').match(/(?:\\.|[^\s])+/g) ?? [];
    for (const word of words) {
      if (word.endsWith(':')) continue;
      const relative = path.relative(root, path.resolve(root, word.replace(/\\(.)/g, '$1'))).split(path.sep).join('/');
      if (relative.startsWith('capture/desktop/ui/frontend/') || relative.startsWith('record/storage/')) throw Error('Native dep-info embeds the consumer surface; audit the closure before preparing');
    }
  }
  const directory = await mkdtemp('/tmp/apparent-dev-sn-');
  await chmod(directory, 0o700);
  const native = path.join(directory, 'native'); await mkdir(native, {mode: 0o700});
  const files: FileHash[] = [];
  for (const [source, name] of [[backend, 'backend'], ...(await readdir(dependencies)).filter(file => runtimeLibrary.test(file)).map(file => [path.join(dependencies, file), file])]) {
    const target = path.join(native, name);
    await copyFile(source, target); await chmod(target, name === 'backend' ? 0o500 : 0o400);
    files.push({name, sha256: await hashFile(target)});
  }
  const linked = await systemLibraries(native, files);
  const after = await nativeInputs(root);
  if (JSON.stringify(inputs) !== JSON.stringify(after)) throw Error('Native inputs changed during preparation');
  const receipt: Receipt = {schema: 1, contract, root: await realpath(root), platform: process.platform, arch: process.arch,
    qualified: true, qualifier,
    release: release(), sourceCommit: execFileSync('git', ['rev-parse', 'HEAD'], {cwd: root, encoding: 'utf8'}).trim(),
    rustc: probe('rustc', ['-Vv']).trim(), inputs, files, systemLibraries: linked,
    timeoutSha256, lddSha256};
  await validate(directory, receipt);
  await runBackend(directory);
  await validate(directory, receipt);
  // Publish success only after the genuine copied provider has passed.
  await writeFile(path.join(directory, 'receipt.json'), JSON.stringify(receipt, null, 2) + '\n', {mode: 0o600, flag: 'wx'});
  console.log(`Qualified Storage provider: ${directory}`);
  return directory;
}
