import assert from 'node:assert/strict';
import { build, type BuildOptions } from 'esbuild';
import { createRequire } from 'node:module';
import { lstat, mkdir, mkdtemp, readFile, rename, rm, rmdir, writeFile } from 'node:fs/promises';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { APP_SHELL_API, REACT_VERSION, parseStreamManifest } from '@apparent/app-contracts';
import { digest, readStream } from './artifacts.ts';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const entry = join(root, 'src/record/pill.tsx');
const external = ['react', 'react/jsx-runtime'];

/** Builds a fresh, sealed Record stream from the installed public package only. */
export async function buildRecordArtifact(destination: string) {
  const output = resolve(destination);
  try {
    await lstat(output);
    throw new Error('Record artifact destination already exists');
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
  }
  const parent = dirname(output);
  await mkdir(parent, { recursive: true });
  const work = await mkdtemp(join(parent, '.record-stream-build-'));
  const stage = join(work, 'artifact');
  await mkdir(stage);
  let reservation: { dev: number; ino: number } | undefined;
  try {
    const recordPackage = JSON.parse(await readFile(join(root, 'node_modules/@apparent/record/package.json'), 'utf8')) as { name?: string; version?: string };
    assert.equal(recordPackage.name, '@apparent/record', 'The pinned public Record package is not installed');
    assert.match(recordPackage.version ?? '', /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/);

    const common: BuildOptions = {
      outdir: stage, assetNames: 'fonts/[name]-[hash]',
      bundle: true, write: false, metafile: true, platform: 'browser', format: 'esm', target: 'es2022',
      jsx: 'automatic', external, nodePaths: [join(root, 'node_modules')],
      loader: { '.ttf': 'file', '.woff2': 'file' },
      define: { 'process.env.NODE_ENV': '"production"' },
      // Match the shell's shared React bridge: bundled CommonJS UI dependencies
      // may require React, but the renderer gets no Node require or other module.
      banner: { js: 'import * as sharedReact from "react"; const require = (name) => { if (name === "react") return sharedReact; throw new Error("Unsupported runtime dependency"); };' },
    };
    const compiled = await build({ ...common, entryPoints: [entry], entryNames: 'index' });
    // Bundled package metadata may mark CSS as side-effect-free, so consume the
    // exact public stylesheet export as its own compiled artifact entry.
    const require = createRequire(import.meta.url);
    const compiledCss = await build({ ...common,
      entryPoints: [require.resolve('@apparent/record/ui/style.css')], entryNames: 'style',
    });
    assert(Object.keys(compiledCss.metafile!.inputs).some(name => name.includes('@apparent/record/') && name.includes('/dist/ui/style.css')),
      'Build did not consume the compiled public Record stylesheet');

    const jsOutput = Object.entries(compiled.metafile!.outputs).find(([name, value]) => name.endsWith('/index.js') && value.entryPoint);
    assert(jsOutput, 'Record entry bundle was not emitted');
    const externalImports = jsOutput[1].imports.filter(item => item.external).map(item => item.path);
    assert.deepEqual([...new Set(externalImports)].sort(), ['react', 'react/jsx-runtime'], 'Only the shared React runtime may remain external');
    assert(Object.keys(compiled.metafile!.inputs).some(name => name.includes('@apparent/record/') && name.includes('/dist/ui/')),
      'Build did not consume the compiled public Record UI package');
    assert(!Object.keys(compiled.metafile!.inputs).some(name => name.includes('@apparent/record/src/')),
      'Build must not compile Record source');

    const files: Record<string, Buffer> = {};
    for (const item of [...(compiled.outputFiles ?? []), ...(compiledCss.outputFiles ?? [])]) {
      const name = relative(stage, resolve(item.path)).replaceAll('\\', '/');
      assert(!name.startsWith('../') && !name.startsWith('/'), 'Compiler output escaped the stream directory');
      assert(name === 'index.js' || name === 'style.css' || /^fonts\/[A-Za-z0-9][A-Za-z0-9._-]{0,95}\.(ttf|woff2)$/u.test(name),
        `Unexpected Record artifact output: ${name}`);
      assert(!files[name], `Duplicate artifact output: ${name}`);
      files[name] = Buffer.from(item.contents);
    }
    assert(files['index.js'] && files['style.css'], `Record JavaScript and compiled stylesheet are required (emitted: ${Object.keys(files).join(', ')})`);
    const fontFiles = Object.keys(files).filter(name => name.startsWith('fonts/'));
    assert(fontFiles.length > 0, 'The public Record stylesheet did not emit its font assets');
    for (const [name, bytes] of Object.entries(files)) {
      const path = join(stage, name);
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, bytes, { flag: 'wx' });
    }

    const assets = Object.fromEntries(Object.entries(files)
      .filter(([name]) => name !== 'index.js').map(([name, bytes]) => [name, digest(bytes)]));
    const manifest = parseStreamManifest({
      schemaVersion: 1, id: 'record', version: recordPackage.version, label: 'Record',
      shellApi: APP_SHELL_API, reactVersion: REACT_VERSION, entry: 'index.js',
      sha256: digest(files['index.js']), assets,
    });
    await writeFile(join(stage, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx' });
    await readStream(stage);

    // Exclusively reserve a new destination; publication can never replace an existing artifact.
    await mkdir(output);
    const reservedStat = await lstat(output);
    reservation = { dev: reservedStat.dev, ino: reservedStat.ino };
    await rename(stage, output);
    reservation = undefined;
    return manifest;
  } catch (error) {
    if (reservation) {
      try {
        const current = await lstat(output);
        if (current.isDirectory() && current.dev === reservation.dev && current.ino === reservation.ino) await rmdir(output);
      } catch { /* Do not remove a destination whose identity or contents changed. */ }
    }
    throw error;
  } finally {
    await rm(work, { recursive: true, force: true });
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const target = process.argv[2];
  assert(target, 'Usage: node scripts/build-record.ts <new-output-directory>');
  await buildRecordArtifact(target);
}
