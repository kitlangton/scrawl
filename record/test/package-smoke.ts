/** Verify the packed artifact in consumers that cannot see repository siblings. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, writeFile, copyFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const run = promisify(execFile);
const scratch = await mkdtemp(path.join(tmpdir(), 'record-packed-'));
const headless = path.join(scratch, 'headless');
const ui = path.join(scratch, 'ui');
const publication = path.join(scratch, 'publication');
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
  await Promise.all([initConsumer(headless), initConsumer(ui), initConsumer(publication)]);

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
    for (const name of ['react', 'react-dom', 'electron', '@apparent/ui', '@apparent/engine-sdk']) assert.throws(() => require.resolve(name));
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
  // The publication profile explicitly supplies the published SDK; it is not
  // installed for the other headless or UI consumers.
  const sdkArtifact = process.env.APPARENT_ENGINE_SDK_TARBALL ?? path.join(root, 'vendor/engine-sdk-0.3.1.tgz');
  const sdkBytes = await readFile(sdkArtifact);
  assert.equal(createHash('sha256').update(sdkBytes).digest('hex'), '1b605f06273f9c1a134b68b0ed233605b120813214914ca70ed0c8c5f0346f04');
  const sdkTarball = path.join(scratch, 'engine-sdk-0.3.1.tgz'); await copyFile(sdkArtifact, sdkTarball);
  await run('npm', ['install', '--no-audit', '--no-fund', sdkTarball, tarball], { cwd: publication });
  const publicationResult = await run(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import {createRequire} from 'node:module';
    import {CaptureClient, preflightCapturePublication} from '@apparent/engine-sdk/capture';
    import {ReferenceCapturePeer} from '@apparent/engine-sdk/capture/testing';
    import {createRecordPublicationRecovery} from '@apparent/record/publication';
    const client = new CaptureClient(new ReferenceCapturePeer(), {allowTestDouble:true});
    const recovery = createRecordPublicationRecovery(client, process.cwd() + '/unused-private-root', preflightCapturePublication);
    assert.equal(typeof recovery.publish, 'function');
    for (const name of ['react','react-dom','electron','@apparent/ui']) assert.throws(()=>createRequire(import.meta.url).resolve(name));
    await client.close();
    console.log('packed publication profile passed without UI peers');
  `], { cwd: publication });
  process.stdout.write(publicationResult.stdout);
  const sdkDeclarations = path.join(publication, 'consumer.ts');
  await writeFile(sdkDeclarations, `
    import {preflightCapturePublication, type CaptureClient} from '@apparent/engine-sdk/capture';
    import {createRecordPublicationRecovery} from '@apparent/record/publication';
    declare const client: CaptureClient;
    const recovery = createRecordPublicationRecovery(client, '/private/control', preflightCapturePublication);
    const result: ReturnType<CaptureClient['publish']> = recovery.publish({} as Parameters<CaptureClient['publish']>[0]);
    void result;
  `);
  await run(process.execPath, [tsc, '--noEmit', '--strict', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', sdkDeclarations], { cwd: publication });
} finally {
  await rm(scratch, { recursive: true, force: true });
}
