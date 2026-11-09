// One reviewed command closure. Expansion requires a new closure audit and proof.
import {execFileSync, spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync, lstatSync, readFileSync, readdirSync, readlinkSync, realpathSync} from 'node:fs';
import {dirname, isAbsolute, join, relative, resolve} from 'node:path';
import {arch, release} from 'node:os';
import {digest, reuse} from './receipt.ts';
import type {Snapshot} from './receipt.ts';
import type {Command} from './run.ts';

export type Admission = {base: string; head: string};
// npm installs both optional libc variants on Linux. Every variant's bytes are
// bound, but only glibc ELF loaders can execute in this qualified host/runtime.
const inactiveMuslPackages = ['@img/sharp-libvips-linuxmusl-x64', '@img/sharp-linuxmusl-x64',
  '@parcel/watcher-linux-x64-musl', '@tailwindcss/oxide-linux-x64-musl', 'lightningcss-linux-x64-musl'];
export const glibcNative = (path: string) => !inactiveMuslPackages.some(name => path.includes(`/node_modules/${name}/`));
export function loaderPaths(output: string): string[] {
  return output.split('\n').flatMap(line => {
    const match = line.match(/(?:=>\s+|^\s*)(\/.+?)\s+\(0x[a-f0-9]+\)\s*$/i);
    if (match) return [match[1]];
    if (!line.trim() || /^\s*(linux-vdso\.so\.\d+ \(0x[a-f0-9]+\)|statically linked|not a dynamic executable)\s*$/i.test(line)) return [];
    throw Error('unrecognized or unresolved runtime loader input');
  });
}
const qualifiedPaths = ['connect/service', 'shared/ui', 'shared/assets'];
// 49d3414b2: complete service tests/helpers/scripts/lock and shared source/assets.
// Test fixtures use in-memory Miniflare and synthetic loopback services. No Git
// ancestry, production credentials, browser, remote service or native app proof.
const qualifiedTree = '88ca1a14744a5a67a2c398f2838e7facb8f35a20cabc46746f5059eb2aa6c045';
const canonicalTest = 'npm run build && node --test --test-concurrency=1 test/*/*.test.mjs';

function git(cwd: string, env: NodeJS.ProcessEnv, args: string[]): string {
  return execFileSync('git', args, {cwd, env, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024});
}

