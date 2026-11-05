/** Verify the packed artifact in consumers that cannot see repository siblings. */
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const run = promisify(execFile);
const scratch = await mkdtemp(path.join(tmpdir(), 'record-packed-'));
const headless = path.join(scratch, 'headless');
const ui = path.join(scratch, 'ui');
const bundle = path.join(scratch, 'bundle');
const root = fileURLToPath(new URL('../', import.meta.url));
const tarball = path.join(scratch, 'apparent-record-0.1.0.tgz');

async function initConsumer(directory: string): Promise<void> {
  await mkdir(directory);
  await writeFile(path.join(directory, 'package.json'), JSON.stringify({ private: true, type: 'module' }));
}

try {
  // The caller builds once before the smoke. Packing the resulting artifact does
  // not rerun its build; tarball installs below use normal lifecycle and peers.
  await run('npm', ['pack', '--ignore-scripts', '--pack-destination', scratch], { cwd: root });
  await Promise.all([initConsumer(headless), initConsumer(ui), initConsumer(bundle)]);

  await run('npm', ['install', '--no-audit', '--no-fund', tarball], { cwd: headless });
  const headlessInstalled = path.join(headless, 'node_modules/@apparent/record');
  const manifest = JSON.parse(await readFile(path.join(headlessInstalled, 'package.json'), 'utf8'));
  assert.equal(manifest.scripts?.prepare, 'npm run build', 'source author lifecycle remains compatible');
  for (const value of Object.values(manifest.dependencies ?? {})) {
    assert.ok(!String(value).startsWith('file:'), 'packed dependencies must not point into the repository');
  }
  const declarations = path.join(headless, 'consumer.ts');
  await writeFile(declarations, `
    import { RecordWorkflow } from '@apparent/record';
    import { createRecordHostRuntime, SqliteCaptureMappingStore, PrivateBundleAllocator } from '@apparent/record/host';
    import { decodeSelectedFrames, assembleProcessingOutput } from '@apparent/record/processing';
    import { RecordMediaJob, processCompletedBundle, createProcessingOutputDirectory } from '@apparent/record/processing/media-consumer';
    import { createRecordCapabilityProvider, hasConfirmedArtifactOpen } from '@apparent/record/capabilities';
    import { createRecordFixture } from '@apparent/record/fixtures';
    void [RecordWorkflow, createRecordHostRuntime, SqliteCaptureMappingStore, PrivateBundleAllocator, decodeSelectedFrames, assembleProcessingOutput, RecordMediaJob, processCompletedBundle, createProcessingOutputDirectory, createRecordCapabilityProvider, hasConfirmedArtifactOpen];
  `);
  const tsc = path.join(root, 'node_modules/typescript/bin/tsc');
  await run(process.execPath, [tsc, '--noEmit', '--strict', '--skipLibCheck', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', declarations], { cwd: headless });
  const runtime = await run(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import { createRequire } from 'node:module';
    import { RecordWorkflow } from '@apparent/record';
    import { createRecordHostRuntime, SqliteCaptureMappingStore, PrivateBundleAllocator } from '@apparent/record/host';
    import { decodeSelectedFrames, assembleProcessingOutput } from '@apparent/record/processing';
    import { RecordMediaJob, processCompletedBundle, createProcessingOutputDirectory } from '@apparent/record/processing/media-consumer';
    import { createRecordCapabilityProvider, hasConfirmedArtifactOpen } from '@apparent/record/capabilities';
    import { createRecordFixture } from '@apparent/record/fixtures';
    const require = createRequire(import.meta.url);
    for (const name of ['react', 'react-dom', 'electron', '@apparent/ui']) assert.throws(() => require.resolve(name));
    assert.equal(typeof RecordWorkflow, 'function');
    assert.equal(typeof createRecordHostRuntime, 'function');
    assert.equal(typeof decodeSelectedFrames, 'function');
    for (const exported of [SqliteCaptureMappingStore, PrivateBundleAllocator, assembleProcessingOutput, RecordMediaJob, processCompletedBundle, createProcessingOutputDirectory]) assert.equal(typeof exported, 'function');
    assert.equal(typeof createRecordCapabilityProvider, 'function');
    assert.equal(typeof hasConfirmedArtifactOpen, 'function');
    assert.equal(hasConfirmedArtifactOpen(createRecordFixture().driver.current(), 'unknown'), false);
    console.log('headless packed runtime and declaration consumer passed');
  `], { cwd: headless });
  process.stdout.write(runtime.stdout);

  // The canonical bundle subpath uses only the packed public SDK contract.
  await run('npm', ['install', '--no-audit', '--no-fund', tarball, path.join(root, 'vendor/engine-sdk-0.3.1.tgz')], { cwd: bundle });
  const bundleDeclarations = path.join(bundle, 'consumer.ts');
  await writeFile(bundleDeclarations, `
    import { assembleCanonicalBundle, type BundleAssemblyInput, type BundleValidators } from '@apparent/record/processing/bundle';
    import { validateMaterialsDocument, validateFactFile, type MaterialsDocument, type ContextFacts, type InteractionFacts, type TranscriptFacts, type StrokeFacts } from '@apparent/engine-sdk/capture';
    const validators: BundleValidators = { materials: validateMaterialsDocument, fact: validateFactFile };
    const consume = (input: BundleAssemblyInput) => assembleCanonicalBundle(input, validators);
    void consume; void (null as unknown as MaterialsDocument | ContextFacts | InteractionFacts | TranscriptFacts | StrokeFacts);
  `);
  const bundleTsc = path.join(root, 'node_modules/typescript/bin/tsc');
  await run(process.execPath, [bundleTsc, '--noEmit', '--strict', '--skipLibCheck', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', bundleDeclarations], { cwd: bundle });
  const bundleRuntime = await run(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import { assembleCanonicalBundle } from '@apparent/record/processing/bundle';
    import { validateMaterialsDocument, validateFactFile } from '@apparent/engine-sdk/capture';
    assert.equal(typeof assembleCanonicalBundle, 'function');
    assert.equal(typeof validateMaterialsDocument, 'function');
    assert.equal(typeof validateFactFile, 'function');
    console.log('packed canonical bundle runtime and SDK consumer passed');
  `], { cwd: bundle });
  process.stdout.write(bundleRuntime.stdout);

  // The UI consumer opts into the declared peers explicitly and exercises the
  // prebuilt component, stylesheet and font artifact.
  await run('npm', ['install', '--no-audit', '--no-fund', 'react@^19.2.0', 'react-dom@^19.2.0', tarball], { cwd: ui });
  const uiInstalled = path.join(ui, 'node_modules/@apparent/record');
  const css = await readFile(path.join(uiInstalled, 'dist/ui/style.css'), 'utf8');
  assert.ok(!/@import|@source|shared\/assets/.test(css), 'stylesheet must be compiled and self-contained');
  assert.ok((await readFile(path.join(uiInstalled, 'dist/ui/HostGrotesk-Variable.ttf'))).length > 1000);
  const uiResult = await run(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import React from 'react';
    import { renderToString } from 'react-dom/server';
    import { RecordingPill } from '@apparent/record/ui';
    assert.match(renderToString(React.createElement(RecordingPill)), /Connecting/);
    console.log('React UI packed consumer passed');
  `], { cwd: ui });
  process.stdout.write(uiResult.stdout);
} finally {
  await rm(scratch, { recursive: true, force: true });
}
