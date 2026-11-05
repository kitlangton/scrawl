import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';
import { APP_SHELL_API, REACT_VERSION, parseStreamManifest } from '@apparent/app-contracts';
import { digest, readRegular, shellFiles } from './artifacts.ts';
import { checkInstalled } from './check-installed.ts';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const external = ['react', 'react/jsx-runtime', 'react-dom/client'];
const common = { bundle: true, format: 'esm' as const, platform: 'browser' as const,
  target: 'es2022', jsx: 'automatic' as const, nodePaths: [join(root, 'node_modules')],
  define: { 'process.env.NODE_ENV': '"production"' }, legalComments: 'inline' as const };

/** Reference producer for synthetic fixtures. Each invocation builds exactly one stream. */
export async function buildStream(source: string, output: string) {
  await checkInstalled();
  const metadata = JSON.parse(await readFile(join(source, 'stream.json'), 'utf8'));
  await mkdir(output, { recursive: true });
  const result = await build({ ...common, entryPoints: [join(source, 'index.tsx')],
    outfile: join(output, 'index.js'), external, metafile: true });
  for (const file of Object.values(result.metafile!.outputs)) {
    assert(file.imports.every(item => external.includes(item.path)), 'Unbundled stream dependency');
  }
  const manifest = parseStreamManifest({ ...metadata, schemaVersion: 1, shellApi: APP_SHELL_API,
    reactVersion: REACT_VERSION, entry: 'index.js', sha256: digest(await readRegular(join(output, 'index.js'))) });
  await writeFile(join(output, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
  return manifest;
}
export async function buildShell(output: string) {
  await checkInstalled();
  await mkdir(join(output, 'vendor'), { recursive: true });
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
  for (const file of shellFiles) files[file] = digest(await readRegular(join(output, file)));
  await writeFile(join(output, 'manifest.json'), JSON.stringify({ schemaVersion: 1,
    shellApi: APP_SHELL_API, reactVersion: REACT_VERSION, files }, null, 2) + '\n');
}
if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const target = process.argv[2];
  if (target === 'shell') await buildShell(resolve(process.argv[3] ?? join(root, 'dist/shell')));
  else if (target === 'alpha' || target === 'beta') {
    await buildStream(join(root, 'fixtures', target), resolve(process.argv[3] ?? join(root, 'dist/artifacts', target)));
  } else throw new Error('Usage: node scripts/build.ts shell|alpha|beta [output-directory]');
}