// Content/mode/link identity permits an identical npm ci reinstall. The separate
// stat identity detects replacement and mutate/restore races during one command.
export function files(paths: string[], allowedRoots: string[], committed = new Map<string, {oid: string; executable: boolean}>()) {
  const content = createHash('sha256'), stability = createHash('sha256');
  const visited = new Set<string>(), native = new Set<string>();
  const inside = (path: string) => allowedRoots.some(root => path === root || (!relative(root, path).startsWith('..') && !isAbsolute(relative(root, path))));
  function walk(path: string) {
    if (visited.has(path)) return;
    visited.add(path);
    const before = lstatSync(path, {bigint: true});
    const expected = committed.get(path);
    if (expected && (!before.isFile() || Boolean(Number(before.mode) & 0o100) !== expected.executable)) throw Error('source type or mode differs from HEAD');
    const stamp = () => [before.dev, before.ino, before.mode, before.size, before.mtimeNs, before.ctimeNs].map(String);
    stability.update(JSON.stringify([path, stamp()]));
    content.update(JSON.stringify([path, String(before.mode)]));
    if (before.isSymbolicLink()) {
      content.update(readlinkSync(path));
      const target = realpathSync(path);
      if (!inside(target)) throw Error('unqualified external symlink');
      walk(target);
    } else if (before.isDirectory()) {
      for (const name of readdirSync(path).sort()) walk(join(path, name));
    } else if (before.isFile()) {
      const bytes = readFileSync(path);
      if (expected && createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex') !== expected.oid) throw Error('source bytes differ from HEAD');
      content.update(digest(bytes));
      if (bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) native.add(path);
    } else throw Error('unqualified special file');
    const after = lstatSync(path, {bigint: true});
    if (before.ino !== after.ino || before.dev !== after.dev || before.ctimeNs !== after.ctimeNs || before.mtimeNs !== after.mtimeNs) throw Error('input changed during hashing');
  }
  for (const path of [...paths].sort()) walk(path);
  return {key: content.digest('hex'), stable: stability.digest('hex'), native: [...native].sort()};
}

function admitted(cwd: string, env: NodeJS.ProcessEnv, admission: Admission) {
  const read = (...args: string[]) => git(cwd, env, args).trim();
  if (read('rev-parse', 'HEAD') !== admission.head || read('status', '--porcelain', '--untracked-files=all')
    || /^[a-zS] /m.test(read('ls-files', '-v'))) throw Error('unclean or changed candidate');
  const main = read('rev-parse', 'refs/remotes/origin/main');
  if (read('merge-base', admission.head, main) !== admission.base || admission.base !== main) throw Error('unintegrated base');
  if (digest(git(cwd, env, ['ls-tree', '-r', '-z', 'HEAD', '--', ...qualifiedPaths])) !== qualifiedTree) throw Error('unreviewed service closure');
  return main;
}

export function npmConfiguration(service: string, env: NodeJS.ProcessEnv) {
  const invoke = (args: string[]) => execFileSync('npm', args, {cwd: service, env,
    encoding: 'utf8', maxBuffer: 4 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe']}).trim();
  const config = JSON.parse(invoke(['config', 'list', '--json']));
  if (!config || typeof config !== 'object' || Array.isArray(config)
    || config['node-options'] !== null || config['script-shell'] !== null
    || config['ignore-scripts'] !== false || config.global !== false || config['dry-run'] !== false
    || config.workspaces !== null || !Array.isArray(config.workspace) || config.workspace.length
    || config['include-workspace-root'] !== false) throw Error('unsupported npm lifecycle configuration');
  const prefix = invoke(['prefix']);
  if (realpathSync(prefix) !== realpathSync(service)) throw Error('npm uses a different package');
  const paths = [config.userconfig, config.globalconfig, join(prefix, '.npmrc')];
  if (paths.some(path => typeof path !== 'string' || !isAbsolute(path))) throw Error('unknown npm config path');
  const presence = [...new Set<string>(paths)].sort().map(path => [path, existsSync(path)] as const);
  const input = files(presence.filter(([, exists]) => exists).map(([path]) => path), ['/']);
  // Bind effective values as well as the files npm read. Values may be private:
  // only their digest enters a receipt, never configuration contents.
  return {key: digest(JSON.stringify([Object.entries(config).sort(([a], [b]) => a.localeCompare(b)), presence, input.key])),
    stable: input.stable};
}

export function serviceSnapshot(cwd: string, env: NodeJS.ProcessEnv, admission: Admission, argv: string[]): Snapshot {
  const main = admitted(cwd, env, admission);
  const tree = git(cwd, env, ['ls-tree', '-r', '-z', 'HEAD']);
  const committed = new Map<string, {oid: string; executable: boolean}>();
  const tracked = tree.split('\0').filter(Boolean).map(entry => {
    const match = /^(100644|100755) blob ([a-f0-9]{40})\t/.exec(entry);
    if (!match) throw Error('unqualified source type');
    const path = resolve(cwd, entry.slice(entry.indexOf('\t') + 1));
    committed.set(path, {oid: match[2], executable: match[1] === '100755'});
    return path;
  });
  const service = join(cwd, 'connect/service');
  const npm = npmConfiguration(service, env);
  // Hash ignored package inputs as well as every tracked source. Unknown extra
  // state in this package is included; escaping links and special files fail closed.
  const source = files([...tracked, service], [cwd], committed);
  const node = realpathSync(process.execPath);
  const runtimePaths = new Set([node, '/usr/bin/ldd', '/etc/ld.so.cache', '/etc/os-release',
    '/etc/hosts', '/etc/resolv.conf', '/etc/nsswitch.conf', '/etc/localtime', '/proc/sys/kernel/random/boot_id',
    '/usr/lib/locale', '/usr/share/zoneinfo']);
  // Bind the actual runtime and its transitive loader closure, including workerd,
  // esbuild and native npm modules. ldd's output already includes indirect libs.
  for (const executable of [node, ...source.native.filter(glibcNative)]) {
    const result = spawnSync('/usr/bin/ldd', [executable], {env, encoding: 'utf8'});
    const output = result.stdout + result.stderr;
    if (result.error || (result.status !== 0 && !/not a dynamic executable|statically linked/.test(output))) throw Error('unknown runtime linkage');
    for (const path of loaderPaths(output)) runtimePaths.add(realpathSync(path));
  }
  const runtime = files([...runtimePaths].map(path => realpathSync(path)), ['/']);
  const endMain = admitted(cwd, env, admission);
  if (main !== endMain) throw Error('base changed during fingerprint');
  const identity = {tree, source: source.key, runtime: runtime.key, npm: npm.key, argv, cwd,
    environment: Object.entries(env).sort(([a], [b]) => a.localeCompare(b)),
    versions: process.versions, platform: process.platform, arch: arch(), kernel: release(), umask: process.umask()};
  return {key: digest(JSON.stringify(identity)), stable: digest(JSON.stringify([source.stable, runtime.stable, npm.stable, main])), head: admission.head};
}

export function serviceTests(command: Command, cwd: string, env: NodeJS.ProcessEnv, admission: Admission | undefined,
  run: (command: Command, environment?: NodeJS.ProcessEnv) => void): boolean {
  if (command.cwd !== 'connect/service' || JSON.stringify(command.argv) !== JSON.stringify(['npm', 'test'])) return false;
  try {
    if (!admission || process.platform !== 'linux' || process.arch !== 'x64' || env.CI || env.GITHUB_ACTIONS
      || !(process.report.getReport() as {header: {glibcVersionRuntime?: string}}).header.glibcVersionRuntime) throw Error('local Linux x64 glibc only');
    // Loader/path overrides can add unbounded inputs. Their normal execution path
    // remains available; no receipt can qualify them using an environment hash alone.
    if (Object.keys(env).some(name => /^(NODE_|LD_|DYLD_|MINIFLARE_|WRANGLER_|WORKERD_|ESBUILD_|OPENSSL_|SSL_CERT_|CLOUDFLARE_|VSCODE_INSPECTOR_OPTIONS|LOCPATH$|GCONV_PATH$|ICU_DATA$|TZDIR$)/.test(name))
      || Object.keys(env).some(name => /^(http_proxy|https_proxy|all_proxy|no_proxy)$/i.test(name))
      || env.TZ?.startsWith(':') || env.TZ?.startsWith('/')) throw Error('runtime override');
    if (realpathSync(execFileSync('/bin/sh', ['-c', 'command -v node'], {env, encoding: 'utf8'}).trim()) !== realpathSync(process.execPath)) throw Error('different Node');
    const pkg = JSON.parse(readFileSync(join(cwd, 'connect/service/package.json'), 'utf8'));
    if (pkg.scripts.test !== canonicalTest) throw Error('unqualified command');
    // Require local locked installs; ancestor module directories permit ambient
    // dependency fallback beyond the audited installation.
    for (let parent = dirname(join(cwd, 'connect/service')); ; parent = dirname(parent)) {
      if (existsSync(join(parent, 'node_modules'))) throw Error('ambient dependencies');
      if (parent === dirname(parent)) break;
    }
    if (!existsSync(join(cwd, 'connect/service/node_modules/.package-lock.json'))) throw Error('missing npm installation');
    npmConfiguration(join(cwd, command.cwd), env);
    admitted(cwd, env, admission);
  } catch {
    console.error('local-reuse: fresh service npm test (outside qualified closure)');
    return false;
  }
  // Preserve the canonical build and generate every output required by subsequent
  // recipient checks even on a hit. Only the serial Node test phase is reusable.
  run({argv: ['npm', 'run', 'build'], cwd: command.cwd, env: command.env});
  const testRoot = join(cwd, command.cwd, 'test');
  const tests = readdirSync(testRoot).sort().flatMap(folder => {
    const path = join(testRoot, folder);
    return lstatSync(path).isDirectory() ? readdirSync(path).sort().filter(name => name.endsWith('.test.mjs')).map(name => `test/${folder}/${name}`) : [];
  });
  if (!tests.length) throw Error('service test inventory empty');
  const testCommand = {argv: [process.execPath, '--no-global-search-paths', '--test', '--test-concurrency=1', ...tests], cwd: command.cwd};
  const testEnv = serviceEnvironment(env);
  reuse(cwd, () => serviceSnapshot(cwd, testEnv, admission!, testCommand.argv), () => {
    // Config can change while the fresh build runs. In that case use npm's
    // canonical lifecycle so a later loader/shell override is not bypassed.
    try { npmConfiguration(join(cwd, command.cwd), env); }
    catch { run(command); return; }
    run(testCommand, testEnv);
  });
  return true;
}

export function serviceEnvironment(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  const result = {...env};
  // Transport/lifecycle IDs are neither inputs nor supplied to the qualified
  // subprocess. All remaining environment is bound, without recording its values.
  for (const name of Object.keys(result)) {
    if (/^(CODEX_|T3CODE_|INVOCATION_ID$|JOURNAL_STREAM$|SYSTEMD_EXEC_PID$|MANAGERPID$|SHLVL$|_$|npm_lifecycle_|npm_package_|npm_command$)/.test(name)) delete result[name];
  }
  // Miniflare otherwise fetches live geolocation metadata and caches it inside
  // node_modules. The audited suite uses synthetic requests and no Request.cf.
  result.CLOUDFLARE_CF_FETCH_ENABLED = 'false';
  return result;
}
