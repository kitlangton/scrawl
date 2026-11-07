/** Verify the packed artifact in consumers that cannot see repository siblings. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmod, mkdtemp, mkdir, readFile, writeFile, copyFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { assertQualificationShutdownProtocol, checkGeneratedNodeScript, runQualification } from './native-qualification/native-qualification.ts';

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
    import { createRecordCapabilityProvider, hasConfirmedArtifactOpen, type DurableNoteSaveOutcome, type RecordCommand, type ScreenStrokeCursor, type ShortcutTrialBinding, type ShortcutTrialFact, type ShortcutTrialReservationIdentity } from '@apparent/record/capabilities';
    import { createRecordLocalClipboardTextReadbackPort, type NativeClipboardTextReadbackPortInput, createRecordNativePort, type NativeHostPortInput } from '@apparent/record/host/native';
    import { createRecordFixture } from '@apparent/record/fixtures';
    declare const cursor: ScreenStrokeCursor;
    declare const trial: ShortcutTrialFact;
    declare const note: DurableNoteSaveOutcome;
    const freshBinding: ShortcutTrialBinding = { sourceEpoch: 'current-native-epoch', sourceRevision: 8, bindingId: 'effective-registration-2', effectiveBinding: 'Alt+R' };
    const predecessor: ShortcutTrialReservationIdentity = { sourceEpoch: 'old-native-epoch', sourceRevision: 3, bindingId: 'old-registration', effectiveBinding: 'Ctrl+Shift+R', reservationId: 'old-reservation' };
    const rearm: Extract<RecordCommand, {type:'begin-shortcut-trial'}> = { type: 'begin-shortcut-trial', requestId: 'request', expected: {providerEpoch:'record',revision:1}, reservationId: 'new-reservation', ...freshBinding, expiresAt: '2026-11-07T00:00:00.000Z', replaces: predecessor };
    void [cursor, trial, note, rearm];
    declare const destinations: AbsentDestinationAllocator;
    const exact: Promise<AbsentDestinationBinding> = destinations.reserveExact('request', '/home/user/output/requested.apparent');
    declare const transientClipboard: NativeClipboardTextReadbackPortInput;
    declare const resolution: Parameters<typeof createRecordLocalClipboardTextReadbackPort>[0]['resolveHandoff'];
    const textPort = createRecordLocalClipboardTextReadbackPort({clipboard:transientClipboard,nativeHost:{readSnapshot:async()=>({processEpoch:'host',revision:1})},maximumTextBytes:21800,resolveHandoff:resolution});
    void textPort;
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
    import { createRecordLocalClipboardTextReadbackPort, createRecordNativePort } from '@apparent/record/host/native';
    assert.equal(typeof createRecordLocalClipboardTextReadbackPort, 'function');
    import { createRecordFixture } from '@apparent/record/fixtures';
    const require = createRequire(import.meta.url);
    for (const name of ['react', 'react-dom', 'electron', '@apparent/ui', '@apparent/engine-sdk', '@apparent/app-contracts']) assert.throws(() => require.resolve(name));
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
    const trial = createRecordFixture('shortcut-trial-physical-press-release').driver.current().shortcutTrial;
    assert.equal(trial?.state, 'observed');
    const note = createRecordFixture('practice-note-first-post-stroke').driver.current().captures[0].outcomes[0];
    assert.equal(note?.kind, 'note-save');
    assert.deepEqual(note?.kind === 'note-save' && note.ok ? note.screenStrokeBarrier : null, { historyId: 'screen-history-1', sequence: 10 });
    const rearm = createRecordFixture('shortcut-trial-terminal-rearm-available').driver.current();
    assert.equal(rearm.shortcutTrial?.state, 'expired');
    assert.equal(rearm.shortcutTrialBeginBinding?.sourceRevision, 7);
    assert.equal(createRecordFixture('shortcut-trial-active-unknown-no-rearm').driver.current().shortcutTrialBeginBinding, undefined);
    console.log('headless packed runtime and declaration consumer passed');
  `], { cwd: headless });
  process.stdout.write(runtime.stdout);

  // The canonical bundle subpath uses only the packed public SDK contract.
  const sdkArtifact = process.env.APPARENT_ENGINE_SDK_TARBALL ?? path.join(root, 'vendor/engine-sdk-0.4.2.tgz');
  const sdkBytes = await readFile(sdkArtifact);
  assert.equal(createHash('sha256').update(sdkBytes).digest('hex'), '62b1b2e103a585dbf10b848c9538ddcf888b3643b85d7eb89db8c1a61647066c');
  await run('npm', ['install', '--no-audit', '--no-fund', tarball, sdkArtifact], { cwd: bundle });
  const bundleDeclarations = path.join(bundle, 'consumer.ts');
  await writeFile(bundleDeclarations, `
    import { RecordMedia2Job, type RecordMedia2JobState } from '@apparent/record/processing/media2-consumer';
    import { readRetainedDerivativeArtifacts, type ReadRetainedDerivativeArtifactsInput } from '@apparent/record/processing/retained-artifacts';
    import { Media2Client, preflightMedia2Dispatch, type Media2JobInput } from '@apparent/engine-sdk/media2';
    import { readFrameVisualObservations, type FrameVisualObservationInput, type FrameVisualObservation, type VisualObservationSource } from '@apparent/engine-sdk/observations';
    import { assembleCanonicalBundle, type BundleAssemblyInput, type BundleValidators, type CanonicalStroke } from '@apparent/record/processing/bundle';
    import type { PreparePublicationInput } from '@apparent/record/publication';
    import { validateMaterialsDocument, validateFactFile, type MaterialsDocument, type ContextFacts, type InteractionFacts, type TranscriptFacts, type StrokeFacts, type StrokeRgb } from '@apparent/engine-sdk/capture';
    import {readCaptureMaterials, type CaptureMaterialsInput} from '@apparent/record/host/materials';
    import {createCapture2RecordRuntime, type Capture2RecordRuntimeOptions} from '@apparent/record/host/capture2-runtime';
    import {createStandardRecordCaptureCoordinator, type StandardRecordCaptureCoordinatorOptions, createRecordCaptureCoordinator, type RecordCaptureCoordinator, type RecordCaptureCoordinatorShutdownReport, type RecordCaptureProcessingSelection} from '@apparent/record/host/capture2-coordinator';
    declare const capture2RuntimeOptions: Capture2RecordRuntimeOptions;
    void createCapture2RecordRuntime(capture2RuntimeOptions);
    declare const standardOptions: StandardRecordCaptureCoordinatorOptions;
    const standardFlow: Promise<RecordCaptureCoordinator> = createStandardRecordCaptureCoordinator(standardOptions);
    void standardFlow;
    declare const coordinator: RecordCaptureCoordinator;
    const factsPort = coordinator.captureFacts;
    const processingIssues = coordinator.processingIssues();
    const detachProcessingIssues: () => void = coordinator.observeProcessingIssues(() => undefined);
    const coordinatorDisposal: Promise<void> = coordinator.dispose();
    const shutdownReport: Promise<RecordCaptureCoordinatorShutdownReport> = coordinator.shutdown();
    void [processingIssues, detachProcessingIssues, coordinatorDisposal, shutdownReport];
    const handoffResolver: RecordCaptureCoordinator['resolveHandoff'] = coordinator.resolveHandoff;
    declare const processingSelection: RecordCaptureProcessingSelection;
    const createMedia2Job = (client: Media2Client, input: Media2JobInput) => RecordMedia2Job.create(client, '/tmp/record-media2-job.json', input);
    const consumeRetainedArtifacts = async (input: ReadRetainedDerivativeArtifactsInput) => {
      const retained = await readRetainedDerivativeArtifacts(input);
      const assemblyArtifacts: BundleAssemblyInput['artifacts'] = retained.artifacts;
      const publicationBindings: NonNullable<PreparePublicationInput['derivative2Artifacts']> = retained.derivative2Artifacts;
      return { assemblyArtifacts, publicationBindings };
    };
    const validators: BundleValidators = { materials: validateMaterialsDocument, fact: validateFactFile };
    const consume = (input: BundleAssemblyInput) => assembleCanonicalBundle(input, validators);
    const retainedVideo: BundleAssemblyInput['materialReferences'] = [{ role: 'video', relativePath: 'capture.mp4' }];
    const loadAndAssemble = async (input: CaptureMaterialsInput, policy: Omit<BundleAssemblyInput, 'manifest' | 'payloadBytes' | 'facts'>) => {
      const loaded = await readCaptureMaterials(input);
      return assembleCanonicalBundle({ ...policy, manifest: loaded.materials.manifest, payloadBytes: loaded.payloadBytes, facts: loaded.assemblyFacts }, validators);
    };
    const rgb: StrokeRgb = [12, 34, 56];
    const canonical: CanonicalStroke = { id: 'record-mapping', t0: 0, t1: 1, fw: 1, fh: 1, pts: [0, 0, 0], rgb };
    declare const media2Input: Media2JobInput;
    const media2State: RecordMedia2JobState = 'ready';
    declare const visualInput: FrameVisualObservationInput;
    const visualObservation: FrameVisualObservation = readFrameVisualObservations(visualInput);
    const visualSource: VisualObservationSource = visualObservation.source;
    if (visualObservation.state === 'measured') {
      const thumbnailCell: number = visualObservation.thumbnail.cells[0]!;
      const regionCell: number = visualObservation.regions.cells[0]!;
      void [visualObservation.schema, visualObservation.algorithm, thumbnailCell, regionCell];
    } else {
      const unavailableReason: 'observation-profile-absent' | 'observation-profile-unsupported' = visualObservation.reason;
      void unavailableReason;
    }
    void [visualInput.document, visualInput.materials, visualInput.requestIndex, visualInput.pngBytes, visualInput.maxFileBytes, visualSource.materials, visualSource.requestSha256, visualSource.file];
    void [RecordMedia2Job, Media2Client, preflightMedia2Dispatch, media2Input, media2State, createMedia2Job, consumeRetainedArtifacts, createRecordCaptureCoordinator, factsPort, handoffResolver, processingSelection, readFrameVisualObservations];
    void canonical; void loadAndAssemble; void consume; void retainedVideo; void (null as unknown as MaterialsDocument | ContextFacts | InteractionFacts | TranscriptFacts | StrokeFacts);
  `);
  const bundleTsc = path.join(root, 'node_modules/typescript/bin/tsc');
  await run(process.execPath, [bundleTsc, '--noEmit', '--strict', '--types', 'node', '--typeRoots', path.join(root, 'node_modules/@types'), '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', bundleDeclarations], { cwd: bundle });
  const bundleRuntime = await run(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import {createRequire} from 'node:module';
    import { assembleCanonicalBundle } from '@apparent/record/processing/bundle';
    import { RecordMedia2Job } from '@apparent/record/processing/media2-consumer';
    import { readRetainedDerivativeArtifacts } from '@apparent/record/processing/retained-artifacts';
    import { validateMaterialsDocument, validateFactFile } from '@apparent/engine-sdk/capture';
    import { Media2Client, preflightMedia2Dispatch } from '@apparent/engine-sdk/media2';
    import { readFrameVisualObservations } from '@apparent/engine-sdk/observations';
    const {readCaptureMaterials} = await import('@apparent/record/host/materials');
    assert.equal(typeof readCaptureMaterials, 'function');
    assert.equal(typeof assembleCanonicalBundle, 'function');
    assert.equal(typeof validateMaterialsDocument, 'function');
    assert.equal(typeof validateFactFile, 'function');
    for (const name of ['react','react-dom','electron']) assert.throws(()=>createRequire(import.meta.url).resolve(name));
    assert.deepEqual([RecordMedia2Job, readRetainedDerivativeArtifacts, Media2Client, preflightMedia2Dispatch, readFrameVisualObservations].map(value => typeof value), ['function','function','function','function','function']);
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
  const sdkTarball = path.join(scratch, 'engine-sdk-0.4.2.tgz'); await copyFile(sdkArtifact, sdkTarball);
  await run('npm', ['install', '--no-audit', '--no-fund', sdkTarball, tarball], { cwd: publication });
  const publicationResult = await run(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import {createRequire} from 'node:module';
    import {CaptureClient, preflightCapturePublication} from '@apparent/engine-sdk/capture';
    import {ReferenceCapturePeer} from '@apparent/engine-sdk/capture/testing';
    import {createRecordPublicationRecovery, createCompletedCapturePublisher, createRecordPublicationOutput, prepareRecordPublication} from '@apparent/record/publication';
    import {prepareMedia2Request, encodeContactSheetLayout, validateDerivative2Document, selectDerivative2Artifact} from '@apparent/engine-sdk/media2';
    import {createCapture2Host} from '@apparent/record/host/capture2';
    import {createCapture2RecordRuntime} from '@apparent/record/host/capture2-runtime';
    import {createRecordCaptureCoordinator, createStandardRecordCaptureCoordinator} from '@apparent/record/host/capture2-coordinator';
    const client = new CaptureClient(new ReferenceCapturePeer(), {allowTestDouble:true});
    assert.equal((await client.handshake()).readiness, 'unavailable');
    const recovery = createRecordPublicationRecovery(client, process.cwd() + '/unused-private-root', preflightCapturePublication);
    assert.equal(typeof recovery.publish, 'function');
    assert.equal(typeof createCapture2Host, 'function');
    assert.equal(typeof createCapture2RecordRuntime, 'function');
    assert.equal(typeof createRecordCaptureCoordinator, 'function');
    assert.equal(typeof createStandardRecordCaptureCoordinator, 'function');
    assert.equal(typeof prepareRecordPublication, 'function');
    assert.equal(typeof createCompletedCapturePublisher, 'function');
    assert.equal(typeof createCompletedCapturePublisher(client, process.cwd() + '/unused-private-root', preflightCapturePublication).publishCompletedCapture, 'function');
    const outputRoot = process.cwd() + '/output-consumer-private';
    const output = createRecordPublicationOutput(client, outputRoot, preflightCapturePublication);
    assert.deepEqual((await output.captureFacts.readFacts()).captures, []);
    assert.equal(await output.resolveHandoff({captureId:'public-request', artifactId:'bundle'}), null);
    const injectedResolver = output.resolveHandoff;
    assert.equal(await injectedResolver({captureId:'public-request', artifactId:'bundle'}), null);
    assert.deepEqual([prepareMedia2Request, encodeContactSheetLayout, validateDerivative2Document, selectDerivative2Artifact].map(value => typeof value), ['function','function','function','function']);
    for (const name of ['react','react-dom','electron','@apparent/ui']) assert.throws(()=>createRequire(import.meta.url).resolve(name));
    await client.close();
    await (await import('node:fs/promises')).rm(outputRoot, {recursive:true, force:true});
    console.log('packed publication profile passed without UI peers');
  `], { cwd: publication });
  process.stdout.write(publicationResult.stdout);

  // Preparation installs the packed consumer and syntax-checks its frozen
  // runner, but never starts Xvfb or a native worker. Execution remains separate.
  if (process.env.APPARENT_CAPTURE2_QUALIFY === '1' || process.env.APPARENT_CAPTURE2_PREPARE_ONLY === '1') {
    const archive = process.env.APPARENT_CAPTURE2_ARCHIVE;
    const mediaArchive = process.env.APPARENT_MEDIA2_ARCHIVE;
    assert.ok(archive && path.isAbsolute(archive), 'APPARENT_CAPTURE2_ARCHIVE must name the admitted archive');
    assert.ok(mediaArchive && path.isAbsolute(mediaArchive), 'APPARENT_MEDIA2_ARCHIVE must name the admitted media archive');
    const qualificationRecordTarball=process.env.APPARENT_CAPTURE2_RECORD_TARBALL;
    assert.ok(qualificationRecordTarball&&path.isAbsolute(qualificationRecordTarball),'APPARENT_CAPTURE2_RECORD_TARBALL must name the admitted #953 Record package');
    const qualificationRecordSha256=createHash('sha256').update(await readFile(qualificationRecordTarball)).digest('hex');
    assert.equal(qualificationRecordSha256,'1f5ee0008734aabc9dfe2234c2cc813b1623af50c7d7aa97b1ce5a010ff412cf','qualification requires the formally admitted #953 Record package');
    const recordAdmissionPath=process.env.APPARENT_CAPTURE2_RECORD_ADMISSION;
    const recordComponentPath=process.env.APPARENT_CAPTURE2_RECORD_COMPONENT;
    assert.ok(recordAdmissionPath&&path.isAbsolute(recordAdmissionPath),'APPARENT_CAPTURE2_RECORD_ADMISSION must name the exact main admission receipt');
    assert.ok(recordComponentPath&&path.isAbsolute(recordComponentPath),'APPARENT_CAPTURE2_RECORD_COMPONENT must name the admitted component envelope');
    const recordAdmission=JSON.parse(await readFile(recordAdmissionPath,'utf8'));
    const recordComponentHash=createHash('sha256').update(await readFile(recordComponentPath)).digest('hex');
    const recordReceipt=JSON.parse(await readFile(recordAdmission.receiptPath,'utf8'));
    assert.equal(recordAdmission.sourceSha,'236a894dcfe6ff79f5def09204c0a8023db4fe90');
    assert.equal(recordAdmission.runId,42328522610); assert.equal(recordAdmission.artifactId,13556098088);
    assert.equal(recordAdmission.componentSha256,'172967ee5deff6b8d432218f44f00c14f08b0b95a5d78329b3b8f0aaa7298b7c');
    assert.equal(path.resolve(recordAdmission.componentPath),path.resolve(recordComponentPath));
    assert.equal(recordReceipt.sourceSha,recordAdmission.sourceSha); assert.equal(recordReceipt.sha256,recordAdmission.componentSha256);
    assert.equal(recordReceipt.runId,String(recordAdmission.runId)); assert.equal(recordReceipt.size,327888);
    assert.equal(recordComponentHash,recordAdmission.componentSha256,'#953 component matches the admitted main receipt');
    const qualificationSdk = process.env.APPARENT_CAPTURE2_SDK_TARBALL;
    assert.ok(qualificationSdk && path.isAbsolute(qualificationSdk), 'APPARENT_CAPTURE2_SDK_TARBALL must name the admitted SDK 0.4.5 tarball');
    assert.equal(createHash('sha256').update(await readFile(qualificationSdk)).digest('hex'), '177f4433964425b0a5d7e4f3228a4db5a47ebcf51deadc5783e7341652d441b2', 'qualification requires the admitted SDK 0.4.5 package');
    const sdkComponentPath='/home/simon/.local/share/apparent-evidence/engine-sdk045-main-4119a08b/admitted-sdk/component.tar.gz';
    const sdkReceiptPath='/home/simon/.local/share/apparent-evidence/engine-sdk045-main-4119a08b/admitted-sdk/receipt.json';
    const sdkReceipt=JSON.parse(await readFile(sdkReceiptPath,'utf8'));
    assert.equal(createHash('sha256').update(await readFile(sdkComponentPath)).digest('hex'),'579abad9ae046f9967e87f7a35cf2bb53523f9b72892ed93dfbe7e51a1a53d9d');
    assert.equal(sdkReceipt.sha256,'579abad9ae046f9967e87f7a35cf2bb53523f9b72892ed93dfbe7e51a1a53d9d');
    assert.equal(sdkReceipt.sourceSha,'4119a08bf6d7057f2123f728553035681de00e06');
    assert.equal(sdkReceipt.buildSourceSha,'fa152688241998ff1c3d8b4c49095ea8d9df578e');
    assert.equal(sdkReceipt.size,62972); assert.equal(sdkReceipt.runId,'42333036882');
    assert.equal(sdkReceipt.reusedFrom.artifactId,'13557563470');
    assert.equal(sdkReceipt.reusedFrom.sourceSha,'fa152688241998ff1c3d8b4c49095ea8d9df578e');
    const recordPackageExtraction=path.join(scratch,'record-953-admitted-component');
    await mkdir(recordPackageExtraction,{mode:0o700});
    await run('tar',['-xzf',recordComponentPath,'-C',recordPackageExtraction],{cwd:scratch});
    const extractedRecordTarball=path.join(recordPackageExtraction,'stream-payload','apparent-record-0.1.0.tgz');
    assert.equal(createHash('sha256').update(await readFile(extractedRecordTarball)).digest('hex'),qualificationRecordSha256,'#953 component contains the exact packed Record tarball used below');
    const capture2 = path.join(scratch, 'capture2');
    await initConsumer(capture2);
    await run('npm', ['install', '--no-audit', '--no-fund', qualificationRecordTarball, qualificationSdk, 'react@19.2.0', 'react-dom@19.2.0'], { cwd: capture2 });
    const qualificationTypes = path.join(capture2, 'consumer.ts');
    await writeFile(qualificationTypes, `
      import { capture2MaterialsRoot, type CaptureClient } from '@apparent/engine-sdk/capture';
      import { type Media2Client } from '@apparent/engine-sdk/media2';
      import { launchCaptureRecoveryPeer, launchMedia2RecoveryPeer, type RecoveryPeer } from '@apparent/engine-sdk/recovery';
      import { createStandardRecordCaptureCoordinator, type StandardRecordCaptureCoordinatorOptions, type RecordCaptureCoordinator } from '@apparent/record/host/capture2-coordinator';
      declare const options: StandardRecordCaptureCoordinatorOptions;
      declare const publicationRecoveryPeer: RecoveryPeer<CaptureClient>;
      declare const mediaRecoveryPeer: RecoveryPeer<Media2Client>;
      const coordinator: Promise<RecordCaptureCoordinator> = createStandardRecordCaptureCoordinator(options);
      declare const opened: RecordCaptureCoordinator;
      const facts: ReturnType<RecordCaptureCoordinator['captureFacts']['readFacts']> = opened.captureFacts.readFacts();
      const handoff: ReturnType<RecordCaptureCoordinator['resolveHandoff']> = opened.resolveHandoff({captureId:'public-request',artifactId:'saved-artifact'});
      const separated: StandardRecordCaptureCoordinatorOptions = {...options, publicationClient: publicationRecoveryPeer.client, mediaClient: mediaRecoveryPeer.client};
      const peerReleases: Promise<unknown[]> = Promise.all([publicationRecoveryPeer.release(), mediaRecoveryPeer.release()]);
      const closeOutcome: Awaited<ReturnType<CaptureClient['close']>> = 'eof-exited';
      const peerLaunches = [launchCaptureRecoveryPeer('/absolute/capture', '/private/runtime'), launchMedia2RecoveryPeer('/absolute/media', '/private/runtime')];
      const trustedRoot: string = capture2MaterialsRoot('/private/runtime', 'capture-id');
      void [coordinator, facts, handoff, separated, peerReleases, closeOutcome, peerLaunches, trustedRoot];
    `);
    const qualificationTsc = path.join(root, 'node_modules/typescript/bin/tsc');
    await run(process.execPath, [qualificationTsc, '--noEmit', '--strict', '--skipLibCheck', '--types', 'node', '--typeRoots', path.join(root, 'node_modules/@types'), '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', qualificationTypes], { cwd: capture2 });
    const qualificationRuntime = await run(process.execPath, ['--input-type=module', '-e', `
      import assert from 'node:assert/strict';
      import { CaptureClient, capture2MaterialsRoot, launchCaptureEngine } from '@apparent/engine-sdk/capture';
      import { launchCaptureRecoveryPeer, launchMedia2RecoveryPeer } from '@apparent/engine-sdk/recovery';
      import { createStandardRecordCaptureCoordinator } from '@apparent/record/host/capture2-coordinator';
      assert.equal(typeof capture2MaterialsRoot, 'function');
      assert.equal(typeof CaptureClient.prototype.close, 'function');
      assert.equal(typeof launchCaptureEngine, 'function');
      assert.equal(typeof launchCaptureRecoveryPeer, 'function');
      assert.equal(typeof launchMedia2RecoveryPeer, 'function');
      assert.equal(typeof createStandardRecordCaptureCoordinator, 'function');
      console.log('packed Record standard coordinator, SDK 0.4.5 owning-close and recovery-peer consumer passed');
    `], { cwd: capture2 });
    process.stdout.write(qualificationRuntime.stdout);
    const nativeScript = `
      import assert from 'node:assert/strict';
      import {createHash} from 'node:crypto';
      import {spawn} from 'node:child_process';
      import {appendFile, chmod, copyFile, lstat, mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile} from 'node:fs/promises';
      import {tmpdir} from 'node:os';
      import path from 'node:path';
      import {pathToFileURL} from 'node:url';
      import {CaptureClient, capture2MaterialsRoot, launchCaptureEngine, preflightCapturePublication} from '@apparent/engine-sdk/capture';
      import {launchCaptureRecoveryPeer, launchMedia2RecoveryPeer} from '@apparent/engine-sdk/recovery';
      import {createStandardRecordCaptureCoordinator} from '@apparent/record/host/capture2-coordinator';
      import {createRecordClientPillHost,disposeWorkflowPillHost} from '@apparent/record/ui';
      import {AbsentDestinationAllocator, SqliteCaptureMappingStore} from '@apparent/record/host';
      const {QUALIFICATION_IPC_SCHEMA,assertNativeQualificationAdmission,assertQualificationShutdownProtocol,canReleaseQualificationRecoveryPeers,closeQualificationCaptureOwner,runBounded,runCleanupSteps,trackChild,waitForExit} = await import(pathToFileURL(process.env.APPARENT_QUALIFICATION_HELPERS).href);

      const archive = path.resolve(process.env.APPARENT_CAPTURE2_ARCHIVE);
      const mediaArchive = path.resolve(process.env.APPARENT_MEDIA2_ARCHIVE);
      const sdkTarball = path.resolve(process.env.APPARENT_CAPTURE2_SDK_TARBALL);
      const root = await realpath(await mkdtemp(path.join(tmpdir(), 'record-standard-native-')));
      await chmod(root, 0o700);
      console.log('private standard qualification root: ' + root);
      const logPath = path.join(root, 'qualification.log');
      await writeFile(logPath, '', {mode:0o600});
      const emittedScript = path.join(root,'qualification-script.mjs');
      await copyFile(process.env.APPARENT_QUALIFICATION_SCRIPT_PATH,emittedScript);
      await chmod(emittedScript,0o600);
      const recordLog = async (label, value) => {
        const line = label + ' ' + JSON.stringify(value);
        console.log(line);
        await appendFile(logPath, line + '\\n', {mode:0o600});
      };
      const fileHash = bytes => createHash('sha256').update(bytes).digest('hex');
      assert.equal(fileHash(await readFile(sdkTarball)), '177f4433964425b0a5d7e4f3228a4db5a47ebcf51deadc5783e7341652d441b2');
      const inputEvidence = path.join(root, 'native-inputs'); await mkdir(inputEvidence,{mode:0o700});
      await copyFile(sdkTarball,path.join(inputEvidence,'apparent-engine-sdk-0.4.5.tgz'));
      await chmod(path.join(inputEvidence,'apparent-engine-sdk-0.4.5.tgz'),0o600);
      async function admitArtifact(kind, archivePath, expected) {
        const evidence=path.dirname(archivePath), evidenceRoot=path.dirname(evidence), archiveBytes=await readFile(archivePath);
        assert.equal(archiveBytes.byteLength,expected.size,kind+' component size'); assert.equal(fileHash(archiveBytes),expected.sha256,kind+' component SHA-256');
        const receipt=JSON.parse(await readFile(path.join(evidence,'receipt.json'),'utf8'));
        const accepted=JSON.parse(await readFile(path.join(evidenceRoot,expected.acceptedFile),'utf8'));
        const inventoryPath=expected.inventoryFile?path.join(evidenceRoot,expected.inventoryFile):expected.inventoryInEvidence?path.join(evidence,'inventory-verification.json'):path.join(evidenceRoot,'inventory-verification.json');
        const inventoryDocument=JSON.parse(await readFile(inventoryPath,'utf8'));
        const inventoryRecord=Array.isArray(inventoryDocument)?inventoryDocument.find(item=>item.receipt?.sha256===expected.sha256):inventoryDocument;
        const inventoryAdmission=Array.isArray(inventoryDocument)?inventoryRecord?.admission:inventoryDocument[kind];
        const inventoryReceipt=Array.isArray(inventoryDocument)?inventoryRecord?.receipt:inventoryDocument[kind+'Receipt'];
        const fileCount=Array.isArray(inventoryDocument)?inventoryRecord?.runtimeFileCount:
          inventoryDocument.runtimeFiles?.length??inventoryDocument.runtimeSeals??inventoryDocument.seals;
        const attestationPath=expected.attestationFile?path.join(evidenceRoot,expected.attestationFile):path.join(evidence,'attestation.json');
        const attestation=JSON.parse(await readFile(attestationPath,'utf8'));
        assert.equal(receipt.sourceSha,expected.sourceSha); assert.equal(receipt.sha256,expected.sha256); assert.equal(receipt.size,expected.size); assert.equal(receipt.runId,String(expected.runId));
        assert.equal(accepted.sourceSha,expected.sourceSha); assert.equal(accepted.runId,expected.runId); assert.equal(accepted.componentSha256,expected.sha256);
        if(inventoryAdmission) {
          assert.equal(inventoryAdmission.componentSha256,expected.sha256); assert.equal(inventoryAdmission.sourceSha,expected.sourceSha); assert.equal(inventoryAdmission.runId,expected.runId);
          assert.equal(inventoryReceipt.sha256,expected.sha256); assert.equal(Number(inventoryReceipt.size),expected.size);
        } else {
          if(inventoryDocument.componentSha256) assert.equal(inventoryDocument.componentSha256,expected.sha256);
          else assert.equal(inventoryDocument.source,expected.sourceSha);
          assert.equal(inventoryDocument.source??inventoryDocument.sourceSha,expected.sourceSha);
          if(inventoryDocument.target!==undefined) assert.equal(inventoryDocument.target,'linux-x64');
          if(inventoryDocument.totalFiles!==undefined) assert.equal(inventoryDocument.totalFiles,expected.files+1,'inventory total includes the manifest');
        }
        assert.equal(fileCount,expected.files);
        if(expected.attestationBindingFile) {
          const binding=JSON.parse(await readFile(path.join(evidence,expected.attestationBindingFile),'utf8'));
          assert.equal(binding.verified,true); assert.equal(binding.source,expected.sourceSha); assert.equal(binding.component,expected.sha256);
          assert.match(binding.certificate?.runInvocationURI??'',new RegExp(String(expected.runId)));
        }
        const bundle=attestation[0]?.attestation?.bundle; assert.ok(bundle,'signed source attestation bundle exists');
        const statement=JSON.parse(Buffer.from(bundle.dsseEnvelope.payload,'base64').toString('utf8'));
        assert.equal(statement.subject[0].digest.sha256,expected.sha256);
        assert.equal(statement.predicate.buildDefinition.resolvedDependencies[0].digest.gitCommit,expected.sourceSha);
        assert.equal(statement.predicate.buildDefinition.externalParameters.workflow.path,'.github/workflows/stream-engine.yml');
        const bundlePath=path.join(inputEvidence,kind+'.attestation.bundle.json');
        await writeFile(bundlePath,JSON.stringify(bundle),{mode:0o600});
        const verification=await runBounded('gh',['attestation','verify',archivePath,'--bundle',bundlePath,'--repo','tryapparent/apparent','--signer-workflow','tryapparent/apparent/.github/workflows/stream-engine.yml','--source-digest',expected.sourceSha,'--source-ref','refs/heads/main'],30000);
        await writeFile(path.join(inputEvidence,kind+'.attestation-verification.log'),verification.stdout+verification.stderr,{mode:0o600});
        for(const name of ['receipt.json','attestation.json']) { const retained=path.join(inputEvidence,kind+'-'+name); await copyFile(name==='attestation.json'?attestationPath:path.join(evidence,name),retained); await chmod(retained,0o600); }
        await copyFile(path.join(evidenceRoot,expected.acceptedFile),path.join(inputEvidence,kind+'-'+expected.acceptedFile));
        await chmod(path.join(inputEvidence,kind+'-'+expected.acceptedFile),0o600);
        const retainedInventory=path.join(inputEvidence,kind+'-inventory-verification.json');
        await copyFile(inventoryPath,retainedInventory); await chmod(retainedInventory,0o600);
        if(expected.attestationBindingFile) {
          const retainedBinding=path.join(inputEvidence,kind+'-attestation-binding.json');
          await copyFile(path.join(evidence,expected.attestationBindingFile),retainedBinding); await chmod(retainedBinding,0o600);
        }
        const extraction=path.join(root,'runtime-artifacts',kind); await mkdir(extraction,{recursive:true,mode:0o700});
        await runBounded('tar',['-xzf',archivePath,'-C',extraction],30000);
        const payload=path.join(extraction,'stream-payload','payload');
        const manifest=JSON.parse(await readFile(path.join(payload,'manifest.json'),'utf8'));
        assert.equal(manifest.sourceSha,expected.manifestSourceSha); assert.equal(manifest.runtimeFiles.length,expected.files);
        const listed=new Map(manifest.runtimeFiles.map(file=>[file.path,file])), seen=new Set();
        async function walk(dir) {
          for(const entry of await readdir(dir,{withFileTypes:true})) {
            const full=path.join(dir,entry.name), relative=path.relative(payload,full), info=await lstat(full);
            assert.equal(info.isSymbolicLink(),false,kind+' payload symlink: '+relative);
            if(info.isDirectory()) await walk(full);
            else { assert.equal(info.isFile(),true,kind+' non-regular payload: '+relative); if(relative==='manifest.json') continue;
              const declared=listed.get(relative); assert.ok(declared,kind+' unlisted payload: '+relative); seen.add(relative);
              assert.equal(info.mode&0o777,declared.mode,kind+' mode: '+relative); assert.equal(fileHash(await readFile(full)),declared.sha256,kind+' hash: '+relative); }
          }
        }
        await walk(payload); assert.equal(seen.size,listed.size,kind+' exact runtime closure');
        if(kind==='media') {
          assert.ok(manifest.capabilities.includes('media/2'));
          assert.deepEqual(manifest.recoveryLaunch.argv,['--media-runtime-directory','<capture-runtime-dir>','--media-channel','media','--media-protocol-version','2','--recovery-peer-v1','<private-peer-nonce>']);
          assert.equal(manifest.recoveryLaunch.startsFreshCapture,false); assert.equal(manifest.recoveryLaunch.onInputEof,'retain-admitted-finite-work');
          assert.equal(manifest.media2Launch.sharesCaptureRuntime,true);
          assert.equal(manifest.visualObservations?.schema,'apparent.frame-observation-profile/1'); assert.equal(manifest.visualObservations?.chunkType,'apOB');
          assert.equal(manifest.launch.entry,'apparent-engine-frames'); assert.equal(manifest.launch.pathLookup,false); assert.equal(manifest.launch.runtimeBuilds,false);
          assert.equal(listed.get('apparent-engine-frames').sha256,'cf86ab5841ea5a847844518bc3e4f0bd94174f23ac86defba65dd2d8493c27e9');
          assert.equal(listed.get('libexec/apparent-engine-frames').sha256,'995ce8722e5b7fa1c579ea8f4db58c68ec9585e227593b4632fcfc8260dceb51');
        } else {
          assert.ok(manifest.capabilities.includes('capture/2')); assert.ok(manifest.capabilities.includes('authenticated-publication'));
          assert.deepEqual(manifest.recoveryLaunch.argv,['--runtime-directory','<private-runtime-dir>','--capture-channel','capture','--capture-protocol-version','2','--recovery-peer-v1','<private-peer-nonce>']);
          assert.equal(manifest.recoveryLaunch.startsFreshCapture,false); assert.equal(manifest.recoveryLaunch.onInputEof,'retain-admitted-finite-work');
          assert.deepEqual(manifest.launch.argv,['--runtime-directory','<private-runtime-dir>','--capture-channel','capture','--capture-protocol-version','2']);
          assert.equal(manifest.launch.shutdownTimeoutMs,40000);
          assert.equal(manifest.launch.entry,'apparent-engine-capture-launch'); assert.equal(manifest.launch.worker,'libexec/apparent-engine-capture');
          assert.equal(manifest.launch.pathLookup,false); assert.equal(manifest.launch.runtimeBuilds,false);
          assert.equal(listed.get('apparent-engine-capture-launch').sha256,'fb2dfa941fcd7f9681a141b8213ecf6031eca82b424e608bbed3309d38b033e6');
          assert.equal(listed.get('libexec/apparent-engine-capture').sha256,'c53e9ee6b850acd7750abf5576468bd6cb9b7c1e655fc64a45af12594cd8767b');
        }
        await recordLog(kind+' admitted archive',{path:archivePath,sha256:expected.sha256,size:expected.size,sourceSha:expected.sourceSha,manifestSourceSha:expected.manifestSourceSha,runtimeFiles:seen.size,payload});
        return {payload,manifest};
      }
      const captureArtifact=await admitArtifact('capture',archive,{acceptedFile:'capture-accepted.json',inventoryFile:'capture-inventory-verification.json',attestationFile:'capture-attestation.json',sourceSha:'4119a08bf6d7057f2123f728553035681de00e06',manifestSourceSha:'4119a08bf6d7057f2123f728553035681de00e06',runId:42333036882,sha256:'6a264d02d2647a21b96a8e83eeaab82af107753e18cd0c28ca4a7aceb5d67b89',size:11198744,files:54});
      const mediaArtifact=await admitArtifact('media',mediaArchive,{acceptedFile:'linux-x64-accepted.json',inventoryInEvidence:true,attestationBindingFile:'attestation-binding.json',sourceSha:'b631acda0af12e30a45cf9885054882cee4da844',manifestSourceSha:'b631acda0af12e30a45cf9885054882cee4da844',runId:42323417518,sha256:'29c5db49ed046c3fb5802fae991027fbacfdd9dbf8df1c834b3a703c73efc730',size:100882653,files:186});
      if(process.env.APPARENT_QUALIFICATION_PREFLIGHT_ONLY==='1') {
        await recordLog('offline archive admission passed',{sdkSha256:fileHash(await readFile(sdkTarball)),capture:archive,media:mediaArchive,
          captureRuntimeFiles:captureArtifact.manifest.runtimeFiles.length,mediaRuntimeFiles:mediaArtifact.manifest.runtimeFiles.length,nativeExecuted:false});
        console.log('exact SDK/Capture/Media archive metadata, attestations and full runtime closures passed; native execution skipped');
        await rm(root,{recursive:true,force:true});
      } else {
      const capturePayload=captureArtifact.payload, captureManifest=captureArtifact.manifest, mediaPayload=mediaArtifact.payload, mediaManifest=mediaArtifact.manifest;
      let xvfb, display='', scene, client, publicationRecoveryPeer, mediaRecoveryPeer, coordinator, recordClient, pillHost, mappings;
      let qualificationError, cleanupErrors=[], finishRequested=false, shutdownRequested=false, shutdownRequest, shutdownPromise;
      let coordinatorConstructionStarted=false, externalAdmissionClosed=false, localBorrowersDrained=false;
      let coordinatorShutdownReport, coordinatorDisposed=false, localAdmissionClosed=false, localDrainComplete=false;
      let qualificationStatus=0, captureOwnerCloseResult, originalTuple=null, authenticatedSavedTuple=null;
      let capturePeerMediaDispatches=0, capturePeerPublicationDispatches=0;
      const recoveryPeerReleases=[];
      const previousDisplay=process.env.DISPLAY, previousWayland=process.env.WAYLAND_DISPLAY;
      const sendShutdownReport=(requestId,report)=>new Promise((resolve,reject)=>{
        if(typeof process.send!=='function') { reject(new Error('qualification supervisor IPC channel is unavailable')); return; }
        process.send({schema:QUALIFICATION_IPC_SCHEMA,kind:'shutdown-report',requestId,report},error=>error?reject(error):resolve());
      });
      const cooperativeShutdown=(request)=>{
        shutdownRequested=true;
        shutdownPromise??=(async()=>{
          if(shutdownRequest&&(shutdownRequest.reason==='deadline'||shutdownRequest.reason==='output-limit')) qualificationStatus=3;
          const steps=[];
          if(pillHost) steps.push({label:'pill/provider admission close',run:async()=>{await disposeWorkflowPillHost(pillHost);pillHost=undefined;if(!recordClient)externalAdmissionClosed=true;}});
          if(recordClient) steps.push({label:'provider admission detach',run:()=>{recordClient.detach();recordClient=undefined;if(!pillHost)externalAdmissionClosed=true;}});
          if(coordinator) steps.push({label:'coordinator shutdown',run:async()=>{
            coordinatorShutdownReport=await coordinator.shutdown();
            await recordLog('coordinator shutdown report',coordinatorShutdownReport);
            localAdmissionClosed=coordinatorShutdownReport.admission==='closed';
            localDrainComplete=coordinatorShutdownReport.localDrain==='complete';
            if(coordinatorShutdownReport.runtimeClearance!=='clear'||coordinatorShutdownReport.work?.some(item=>item.state==='unknown')) qualificationStatus=3;
            if(!localAdmissionClosed||!localDrainComplete) throw new Error('local coordinator admission/drain is unresolved; recovery peers retained');
          }});
          if(coordinator) steps.push({label:'coordinator disposal',run:async()=>{await coordinator.dispose();coordinatorDisposed=true;coordinator=undefined;localBorrowersDrained=localDrainComplete;}});
          const mayReleaseRecoveryPeers=()=>canReleaseQualificationRecoveryPeers({neverInjected:!coordinatorConstructionStarted,
            externalAdmissionClosed,admission:coordinatorShutdownReport?.admission??'unknown',localDrain:coordinatorShutdownReport?.localDrain??'unknown',
            disposed:coordinatorDisposed,borrowersDrained:localBorrowersDrained});
          if(publicationRecoveryPeer) steps.push({label:'publication recovery connection release',run:async()=>{
            if(!mayReleaseRecoveryPeers()) { qualificationStatus=3; return; }
            const result=await publicationRecoveryPeer.release(); recoveryPeerReleases.push({role:'publication',result}); publicationRecoveryPeer=undefined;
            if(result.inputClosure==='unconfirmed') qualificationStatus=3;
          }});
          if(mediaRecoveryPeer) steps.push({label:'media recovery connection release',run:async()=>{
            if(!mayReleaseRecoveryPeers()) { qualificationStatus=3; return; }
            const result=await mediaRecoveryPeer.release(); recoveryPeerReleases.push({role:'media',result}); mediaRecoveryPeer=undefined;
            if(result.inputClosure==='unconfirmed') qualificationStatus=3;
          }});
          if(client) steps.push({label:'dedicated foreground capture owner close',run:async()=>{
            captureOwnerCloseResult=await closeQualificationCaptureOwner({externalAdmissionClosed,admission:coordinatorShutdownReport?.admission??'unknown',
              localDrain:coordinatorShutdownReport?.localDrain??'unknown',disposed:coordinatorDisposed,borrowersDrained:localBorrowersDrained,
              endIntent:finishRequested?'finish':undefined,mediaDispatchedOnCapturePeer:capturePeerMediaDispatches>0,
              publicationDispatchedOnCapturePeer:capturePeerPublicationDispatches>0,close:()=>client.close()});
            if(captureOwnerCloseResult.state==='ended'||(captureOwnerCloseResult.state==='unknown'&&'outcome' in captureOwnerCloseResult)) client=undefined;
            if(captureOwnerCloseResult.status===3) qualificationStatus=3;
          }});
          const errors=await runCleanupSteps(steps);
          if(errors.length) qualificationStatus=3;
          if(!externalAdmissionClosed||!localAdmissionClosed||!localDrainComplete||!coordinatorDisposed||!localBorrowersDrained) qualificationStatus=3;
          const report={schema:QUALIFICATION_IPC_SCHEMA,
            coordinatorShutdown:coordinatorShutdownReport??null,coordinatorDisposed,localAdmissionClosed,localDrainComplete,
            runtimeClearance:coordinatorShutdownReport?.runtimeClearance??'unknown',work:coordinatorShutdownReport?.work??null,
            status:qualificationStatus,externalAdmissionClosed,localBorrowersDrained,capturePeerMediaDispatches,capturePeerPublicationDispatches,
            originalTuple,authenticatedSavedTuple,
            recoveryPeerReleases:captureRecoveryPeerResults(),captureOwner:{state:captureOwnerCloseResult?.state??'retained',finishRequested,
              endIntent:finishRequested?'finish':null,close:captureOwnerCloseResult??{state:'not-attempted',reason:'close step did not complete'},
              localCloseIsNotRuntimeClearance:true},
            cleanupErrors:errors.map(error=>({message:error.message,cause:error.cause instanceof Error?error.cause.message:undefined})),authorityRoot:root};
          await recordLog('cooperative shutdown report',report);
          return {report,errors};
        })();
        return shutdownPromise.then(async ({report,errors})=>{
          const requestId=request.requestId??process.env.APPARENT_QUALIFICATION_RUN_ID;
          assert.equal(typeof requestId,'string','supervisor run identity is available for shutdown reports');
          const correlated={...report,requestId,reason:request.reason};
          await sendShutdownReport(requestId,correlated);
          if(errors.length) throw new AggregateError(errors,'cooperative coordinator shutdown retained cleanup failures');
          return correlated;
        });
      };
      const captureRecoveryPeerResults=()=>structuredClone(recoveryPeerReleases);
      process.on('message',message=>{
        if(!message||typeof message!=='object'||Array.isArray(message)) return;
        if(message.schema!==QUALIFICATION_IPC_SCHEMA||message.kind!=='shutdown-request'||typeof message.requestId!=='string'||
           !['deadline','output-limit'].includes(message.reason)) return;
        shutdownRequest={requestId:message.requestId,reason:message.reason};
        shutdownRequested=true;
        if(coordinator) void cooperativeShutdown(shutdownRequest).catch(error=>recordLog('cooperative shutdown IPC failed',{message:error.message,authorityRoot:root}));
      });
      try {
        xvfb=trackChild(spawn('Xvfb',['-displayfd','1','-screen','0','683x480x24','-nolisten','tcp'],{stdio:['ignore','pipe','pipe']}));
        display=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(new Error('owned Xvfb startup deadline')),5000);
          xvfb.stdout.setEncoding('utf8'); xvfb.stdout.once('data',value=>{clearTimeout(timer);resolve(':'+value.trim());});
          xvfb.once('exit',code=>{clearTimeout(timer);reject(new Error('owned Xvfb exited: '+code));}); xvfb.once('error',reject);});
        const env={...process.env,DISPLAY:display}; delete env.WAYLAND_DISPLAY; process.env.DISPLAY=display; delete process.env.WAYLAND_DISPLAY;
        scene=trackChild(spawn('xmessage',['-geometry','+170+120','-timeout','240','Record standard synthetic Xvfb scene'],{env,stdio:'ignore'}));
        let sceneSpawnError; scene.once('error',error=>{sceneSpawnError=error;});
        const runtime=path.join(root,'runtime'), control=path.join(root,'control'), bundles=path.join(root,'bundles');
        await Promise.all([runtime,control,bundles].map(dir=>mkdir(dir,{mode:0o700})));
        assertQualificationShutdownProtocol(captureManifest.launch.shutdownTimeoutMs===40000);
        client=new CaptureClient(launchCaptureEngine(path.join(capturePayload,captureManifest.launch.entry),runtime,captureManifest.launch.shutdownTimeoutMs));
        const captureEntry=path.join(capturePayload,captureManifest.launch.entry);
        const mediaEntry=mediaManifest.runtimeFiles.find(file=>file.path==='apparent-engine-frames'); assert.ok(mediaEntry);
        publicationRecoveryPeer=await launchCaptureRecoveryPeer(captureEntry,runtime);
        mediaRecoveryPeer=await launchMedia2RecoveryPeer(path.join(mediaPayload,mediaEntry.path),runtime);
        assert.equal(publicationRecoveryPeer.lifetime.channel,'capture/2'); assert.equal(publicationRecoveryPeer.lifetime.role,'recovery');
        assert.equal(publicationRecoveryPeer.lifetime.startsFreshCapture,false); assert.equal(publicationRecoveryPeer.lifetime.onInputEof,'retain-admitted-finite-work');
        assert.equal(mediaRecoveryPeer.lifetime.channel,'media/2'); assert.equal(mediaRecoveryPeer.lifetime.role,'recovery');
        assert.equal(mediaRecoveryPeer.lifetime.startsFreshCapture,false); assert.equal(mediaRecoveryPeer.lifetime.onInputEof,'retain-admitted-finite-work');
        const captureHandshake=await client.handshake(5000); assert.equal(captureHandshake.implementation,'native');
        const mediaHandshake=await mediaRecoveryPeer.client.handshake(5000); assert.equal(mediaHandshake.implementation,'native');
        mappings=new SqliteCaptureMappingStore(control); const destinations=new AbsentDestinationAllocator(control,bundles);
        const options={audio:'none',localOnly:true}, invocation={kind:'explicit',invocationId:'a'.repeat(32),observedAtMs:Date.now()};
        const requestId='packed-standard-native-'+Date.now();
        const createStartIntent=(id,fresh)=>{assert.equal(fresh.state,'available');
          const source=fresh.sources.find(item=>item.kind==='display'&&item.bounds.width===683&&item.bounds.height===480);
          assert.ok(source,'fresh discovery must contain the owned 683x480 synthetic display');
          const digest=createHash('sha256').update(id,'utf8').digest('hex');
          return {source,options,invocation:{...invocation,observedAtMs:Date.now()},destinationPath:path.join(bundles,'record-'+digest+'.apparent')};};
        const coordinatorOptions={captureClient:client,publicationClient:publicationRecoveryPeer.client,mediaClient:mediaRecoveryPeer.client,mappings,destinations,privateControlRoot:control,createStartIntent,
          trustedMaterialsRoot:mapping=>capture2MaterialsRoot(runtime,mapping.captureId),preflight:preflightCapturePublication,
          pollIntervalMs:200,operationTimeoutMs:10000,standardProcessing:{requestVideoProof:true}};
        coordinatorConstructionStarted=true;
        coordinator=await createStandardRecordCaptureCoordinator(coordinatorOptions);
        if(shutdownRequest) { await cooperativeShutdown(shutdownRequest); throw new Error('qualification stopped after cooperative supervisor request'); }
        const createProvider=providerEpoch=>coordinator.createProvider({providerEpoch,native:{readFacts:async()=>({version:1,providerEpoch:providerEpoch+'-facts',revision:1,readiness:[],shortcut:{effective:null,label:null,registration:'unknown',externallyManaged:false,draft:null}})}});
        recordClient=createProvider('packed-standard-native-provider');
        if(sceneSpawnError) throw sceneSpawnError;
        const deadline=Date.now()+150000, waitFor=async(fn,label)=>{while(Date.now()<deadline){if(shutdownRequested)throw new Error('qualification shutdown requested; original capture and authority retained while shutdown report is reconciled');const value=await fn();if(value)return value;await new Promise(r=>setTimeout(r,250));}throw new Error('qualification deadline waiting for '+label);};
        const before=recordClient.current();
        const localOnly=await recordClient.execute({type:'start-capture',delivery:'local-only',requestId:'packed-local-only-'+Date.now(),expected:{providerEpoch:before.providerEpoch,revision:before.revision}});
        assert.equal(localOnly.status,'refused','native delivery prohibition is not proof of whole-pipeline local-only enforcement');
        let actionIds=0; pillHost=createRecordClientPillHost(recordClient,{createRequestId:()=>actionIds++===0?requestId:'packed-pill-action-'+actionIds});
        const initialPill=await pillHost.read(); assert.ok(initialPill.actions.some(action=>action.id==='record'&&action.enabled));
        await pillHost.act(initialPill.revision,'record');
        await waitFor(async()=>recordClient.current().captures.find(item=>item.captureId===requestId&&item.phase==='recording'),'public provider recording fact');
        await new Promise(resolve=>setTimeout(resolve,1800));
        const mapping=await mappings.getByRequestId(requestId); assert.ok(mapping?.capture2Intent);
        originalTuple={requestId:mapping.requestId,captureId:mapping.captureId,bundleId:mapping.bundleId,bundlePath:mapping.bundlePath,
          captureIntentSha256:createHash('sha256').update(mapping.capture2Intent,'utf8').digest('hex')};
        assert.equal(mapping.bundlePath,path.join(bundles,path.basename(mapping.bundlePath)));
        await assert.rejects(lstat(mapping.bundlePath),error=>error.code==='ENOENT','destination stays absent before authenticated publication');
        const recordingPill=await pillHost.read(); assert.equal(recordingPill.phase,'recording'); finishRequested=true; await pillHost.act(recordingPill.revision,'finish');
        const saved=await waitFor(async()=>{await coordinator.reconcile();const facts=await coordinator.captureFacts.readFacts();return facts.captures.find(fact=>fact.captureId===requestId&&fact.phase==='saved');},'authenticated Record Saved output');
        assert.equal(saved.delivery,'unknown','nativeRemoteDelivery forbidden does not imply local-only delivery'); assert.equal(saved.artifactId,mapping.bundleId);
        const originalHandoff=await coordinator.resolveHandoff({captureId:requestId,artifactId:saved.artifactId});
        assert.ok(originalHandoff?.savedBundle.status==='saved','exact public request/artifact binding resolves after authenticated Saved');
        assert.equal(originalHandoff.savedBundle.bundlePath,mapping.bundlePath);
        authenticatedSavedTuple={requestId,captureId:saved.captureId,artifactId:saved.artifactId,bundlePath:originalHandoff.savedBundle.bundlePath};
        await recordLog('authenticated Record Saved output',{requestId,artifactId:saved.artifactId,bundlePath:originalHandoff.savedBundle.bundlePath,
          processing:originalHandoff.processing,requestVideoProof:originalHandoff.requestVideoProof,delivery:saved.delivery});
        const entries=await readdir(originalHandoff.savedBundle.bundlePath,{recursive:true,withFileTypes:true}), pngs=[];
        for(const entry of entries) if(entry.isFile()&&entry.name.toLowerCase().endsWith('.png')) {
          const imagePath=path.join(originalHandoff.savedBundle.bundlePath,entry.parentPath??'',entry.name), bytes=await readFile(imagePath);
          assert.equal(bytes.subarray(0,8).toString('hex'),'89504e470d0a1a0a');
          pngs.push({path:path.relative(originalHandoff.savedBundle.bundlePath,imagePath),sha256:fileHash(bytes),byteLength:bytes.byteLength,width:bytes.readUInt32BE(16),height:bytes.readUInt32BE(20)});
        }
        assert.ok(pngs.length>=2,'standard visual processing should publish selected frames and the requested contact sheet');
        await recordLog('Saved image inventory',{pngs});
        await disposeWorkflowPillHost(pillHost); pillHost=undefined; recordClient.detach(); recordClient=undefined; await coordinator.dispose(); coordinator=undefined;
        coordinator=await createStandardRecordCaptureCoordinator(coordinatorOptions); recordClient=createProvider('packed-standard-native-reopened-provider');
        await coordinator.reconcile();
        const reopenedFacts=await coordinator.captureFacts.readFacts(), reopenedSaved=reopenedFacts.captures.find(fact=>fact.captureId===requestId&&fact.phase==='saved');
        assert.deepEqual(reopenedSaved,saved,'restart preserves the same authenticated Saved fact');
        const reopenedHandoff=await coordinator.resolveHandoff({captureId:requestId,artifactId:saved.artifactId});
        assert.deepEqual(reopenedHandoff,originalHandoff,'same public request/artifact resolves identical handoff after restart');
        await recordLog('cold standard coordinator handoff',{requestId,artifactId:saved.artifactId,bundlePath:reopenedHandoff.savedBundle.bundlePath,
          processing:reopenedHandoff.processing,requestVideoProof:reopenedHandoff.requestVideoProof});
        console.log('packed Record standard coordinator native Xvfb qualification completed');
      } catch(error) { qualificationError=error; }
      finally {
        try { await cooperativeShutdown(shutdownRequest??{requestId:null,reason:'runner-finally'}); }
        catch(error) { cleanupErrors.push(error instanceof Error ? error : new Error(String(error))); }
        const cleanupSteps=[];
        // Keep the capture source alive when its owner is retained or its exit was not observed.
        const captureSourceCanRetire=!client&&(!captureOwnerCloseResult||captureOwnerCloseResult.state==='ended');
        if(captureSourceCanRetire) {
          if(scene) cleanupSteps.push({label:'synthetic scene exit',run:()=>waitForExit(scene,1000)});
          if(xvfb) cleanupSteps.push({label:'owned Xvfb exit',run:()=>waitForExit(xvfb,1000)});
        } else if(scene||xvfb) cleanupErrors.push(new Error('synthetic display retained with unresolved capture owner'));
        const displayCleanupErrors=await runCleanupSteps(cleanupSteps); cleanupErrors.push(...displayCleanupErrors); for(const failure of cleanupErrors) console.error(failure.stack);
        if(previousDisplay===undefined)delete process.env.DISPLAY;else process.env.DISPLAY=previousDisplay;
        if(previousWayland===undefined)delete process.env.WAYLAND_DISPLAY;else process.env.WAYLAND_DISPLAY=previousWayland;
        console.error('private standard qualification evidence retained: '+root);
      }
        if(qualificationError&&qualificationStatus===0) qualificationStatus=1;
        if(cleanupErrors.length) qualificationStatus=3;
        await recordLog('qualification terminal result',{status:qualificationStatus,error:qualificationError instanceof Error?qualificationError.stack:qualificationError,
          cleanupErrors:cleanupErrors.map(error=>error.message),authorityRoot:root});
        process.exitCode=qualificationStatus;
      }
    `;
    // The consumer scratch is removed in the outer finally; only an explicit
    // prepare-only invocation retains the generated script outside that scratch.
    const qualificationPacket = path.join(scratch,'qualification-script');
    await mkdir(qualificationPacket,{mode:0o700});
    const qualificationScript = path.join(qualificationPacket,'qualification.mjs');
    const qualificationHelper = path.join(root,'test/native-qualification/native-qualification.ts');
    await writeFile(qualificationScript,nativeScript,{mode:0o600});
    await checkGeneratedNodeScript(nativeScript);
    console.log('private generated native script packet: '+qualificationScript);
    if (process.env.APPARENT_CAPTURE2_PREPARE_ONLY === '1') {
      const output = process.env.APPARENT_CAPTURE2_PREPARATION_DIR;
      assert.ok(output && path.isAbsolute(output), 'APPARENT_CAPTURE2_PREPARATION_DIR must name an absolute private preparation directory');
      await mkdir(output,{recursive:true,mode:0o700}); await chmod(output,0o700);
      const frozenScript=path.join(output,'qualification.mjs');
      const frozenHelper=path.join(output,'native-qualification.ts');
      await copyFile(qualificationScript,frozenScript); await chmod(frozenScript,0o600);
      await copyFile(qualificationHelper,frozenHelper); await chmod(frozenHelper,0o600);
      const preflightScript=path.join(capture2,'qualification-preflight.mjs');
      await copyFile(frozenScript,preflightScript); await chmod(preflightScript,0o600);
      const preflight=await runQualification(process.execPath,['--experimental-strip-types',preflightScript],{cwd:capture2,timeout:180000,shutdownReportTimeout:5000,
        env:{...process.env,APPARENT_CAPTURE2_ARCHIVE:archive,APPARENT_MEDIA2_ARCHIVE:mediaArchive,APPARENT_CAPTURE2_SDK_TARBALL:qualificationSdk,
          APPARENT_QUALIFICATION_HELPERS:frozenHelper,APPARENT_QUALIFICATION_SCRIPT_PATH:preflightScript,APPARENT_QUALIFICATION_PREFLIGHT_ONLY:'1'}});
      process.stdout.write(preflight.stdout);
      const preflightLog=path.join(output,'archive-preflight.log');
      await writeFile(preflightLog,preflight.stdout+'\n--- stderr ---\n'+preflight.stderr,{mode:0o600});
        const base=(await run('git',['merge-base','HEAD','origin/main'],{cwd:root})).stdout.trim();
      const candidateHead=(await run('git',['rev-parse','HEAD'],{cwd:root})).stdout.trim();
      const runnerSha256=createHash('sha256').update(nativeScript).digest('hex');
      const helperSha256=createHash('sha256').update(await readFile(qualificationHelper)).digest('hex');
      const packet={schema:1,mode:'offline-preparation-only',base,candidateHead,archiveAdmissionPerformed:true,admissionRoutinePerformed:true,
        nativeQualificationRoutinePerformed:false,resourceAdmissionPerformed:false,
        archivePreflight:{performed:true,log:preflightLog,sha256:createHash('sha256').update(await readFile(preflightLog)).digest('hex')},
        archiveAdmissionMode:'generated-runner preflight; exact metadata/attestations/runtime closures checked; no native child launched',
        preparedAt:new Date().toISOString(),sdk:{path:qualificationSdk,version:'0.4.5',sha256:'177f4433964425b0a5d7e4f3228a4db5a47ebcf51deadc5783e7341652d441b2',
          acceptedPath:'/home/simon/.local/share/apparent-evidence/engine-sdk045-main-4119a08b/admitted-sdk/receipt.json',componentPath:sdkComponentPath,componentSha256:'579abad9ae046f9967e87f7a35cf2bb53523f9b72892ed93dfbe7e51a1a53d9d',componentBytes:62972,receiptPath:sdkReceiptPath,attestationPath:'/home/simon/.local/share/apparent-evidence/engine-sdk045-main-4119a08b/attestation.json',sourceCommit:'4119a08bf6d7057f2123f728553035681de00e06',artifactId:13558053710,verification:'promoted verified PR build; tested build source fa152688241998ff1c3d8b4c49095ea8d9df578e; not a fresh SDK build or whole-tree equality'},
        record:{path:qualificationRecordTarball,sha256:qualificationRecordSha256,componentPath:recordComponentPath,componentSha256:recordComponentHash,componentSize:327888,admissionPath:recordAdmissionPath,sourceCommit:'236a894dcfe6ff79f5def09204c0a8023db4fe90',runId:42328522610,artifactId:13556098088},
        capture:{path:archive,acceptedPath:'/home/simon/.local/share/apparent-evidence/engine-sdk045-main-4119a08b/capture-accepted.json',receiptPath:'/home/simon/.local/share/apparent-evidence/engine-sdk045-main-4119a08b/admitted-capture/receipt.json',attestationPath:'/home/simon/.local/share/apparent-evidence/engine-sdk045-main-4119a08b/capture-attestation.json',inventoryPath:'/home/simon/.local/share/apparent-evidence/engine-sdk045-main-4119a08b/capture-inventory-verification.json',sourceSha:'4119a08bf6d7057f2123f728553035681de00e06',runId:42333036882,artifactId:13558333683,sha256:'6a264d02d2647a21b96a8e83eeaab82af107753e18cd0c28ca4a7aceb5d67b89',size:11198744,runtimeFiles:54,entryPath:'/home/simon/.local/share/apparent-evidence/engine-sdk045-main-4119a08b/capture-extracted/stream-payload/payload/apparent-engine-capture-launch',launcherSha256:'fb2dfa941fcd7f9681a141b8213ecf6031eca82b424e608bbed3309d38b033e6',workerSha256:'c53e9ee6b850acd7750abf5576468bd6cb9b7c1e655fc64a45af12594cd8767b',recoveryChannel:'capture/2',shutdownTimeoutMs:40000,closeMaxMs:120000},
        media:{path:mediaArchive,acceptedPath:'/home/simon/.local/share/apparent-evidence/engine-media2-cleanup-main-b631acda/linux-x64-accepted.json',receiptPath:'/home/simon/.local/share/apparent-evidence/engine-media2-cleanup-main-b631acda/linux-x64/receipt.json',attestationPath:'/home/simon/.local/share/apparent-evidence/engine-media2-cleanup-main-b631acda/linux-x64/attestation.json',attestationBindingPath:'/home/simon/.local/share/apparent-evidence/engine-media2-cleanup-main-b631acda/linux-x64/attestation-binding.json',inventoryPath:'/home/simon/.local/share/apparent-evidence/engine-media2-cleanup-main-b631acda/linux-x64/inventory-verification.json',sourceSha:'b631acda0af12e30a45cf9885054882cee4da844',manifestSourceSha:'b631acda0af12e30a45cf9885054882cee4da844',runId:42323417518,artifactId:13554766965,sha256:'29c5db49ed046c3fb5802fae991027fbacfdd9dbf8df1c834b3a703c73efc730',size:100882653,runtimeFiles:186,recoveryChannel:'media/2',partialOutputCleanup:'admitted b631acda; recovery argv compatible'},
        runner:frozenScript,runnerSha256,helper:frozenHelper,helperSha256,syntaxChecked:true,sdk045LifecycleInputSatisfied:true,nativeExecuted:false,resourcesAllocated:false,consumerReviewed:false,nativeRunAuthorized:false,
        nextRun:{explicitOptIn:'APPARENT_CAPTURE2_QUALIFY=1',attempts:1,
          budget:{archiveAttestationAndClosureMs:120000,displayStartupMs:5000,foregroundObservationMs:150000,coordinatorDrainAndDisposeAllowanceMs:20000,publicationRecoveryReleaseMs:2000,mediaRecoveryReleaseMs:2000,owningCaptureCloseMaxMs:120000,sceneAndDisplayCleanupMs:4000,shutdownReportWindowMs:5000,supervisorDeadlineMs:423000,maximumSupervisorReturnMs:428000,unresolvedOwnerMayOutliveReturn:true},
          deadlineSemantics:'the 150 second observation cutoff is not finish/cancel intent; 423 seconds bounds supervisor waiting only, the 5 second report window bounds receipt, and unresolved native ownership is retained beyond that envelope without signaling or guaranteed cleanup',
          executionBlockedUntil:'independent consumer review and a separate explicit native attempt/resource authorization',
          display:'inspect existing process inventory, then owned synthetic Xvfb only',runtime:'one canonical private 0700 capture/media runtime',
          flow:'public Record pill start/finish -> standard coordinator -> authenticated Saved -> same-binding restart resolver',copy:'not exercised',
          endPolicy:'Only an explicit foreground finish/end/interrupt plus successful admission close, local drain, dispose and borrower drain permits closing the dedicated capture owner; SDK close ending is not terminal/Saved/runtime-clearance/material-release proof. Before finish, retain owner and authority, report status 3, and require an explicit operator end decision; never turn timeout into cancel or close.',
          recoveryPolicy:'Publication and media recovery peers are separately owned on the same runtime. Release after successful local drain/dispose even if runtime clearance is unknown. A recovery release never authorizes process-group termination. Direct release is permitted only for a peer definitely never injected into the coordinator.',
          preflight:'verify exact frozen runner/helper hashes, SDK/Record package seals, archive receipts/attestations and complete native closure before any separately authorized run',
          cleanup:'close external/provider admission; await coordinator shutdown and dispose; release eligible recovery connections; close the owning capture peer only under explicit intent and positive drain; otherwise retain root and owner without group signals',
          stopOn:'archive/attestation mismatch, non-native handshake, unknown media/publication result, failed drain or cleanup error; retain the private root and original tuples'}};
      await writeFile(path.join(output,'preparation.json'),JSON.stringify(packet,null,2)+'\n',{mode:0o600});
      console.log('offline qualification preparation retained: '+output+' (native execution skipped)');
    } else {
      const output=process.env.APPARENT_CAPTURE2_PREPARATION_DIR;
      assert.ok(output&&path.isAbsolute(output),'APPARENT_CAPTURE2_PREPARATION_DIR must identify the frozen qualification packet');
      const frozenScript=path.join(output,'qualification.mjs'), frozenHelper=path.join(output,'native-qualification.ts');
      const packet=JSON.parse(await readFile(path.join(output,'preparation.json'),'utf8'));
      assert.equal(packet.mode,'offline-preparation-only');
      assert.equal(createHash('sha256').update(await readFile(frozenScript)).digest('hex'),packet.runnerSha256,'frozen qualification runner changed');
      assert.equal(createHash('sha256').update(await readFile(frozenHelper)).digest('hex'),packet.helperSha256,'frozen supervisor helper changed');
      assert.equal(createHash('sha256').update(nativeScript).digest('hex'),packet.runnerSha256,'generated qualification source differs from frozen runner; prepare a new packet');
      assert.equal(createHash('sha256').update(await readFile(qualificationHelper)).digest('hex'),packet.helperSha256,'repository supervisor helper differs from frozen helper; prepare a new packet');
      const executionScript=path.join(capture2,'qualification.mjs');
      await copyFile(frozenScript,executionScript); await chmod(executionScript,0o600);
      assert.equal(packet.sdk045LifecycleInputSatisfied,true,'SDK 0.4.5 owner-close API is the frozen input');
      assert.equal(packet.consumerReviewed,false,'preparation cannot claim independent consumer review');
      assert.equal(packet.nativeRunAuthorized,false,'preparation cannot authorize native execution');
      assertQualificationShutdownProtocol(true);
      assertNativeQualificationAdmission(false);
      const capture2Result = await runQualification(process.execPath, ['--experimental-strip-types', executionScript], { cwd: capture2, timeout: 423000, shutdownReportTimeout:5000, env: { ...process.env, APPARENT_CAPTURE2_ARCHIVE: archive, APPARENT_MEDIA2_ARCHIVE: mediaArchive, APPARENT_CAPTURE2_SDK_TARBALL: qualificationSdk, APPARENT_QUALIFICATION_HELPERS: frozenHelper, APPARENT_QUALIFICATION_SCRIPT_PATH: executionScript } });
      process.stdout.write(capture2Result.stdout);
    }
  }
  const sdkDeclarations = path.join(publication, 'consumer.ts');
  await writeFile(sdkDeclarations, `
    import {preflightCapturePublication, type CaptureClient, type SourceDescriptor, type CaptureOptions, type InvocationFact} from '@apparent/engine-sdk/capture';
    import {type Derivative2DocumentInput, type Derivative2Selector} from '@apparent/engine-sdk/media2';
    import {createCapture2Host, type Capture2Host, type Capture2StartIntent} from '@apparent/record/host/capture2';
    import {createCompletedCapturePublisher, createRecordPublicationOutput, createRecordPublicationRecovery, prepareRecordPublication, type CompletedCapturePublisher, type PublishCompletedCaptureInput, type ReconcileCompletedCaptureInput, type RecordPublicationOutput} from '@apparent/record/publication';
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
    const output: RecordPublicationOutput = createRecordPublicationOutput(client, '/private/control', preflightCapturePublication);
    const factsPromise: ReturnType<RecordPublicationOutput['captureFacts']['readFacts']> = output.captureFacts.readFacts();
    const resolver: RecordPublicationOutput['resolveHandoff'] = output.resolveHandoff;
    const handoffPromise: ReturnType<RecordPublicationOutput['resolveHandoff']> = resolver({captureId:'public-request', artifactId:'bundle'});
    const publish: (input: PublishCompletedCaptureInput) => ReturnType<CompletedCapturePublisher['publishCompletedCapture']> = publisher.publishCompletedCapture;
    const reconcile: (input: ReconcileCompletedCaptureInput) => ReturnType<CompletedCapturePublisher['reconcile']> = publisher.reconcile;
    declare const documents: readonly Derivative2DocumentInput[];
    declare const selector: Derivative2Selector;
    const media2Input: PublishCompletedCaptureInput = { status: {} as never, assembly: {} as never, ownerToken: 'a'.repeat(64), publicationId: 'b'.repeat(32), stagingRoot: '/private/staging', destinationPath: '/bundle', derivatives2: documents, derivative2Artifacts: {'frames/renamed.png': {receiptId:'receipt', selector}} };
    void result; void stage; void publish; void reconcile; void capture; void exactReceipt; void media2Input; void factsPromise; void handoffPromise;
  `);
  await run(process.execPath, [tsc, '--noEmit', '--strict', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', sdkDeclarations], { cwd: publication });
} finally {
  await rm(scratch, { recursive: true, force: true });
}
