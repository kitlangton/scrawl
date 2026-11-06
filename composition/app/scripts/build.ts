import { build } from 'esbuild';
import { chmod, mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';
import { APP_SHELL_API, REACT_VERSION } from '@apparent/app-contracts';
import { digest, readRegular, shellFiles } from './artifacts.ts';
import { checkInstalled } from './check-installed.ts';
import { stageRecordNative, type RecordNativeArtifacts } from './record-native.ts';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const external = ['react', 'react/jsx-runtime', 'react-dom/client'];
const common = { bundle: true, format: 'esm' as const, platform: 'browser' as const,
  target: 'es2022', jsx: 'automatic' as const, nodePaths: [join(root, 'node_modules')],
  define: { 'process.env.NODE_ENV': '"production"' }, legalComments: 'inline' as const };

export interface OnboardingMainBindingInput {
  /** Trusted, compiled composition module. It imports only published owner exports. */
  module: string;
  /** Exact staged helper admitted by the producer receipt/attestation. */
  configHost: { path: string; sha256: string };
  /** `dist/assets` from the same published Onboarding package. */
  ownerAssets: string;
  /** Trusted package roots supplied by the assembly process for module resolution. */
  resolvePaths?: string[];
}
export interface RecordMainBindingInput {
  /** Trusted composition module exporting createRecordHostServices({ profilePath }). */
  module: string;
  resolvePaths?: string[];
  /** Complete externally admitted native payloads, staged without building owner source. */
  nativeArtifacts?: RecordNativeArtifacts;
}
export async function buildShell(output: string, onboarding?: OnboardingMainBindingInput, record?: RecordMainBindingInput) {
  if (onboarding && record) throw new Error('Owner host bindings must be assembled separately');
  if (record?.nativeArtifacts && (process.platform !== 'linux' || process.arch !== 'x64')) {
    throw new Error('Record native Capture/2 assembly requires Linux x64');
  }
  await checkInstalled();
  if (onboarding && !/^[a-f0-9]{64}$/.test(onboarding.configHost.sha256)) throw new Error('Invalid admitted config host digest');
  await mkdir(join(output, 'vendor'), { recursive: true });
  const nativeFiles = record?.nativeArtifacts ? await stageRecordNative(output, record.nativeArtifacts) : undefined;
  await build({ ...common, entryPoints: [join(root, 'src/renderer.tsx')], outfile: join(output, 'renderer.js'),
    external: [...external, './registry.js'] });
  const require = createRequire(import.meta.url);
  const reactExports = Object.keys(require('react')).filter(name => /^[A-Za-z_$][\w$]*$/.test(name));
  for (const [file, contents, dependencies] of [
    ['react', `import React from "react"; export default React; export const {${reactExports.join(',')}} = React;`, []],
    ['jsx', `import runtime from ${JSON.stringify(require.resolve('react/jsx-runtime'))}; export const {jsx,jsxs,Fragment} = runtime;`, ['react']],
    ['dom', 'export { createRoot } from "react-dom/client";', ['react']],
  ] as const) {
    await build({ ...common, stdin: { contents, resolveDir: root },
      outfile: join(output, 'vendor', file + '.js'), external: [...dependencies],
      ...(file === 'dom' ? { banner: { js: 'import * as sharedReact from "react"; const require = (name) => { if (name === "react") return sharedReact; throw new Error("Unsupported runtime dependency"); };' } } : {}) });
  }
  await build({ entryPoints: [join(root, 'src/main.cts')], bundle: true, platform: 'node',
    format: 'cjs', target: 'node22', external: ['electron'], outfile: join(output, 'main.cjs') });
  await build({ entryPoints: [join(root, 'src/preload.cts')], bundle: true, platform: 'node',
    format: 'cjs', target: 'node22', external: ['electron'], outfile: join(output, 'preload.cjs') });
  if (record) {
    await build({ entryPoints: [resolve(record.module)], bundle: true, platform: 'node',
      format: 'esm', target: 'node22', external: ['electron'],
      nodePaths: [join(root, 'node_modules'), ...(record.resolvePaths ?? []).map(path => resolve(path))],
      outfile: join(output, 'owner-binding.mjs') });
  } else if (onboarding) {
    const helper = await readRegular(onboarding.configHost.path);
    if (digest(helper) !== onboarding.configHost.sha256) throw new Error('Config host artifact digest mismatch');
    await mkdir(join(output, 'native'), { recursive: true });
    const target = join(output, 'native/apparent-config-host');
    await writeFile(target, helper, { mode: 0o755 });
    await chmod(target, 0o755);
    await build({ entryPoints: [resolve(onboarding.module)], bundle: true, platform: 'node',
      format: 'esm', target: 'node22', external: ['electron'],
      nodePaths: [join(root, 'node_modules'), ...(onboarding.resolvePaths ?? []).map(path => resolve(path))],
      define: { ONBOARDING_CONFIG_HOST_SHA256: JSON.stringify(onboarding.configHost.sha256) },
      outfile: join(output, 'owner-binding.mjs') });
    for (const name of ['frog.png', 'apparent-app-icon.svg']) {
      const asset = await readRegular(join(onboarding.ownerAssets, name));
      await mkdir(join(output, 'assets'), { recursive: true });
      await writeFile(join(output, 'assets', name), asset);
    }
  } else {
    await writeFile(join(output, 'owner-binding.mjs'), 'export {};\n');
  }
  let html = await readFile(join(root, 'src/index.html'), 'utf8');
  const importmap = html.match(/<script type="importmap">([^<]+)<\/script>/)![1];
  // CSP permits only this exact inline import map; no unsafe-inline or eval.
  const { createHash } = await import('node:crypto');
  html = html.replace('IMPORT_MAP_HASH', createHash('sha256').update(importmap).digest('base64'));
  await writeFile(join(output, 'index.html'), html);
  await writeFile(join(output, 'style.css'),
    await readFile(join(root, '../../shared/ui/src/tokens.css'), 'utf8') + '\n' +
    await readFile(join(root, 'src/style.css'), 'utf8'));
  await writeFile(join(output, 'package.json'), JSON.stringify({ name: 'apparent-foundation',
    productName: 'Apparent Foundation', version: '0.0.0', main: 'main.cjs' }, null, 2) + '\n');
  const files: Record<string, string> = {};
  const inventory = [...shellFiles, ...Object.keys(nativeFiles ?? {}), ...(onboarding ? ['native/apparent-config-host', 'assets/frog.png', 'assets/apparent-app-icon.svg'] : [])];
  for (const file of inventory) files[file] = digest(await readRegular(join(output, file)));
  await writeFile(join(output, 'manifest.json'), JSON.stringify({ schemaVersion: 1,
    shellApi: APP_SHELL_API, reactVersion: REACT_VERSION, files, ...(nativeFiles ? { nativeFiles } : {}) }, null, 2) + '\n');
}
export async function buildApp(output: string, onboarding?: OnboardingMainBindingInput) {
  await buildShell(join(output, 'shell'), onboarding);
  // Preserve independently built stream artifacts for explicit assembly.
  await mkdir(join(output, 'artifacts'), { recursive: true });
}
if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const target = process.argv[2];
  if (target === 'shell') await buildShell(resolve(process.argv[3] ?? join(root, 'dist/shell')));
  else if (target === 'app') await buildApp(resolve(process.argv[3] ?? join(root, 'dist')));
  else throw new Error('Usage: node scripts/build.ts shell|app [output-directory]');
}
