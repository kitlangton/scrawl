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
const bundle = path.join(scratch, 'bundle');
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
  await Promise.all([initConsumer(headless), initConsumer(ui), initConsumer(bundle), initConsumer(publication)]);

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
    import { createRecordHostRuntime, SqliteCaptureMappingStore, PrivateBundleAllocator, AbsentDestinationAllocator, type AbsentDestinationBinding } from '@apparent/record/host';
    import { decodeSelectedFrames, assembleProcessingOutput } from '@apparent/record/processing';
    import { RecordMediaJob, processCompletedBundle, createProcessingOutputDirectory } from '@apparent/record/processing/media-consumer';
    import { createRecordCapabilityProvider, hasConfirmedArtifactOpen } from '@apparent/record/capabilities';
    import { createRecordNativePort, type NativeHostPortInput } from '@apparent/record/host/native';
    import { createRecordFixture } from '@apparent/record/fixtures';
    declare const destinations: AbsentDestinationAllocator;
    const exact: Promise<AbsentDestinationBinding> = destinations.reserveExact('request', '/home/user/output/requested.apparent');
    declare const nativePort: NativeHostPortInput;
    void createRecordNativePort(nativePort, { requiredPermissionIds: ['screen-recording'] });
    void [RecordWorkflow, createRecordHostRuntime, SqliteCaptureMappingStore, PrivateBundleAllocator, AbsentDestinationAllocator, decodeSelectedFrames, assembleProcessingOutput, RecordMediaJob, processCompletedBundle, createProcessingOutputDirectory, createRecordCapabilityProvider, createRecordNativePort, hasConfirmedArtifactOpen, exact];
  `);
  const tsc = path.join(root, 'node_modules/typescript/bin/tsc');
  await run(process.execPath, [tsc, '--noEmit', '--strict', '--skipLibCheck', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', declarations], { cwd: headless });
  const runtime = await run(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import { createRequire } from 'node:module';
    import { RecordWorkflow } from '@apparent/record';
    import { createRecordHostRuntime, SqliteCaptureMappingStore, PrivateBundleAllocator, AbsentDestinationAllocator } from '@apparent/record/host';
    import { decodeSelectedFrames, assembleProcessingOutput } from '@apparent/record/processing';
    import { RecordMediaJob, processCompletedBundle, createProcessingOutputDirectory } from '@apparent/record/processing/media-consumer';
    import { createRecordCapabilityProvider, hasConfirmedArtifactOpen } from '@apparent/record/capabilities';
    import { createRecordNativePort } from '@apparent/record/host/native';
    import { createRecordFixture } from '@apparent/record/fixtures';
    const require = createRequire(import.meta.url);
    for (const name of ['react', 'react-dom', 'electron', '@apparent/ui', '@apparent/engine-sdk']) assert.throws(() => require.resolve(name));
    assert.equal(typeof RecordWorkflow, 'function');
    assert.equal(typeof createRecordHostRuntime, 'function');
    const requested = process.cwd() + '/requested.apparent';
    const exactDestinations = new AbsentDestinationAllocator(process.cwd() + '/private-control', process.cwd() + '/default-output');
    const binding = await exactDestinations.reserveExact('packed-exact-request', requested);
    assert.equal(binding.bundlePath, requested);
    assert.deepEqual(await new AbsentDestinationAllocator(process.cwd() + '/private-control', process.cwd() + '/unused-default').reserveExact('packed-exact-request', requested), binding);
    await assert.rejects((await import('node:fs/promises')).lstat(requested), error => error.code === 'ENOENT');
    assert.equal(typeof decodeSelectedFrames, 'function');
    for (const exported of [SqliteCaptureMappingStore, PrivateBundleAllocator, AbsentDestinationAllocator, assembleProcessingOutput, RecordMediaJob, processCompletedBundle, createProcessingOutputDirectory]) assert.equal(typeof exported, 'function');
    assert.equal(typeof createRecordCapabilityProvider, 'function');
    assert.equal(typeof createRecordNativePort, 'function');
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
    import {readCaptureMaterials, type CaptureMaterialsInput} from '@apparent/record/host/materials';
    const validators: BundleValidators = { materials: validateMaterialsDocument, fact: validateFactFile };
    const consume = (input: BundleAssemblyInput) => assembleCanonicalBundle(input, validators);
    const loadAndAssemble = async (input: CaptureMaterialsInput, policy: Omit<BundleAssemblyInput, 'manifest' | 'payloadBytes' | 'facts'>) => {
      const loaded = await readCaptureMaterials(input);
      return assembleCanonicalBundle({ ...policy, manifest: loaded.materials.manifest, payloadBytes: loaded.payloadBytes, facts: loaded.assemblyFacts }, validators);
    };
    void loadAndAssemble; void consume; void (null as unknown as MaterialsDocument | ContextFacts | InteractionFacts | TranscriptFacts | StrokeFacts);
  `);
  const bundleTsc = path.join(root, 'node_modules/typescript/bin/tsc');
  await run(process.execPath, [bundleTsc, '--noEmit', '--strict', '--types', 'node', '--typeRoots', path.join(root, 'node_modules/@types'), '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', bundleDeclarations], { cwd: bundle });
  const bundleRuntime = await run(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import { assembleCanonicalBundle } from '@apparent/record/processing/bundle';
    import { validateMaterialsDocument, validateFactFile } from '@apparent/engine-sdk/capture';
    const {readCaptureMaterials} = await import('@apparent/record/host/materials');
    assert.equal(typeof readCaptureMaterials, 'function');
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
    import {createRecordPublicationRecovery, createCompletedCapturePublisher, prepareRecordPublication} from '@apparent/record/publication';
    import {createCapture2Host} from '@apparent/record/host/capture2';
    const client = new CaptureClient(new ReferenceCapturePeer(), {allowTestDouble:true});
    assert.equal((await client.handshake()).readiness, 'unavailable');
    const recovery = createRecordPublicationRecovery(client, process.cwd() + '/unused-private-root', preflightCapturePublication);
    assert.equal(typeof recovery.publish, 'function');
    assert.equal(typeof createCapture2Host, 'function');
    assert.equal(typeof prepareRecordPublication, 'function');
    assert.equal(typeof createCompletedCapturePublisher, 'function');
    assert.equal(typeof createCompletedCapturePublisher(client, process.cwd() + '/unused-private-root', preflightCapturePublication).publishCompletedCapture, 'function');
    for (const name of ['react','react-dom','electron','@apparent/ui']) assert.throws(()=>createRequire(import.meta.url).resolve(name));
    await client.close();
    console.log('packed publication profile passed without UI peers');
  `], { cwd: publication });
  process.stdout.write(publicationResult.stdout);
  const sdkDeclarations = path.join(publication, 'consumer.ts');
  await writeFile(sdkDeclarations, `
    import {preflightCapturePublication, type CaptureClient, type SourceDescriptor, type CaptureOptions, type InvocationFact} from '@apparent/engine-sdk/capture';
    import {createCapture2Host, type Capture2Host, type Capture2StartIntent} from '@apparent/record/host/capture2';
    import {createCompletedCapturePublisher, createRecordPublicationRecovery, prepareRecordPublication, type CompletedCapturePublisher, type PublishCompletedCaptureInput, type ReconcileCompletedCaptureInput} from '@apparent/record/publication';
    declare const client: CaptureClient;
    declare const host: Capture2Host;
    declare const intent: Capture2StartIntent;
    declare const source: SourceDescriptor;
    declare const options: CaptureOptions;
    declare const invocation: InvocationFact;
    const exactStart: Capture2StartIntent = { requestId: 'request', source, options, invocation, destinationPath: '/home/user/output/requested.apparent' };
    const exactReceipt: ReturnType<Capture2Host['start']> = host.start(exactStart);
    const capture: ReturnType<Capture2Host['start']> = host.start(intent);
    const recovery = createRecordPublicationRecovery(client, '/private/control', preflightCapturePublication);
    const result: ReturnType<CaptureClient['publish']> = recovery.publish({} as Parameters<CaptureClient['publish']>[0]);
    const stage: (input: Parameters<typeof prepareRecordPublication>[0]) => Promise<import('@apparent/record/publication').PreparedRecordPublication> = prepareRecordPublication;
    const publisher = createCompletedCapturePublisher(client, '/private/control', preflightCapturePublication);
    const publish: (input: PublishCompletedCaptureInput) => ReturnType<CompletedCapturePublisher['publishCompletedCapture']> = publisher.publishCompletedCapture;
    const reconcile: (input: ReconcileCompletedCaptureInput) => ReturnType<CompletedCapturePublisher['reconcile']> = publisher.reconcile;
    void result; void stage; void publish; void reconcile; void capture; void exactReceipt;
  `);
  await run(process.execPath, [tsc, '--noEmit', '--strict', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', sdkDeclarations], { cwd: publication });
} finally {
  await rm(scratch, { recursive: true, force: true });
}
