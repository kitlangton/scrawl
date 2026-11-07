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
    import { createRecordNativePort, type NativeHostPortInput } from '@apparent/record/host/native';
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
    const qualificationSdk = process.env.APPARENT_CAPTURE2_SDK_TARBALL;
    assert.ok(qualificationSdk && path.isAbsolute(qualificationSdk), 'APPARENT_CAPTURE2_SDK_TARBALL must name the admitted SDK 0.4.4 tarball');
    assert.equal(createHash('sha256').update(await readFile(qualificationSdk)).digest('hex'), '62e188a118261b73097372000d5ae4fe0f0e72ca2ad710f0714002f276856172', 'qualification requires the admitted SDK 0.4.4 package');
    const capture2 = path.join(scratch, 'capture2');
    await initConsumer(capture2);
    await run('npm', ['install', '--no-audit', '--no-fund', tarball, qualificationSdk, 'react@19.2.0', 'react-dom@19.2.0'], { cwd: capture2 });
    const qualificationTypes = path.join(capture2, 'consumer.ts');
    await writeFile(qualificationTypes, `
      import { capture2MaterialsRoot } from '@apparent/engine-sdk/capture';
      import { createStandardRecordCaptureCoordinator, type StandardRecordCaptureCoordinatorOptions, type RecordCaptureCoordinator } from '@apparent/record/host/capture2-coordinator';
      declare const options: StandardRecordCaptureCoordinatorOptions;
      const coordinator: Promise<RecordCaptureCoordinator> = createStandardRecordCaptureCoordinator(options);
      const trustedRoot: string = capture2MaterialsRoot('/private/runtime', 'capture-id');
      void [coordinator, trustedRoot];
    `);
    const qualificationTsc = path.join(root, 'node_modules/typescript/bin/tsc');
    await run(process.execPath, [qualificationTsc, '--noEmit', '--strict', '--skipLibCheck', '--types', 'node', '--typeRoots', path.join(root, 'node_modules/@types'), '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', qualificationTypes], { cwd: capture2 });
    const qualificationRuntime = await run(process.execPath, ['--input-type=module', '-e', `
      import assert from 'node:assert/strict';
      import { capture2MaterialsRoot } from '@apparent/engine-sdk/capture';
      import { createStandardRecordCaptureCoordinator } from '@apparent/record/host/capture2-coordinator';
      assert.equal(typeof capture2MaterialsRoot, 'function');
      assert.equal(typeof createStandardRecordCaptureCoordinator, 'function');
      console.log('packed Record standard coordinator and SDK 0.4.4 root consumer passed');
    `], { cwd: capture2 });
    process.stdout.write(qualificationRuntime.stdout);
    const nativeScript = `
      import assert from 'node:assert/strict';
      import {createHash} from 'node:crypto';
      import {spawn} from 'node:child_process';
      import {appendFile, chmod, copyFile, lstat, mkdir, mkdtemp, readFile, readdir, realpath, writeFile} from 'node:fs/promises';
      import {tmpdir} from 'node:os';
      import path from 'node:path';
      import {pathToFileURL} from 'node:url';
      import {CaptureClient, capture2MaterialsRoot, launchCaptureEngine, preflightCapturePublication} from '@apparent/engine-sdk/capture';
      import {Media2Client, launchMedia2Worker} from '@apparent/engine-sdk/media2';
      import {createStandardRecordCaptureCoordinator} from '@apparent/record/host/capture2-coordinator';
      import {createRecordClientPillHost,disposeWorkflowPillHost} from '@apparent/record/ui';
      import {AbsentDestinationAllocator, SqliteCaptureMappingStore} from '@apparent/record/host';
      const {runBounded,runCleanupSteps,trackChild,waitForExit} = await import(pathToFileURL(process.env.APPARENT_QUALIFICATION_HELPERS).href);

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
      assert.equal(fileHash(await readFile(sdkTarball)), '62e188a118261b73097372000d5ae4fe0f0e72ca2ad710f0714002f276856172');
      const inputEvidence = path.join(root, 'native-inputs'); await mkdir(inputEvidence,{mode:0o700});
      await copyFile(sdkTarball,path.join(inputEvidence,'apparent-engine-sdk-0.4.4.tgz'));
      await chmod(path.join(inputEvidence,'apparent-engine-sdk-0.4.4.tgz'),0o600);
      async function admitArtifact(kind, archivePath, expected) {
        const evidence=path.dirname(archivePath), evidenceRoot=path.dirname(evidence), archiveBytes=await readFile(archivePath);
        assert.equal(archiveBytes.byteLength,expected.size,kind+' component size'); assert.equal(fileHash(archiveBytes),expected.sha256,kind+' component SHA-256');
        const receipt=JSON.parse(await readFile(path.join(evidence,'receipt.json'),'utf8'));
        const accepted=JSON.parse(await readFile(path.join(evidenceRoot,expected.acceptedFile),'utf8'));
        const inventoryDocument=JSON.parse(await readFile(path.join(evidenceRoot,'inventory-verification.json'),'utf8'));
        const inventoryRecord=Array.isArray(inventoryDocument)?inventoryDocument.find(item=>item.receipt?.sha256===expected.sha256):inventoryDocument;
        const inventoryAdmission=Array.isArray(inventoryDocument)?inventoryRecord?.admission:inventoryDocument[kind];
        const inventoryReceipt=Array.isArray(inventoryDocument)?inventoryRecord?.receipt:inventoryDocument[kind+'Receipt'];
        const fileCount=Array.isArray(inventoryDocument)?inventoryRecord?.runtimeFileCount:inventoryDocument.runtimeFiles?.length;
        const attestation=JSON.parse(await readFile(path.join(evidence,'attestation.json'),'utf8'));
        assert.equal(receipt.sourceSha,expected.sourceSha); assert.equal(receipt.sha256,expected.sha256); assert.equal(receipt.size,expected.size); assert.equal(receipt.runId,String(expected.runId));
        assert.equal(accepted.sourceSha,expected.sourceSha); assert.equal(accepted.runId,expected.runId); assert.equal(accepted.componentSha256,expected.sha256);
        assert.equal(inventoryAdmission.componentSha256,expected.sha256); assert.equal(inventoryAdmission.sourceSha,expected.sourceSha); assert.equal(inventoryAdmission.runId,expected.runId);
        assert.equal(inventoryReceipt.sha256,expected.sha256); assert.equal(Number(inventoryReceipt.size),expected.size); assert.equal(fileCount,expected.files);
        const bundle=attestation[0]?.attestation?.bundle; assert.ok(bundle,'signed source attestation bundle exists');
        const statement=JSON.parse(Buffer.from(bundle.dsseEnvelope.payload,'base64').toString('utf8'));
        assert.equal(statement.subject[0].digest.sha256,expected.sha256);
        assert.equal(statement.predicate.buildDefinition.resolvedDependencies[0].digest.gitCommit,expected.sourceSha);
        assert.equal(statement.predicate.buildDefinition.externalParameters.workflow.path,'.github/workflows/stream-engine.yml');
        const bundlePath=path.join(inputEvidence,kind+'.attestation.bundle.json');
        await writeFile(bundlePath,JSON.stringify(bundle),{mode:0o600});
        const verification=await runBounded('gh',['attestation','verify',archivePath,'--bundle',bundlePath,'--repo','tryapparent/apparent','--signer-workflow','tryapparent/apparent/.github/workflows/stream-engine.yml','--source-digest',expected.sourceSha,'--source-ref','refs/heads/main'],30000);
        await writeFile(path.join(inputEvidence,kind+'.attestation-verification.log'),verification.stdout+verification.stderr,{mode:0o600});
        for(const name of ['receipt.json','attestation.json']) { const retained=path.join(inputEvidence,kind+'-'+name); await copyFile(path.join(evidence,name),retained); await chmod(retained,0o600); }
        await copyFile(path.join(evidenceRoot,expected.acceptedFile),path.join(inputEvidence,kind+'-'+expected.acceptedFile));
        await chmod(path.join(inputEvidence,kind+'-'+expected.acceptedFile),0o600);
        const retainedInventory=path.join(inputEvidence,kind+'-inventory-verification.json');
        await copyFile(path.join(evidenceRoot,'inventory-verification.json'),retainedInventory); await chmod(retainedInventory,0o600);
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
          assert.deepEqual(manifest.media2Launch.argv,['--media-runtime-directory','<capture-runtime-dir>','--media-channel','media','--media-protocol-version','2']);
          assert.equal(manifest.media2Launch.sharesCaptureRuntime,true);
          assert.equal(manifest.visualObservations?.schema,'apparent.frame-observation-profile/1'); assert.equal(manifest.visualObservations?.chunkType,'apOB');
          assert.equal(manifest.launch.entry,'apparent-engine-frames'); assert.equal(manifest.launch.pathLookup,false); assert.equal(manifest.launch.runtimeBuilds,false);
          assert.equal(listed.get('apparent-engine-frames').sha256,'cf86ab5841ea5a847844518bc3e4f0bd94174f23ac86defba65dd2d8493c27e9');
          assert.equal(listed.get('libexec/apparent-engine-frames').sha256,'62095759727d826073f67402942d7826185f6590dbae08be2fee3d501f3d3343');
        } else {
          assert.ok(manifest.capabilities.includes('capture/2')); assert.ok(manifest.capabilities.includes('authenticated-publication'));
          assert.deepEqual(manifest.launch.argv,['--runtime-directory','<private-runtime-dir>','--capture-channel','capture','--capture-protocol-version','2']);
          assert.equal(manifest.launch.entry,'apparent-engine-capture-launch'); assert.equal(manifest.launch.worker,'libexec/apparent-engine-capture');
          assert.equal(manifest.launch.pathLookup,false); assert.equal(manifest.launch.runtimeBuilds,false);
          assert.equal(listed.get('apparent-engine-capture-launch').sha256,'fb2dfa941fcd7f9681a141b8213ecf6031eca82b424e608bbed3309d38b033e6');
          assert.equal(listed.get('libexec/apparent-engine-capture').sha256,'63f02b819523367da1996d1bb3760118b8a34f840de2458b42996eda85ff822b');
        }
        await recordLog(kind+' admitted archive',{path:archivePath,sha256:expected.sha256,size:expected.size,sourceSha:expected.sourceSha,manifestSourceSha:expected.manifestSourceSha,runtimeFiles:seen.size,payload});
        return {payload,manifest};
      }
      const captureArtifact=await admitArtifact('capture',archive,{acceptedFile:'capture-accepted.json',sourceSha:'c18340f1886d0b796717bee91333fc1c8533b993',manifestSourceSha:'c18340f1886d0b796717bee91333fc1c8533b993',runId:42319740724,sha256:'00577928145323f8c964dda8cc1e47668d1f58b7d571ec9f7fb896aadc277708',size:11197126,files:54});
      const mediaArtifact=await admitArtifact('media',mediaArchive,{acceptedFile:'linux-accepted.json',sourceSha:'9915e8d343a4feafbeb31b4cea6bed43f2cb93a1',manifestSourceSha:'cf8356eb869038f6737a8e83c1cb4ab655fa0237',runId:42314800490,sha256:'cd7346951646f440c0d67e1fe10a8bc44dfde4329e3a9a3cc7d6f0d4297604f4',size:100858133,files:186});
      const capturePayload=captureArtifact.payload, captureManifest=captureArtifact.manifest, mediaPayload=mediaArtifact.payload, mediaManifest=mediaArtifact.manifest;
      let xvfb, display='', scene, client, mediaClient, coordinator, recordClient, pillHost, mappings;
      let qualificationError, cleanupErrors=[], owningClientsClear=false;
      const previousDisplay=process.env.DISPLAY, previousWayland=process.env.WAYLAND_DISPLAY;
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
        client=new CaptureClient(launchCaptureEngine(path.join(capturePayload,captureManifest.launch.entry),runtime,40000));
        const mediaEntry=mediaManifest.runtimeFiles.find(file=>file.path==='apparent-engine-frames'); assert.ok(mediaEntry);
        mediaClient=new Media2Client(launchMedia2Worker(path.join(mediaPayload,mediaEntry.path),runtime,40000));
        const captureHandshake=await client.handshake(5000); assert.equal(captureHandshake.implementation,'native');
        const mediaHandshake=await mediaClient.handshake(5000); assert.equal(mediaHandshake.implementation,'native');
        mappings=new SqliteCaptureMappingStore(control); const destinations=new AbsentDestinationAllocator(control,bundles);
        const options={audio:'none',localOnly:true}, invocation={kind:'explicit',invocationId:'a'.repeat(32),observedAtMs:Date.now()};
        const requestId='packed-standard-native-'+Date.now();
        const createStartIntent=(id,fresh)=>{assert.equal(fresh.state,'available');
          const source=fresh.sources.find(item=>item.kind==='display'&&item.bounds.width===683&&item.bounds.height===480);
          assert.ok(source,'fresh discovery must contain the owned 683x480 synthetic display');
          const digest=createHash('sha256').update(id,'utf8').digest('hex');
          return {source,options,invocation:{...invocation,observedAtMs:Date.now()},destinationPath:path.join(bundles,'record-'+digest+'.apparent')};};
        const coordinatorOptions={captureClient:client,mediaClient,mappings,destinations,privateControlRoot:control,createStartIntent,
          trustedMaterialsRoot:mapping=>capture2MaterialsRoot(runtime,mapping.captureId),preflight:preflightCapturePublication,
          pollIntervalMs:200,operationTimeoutMs:10000,standardProcessing:{requestVideoProof:true}};
        coordinator=await createStandardRecordCaptureCoordinator(coordinatorOptions);
        const createProvider=providerEpoch=>coordinator.createProvider({providerEpoch,native:{readFacts:async()=>({version:1,providerEpoch:providerEpoch+'-facts',revision:1,readiness:[],shortcut:{effective:null,label:null,registration:'unknown',externallyManaged:false,draft:null}})}});
        recordClient=createProvider('packed-standard-native-provider');
        if(sceneSpawnError) throw sceneSpawnError;
        const deadline=Date.now()+150000, waitFor=async(fn,label)=>{while(Date.now()<deadline){const value=await fn();if(value)return value;await new Promise(r=>setTimeout(r,250));}throw new Error('qualification deadline waiting for '+label);};
        const before=recordClient.current();
        const localOnly=await recordClient.execute({type:'start-capture',delivery:'local-only',requestId:'packed-local-only-'+Date.now(),expected:{providerEpoch:before.providerEpoch,revision:before.revision}});
        assert.equal(localOnly.status,'refused','native delivery prohibition is not proof of whole-pipeline local-only enforcement');
        let actionIds=0; pillHost=createRecordClientPillHost(recordClient,{createRequestId:()=>actionIds++===0?requestId:'packed-pill-action-'+actionIds});
        const initialPill=await pillHost.read(); assert.ok(initialPill.actions.some(action=>action.id==='record'&&action.enabled));
        await pillHost.act(initialPill.revision,'record');
        await waitFor(async()=>recordClient.current().captures.find(item=>item.captureId===requestId&&item.phase==='recording'),'public provider recording fact');
        await new Promise(resolve=>setTimeout(resolve,1800));
        const mapping=await mappings.getByRequestId(requestId); assert.ok(mapping?.capture2Intent);
        assert.equal(mapping.bundlePath,path.join(bundles,path.basename(mapping.bundlePath)));
        await assert.rejects(lstat(mapping.bundlePath),error=>error.code==='ENOENT','destination stays absent before authenticated publication');
        const recordingPill=await pillHost.read(); assert.equal(recordingPill.phase,'recording'); await pillHost.act(recordingPill.revision,'finish');
        const saved=await waitFor(async()=>{await coordinator.reconcile();const facts=await coordinator.captureFacts.readFacts();return facts.captures.find(fact=>fact.captureId===requestId&&fact.phase==='saved');},'authenticated Record Saved output');
        assert.equal(saved.delivery,'unknown','nativeRemoteDelivery forbidden does not imply local-only delivery'); assert.equal(saved.artifactId,mapping.bundleId);
        const originalHandoff=await coordinator.resolveHandoff({captureId:requestId,artifactId:saved.artifactId});
        assert.ok(originalHandoff?.savedBundle.status==='saved','exact public request/artifact binding resolves after authenticated Saved');
        assert.equal(originalHandoff.savedBundle.bundlePath,mapping.bundlePath);
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
        const cleanupSteps=[];
        if(pillHost) cleanupSteps.push({label:'pill host disposal',run:()=>disposeWorkflowPillHost(pillHost)});
        if(recordClient) cleanupSteps.push({label:'Record client detach',run:()=>recordClient.detach()});
        if(coordinator) cleanupSteps.push({label:'coordinator drain and disposal',run:async()=>{const report=await coordinator.shutdown();await recordLog('coordinator shutdown report',report);await coordinator.dispose();owningClientsClear=report.scope==='record-coordinator'&&report.admission==='closed'&&report.localDrain==='complete'&&report.runtimeClearance==='clear';coordinator=undefined;if(!owningClientsClear)throw new Error('Owning clients must remain retained: coordinator clearance is unresolved');}});
        if(mediaClient) cleanupSteps.push({label:'media/2 client close',run:async()=>{if(!owningClientsClear)throw new Error('Media owning close refused without positive clearance');await mediaClient.close();mediaClient=undefined;}});
        if(client) cleanupSteps.push({label:'capture client close',run:async()=>{if(!owningClientsClear)throw new Error('Capture owning close refused without positive clearance');await client.close();}});
        if(scene) cleanupSteps.push({label:'synthetic scene exit',run:()=>waitForExit(scene,1000)});
        if(xvfb) cleanupSteps.push({label:'owned Xvfb exit',run:()=>waitForExit(xvfb,1000)});
        cleanupErrors=await runCleanupSteps(cleanupSteps); for(const failure of cleanupErrors) console.error(failure.stack);
        if(previousDisplay===undefined)delete process.env.DISPLAY;else process.env.DISPLAY=previousDisplay;
        if(previousWayland===undefined)delete process.env.WAYLAND_DISPLAY;else process.env.WAYLAND_DISPLAY=previousWayland;
        console.error('private standard qualification evidence retained: '+root);
      }
      if(qualificationError||cleanupErrors.length) throw new AggregateError([...(qualificationError?[qualificationError]:[]),...cleanupErrors],'standard native qualification failed; inspect retained evidence and cleanup report',{cause:qualificationError});
    `;
    // The consumer scratch is removed in the outer finally; only an explicit
    // prepare-only invocation retains the generated script outside that scratch.
    const qualificationPacket = path.join(scratch,'qualification-script');
    await mkdir(qualificationPacket,{mode:0o700});
    const qualificationScript = path.join(qualificationPacket,'qualification.mjs');
    await writeFile(qualificationScript,nativeScript,{mode:0o600});
    await checkGeneratedNodeScript(nativeScript);
    console.log('private generated native script packet: '+qualificationScript);
    if (process.env.APPARENT_CAPTURE2_PREPARE_ONLY === '1') {
      const output = process.env.APPARENT_CAPTURE2_PREPARATION_DIR;
      assert.ok(output && path.isAbsolute(output), 'APPARENT_CAPTURE2_PREPARATION_DIR must name an absolute private preparation directory');
      await mkdir(output,{recursive:true,mode:0o700}); await chmod(output,0o700);
      const frozenScript=path.join(output,'qualification.mjs');
      await copyFile(qualificationScript,frozenScript); await chmod(frozenScript,0o600);
      const base=(await run('git',['merge-base','HEAD','origin/main'],{cwd:root})).stdout.trim();
      const candidateHead=(await run('git',['rev-parse','HEAD'],{cwd:root})).stdout.trim();
      const packet={schema:1,mode:'offline-preparation-only',base,candidateHead,
        preparedAt:new Date().toISOString(),sdk:{path:qualificationSdk,version:'0.4.4',sha256:'62e188a118261b73097372000d5ae4fe0f0e72ca2ad710f0714002f276856172',
          acceptedPath:'/home/simon/.local/share/apparent-evidence/engine-materials-root-main-c18340f1/sdk-accepted.json',receiptPath:'/home/simon/.local/share/apparent-evidence/engine-materials-root-main-c18340f1/sdk/receipt.json',attestationPath:'/home/simon/.local/share/apparent-evidence/engine-materials-root-main-c18340f1/sdk/attestation.json'},
        capture:{path:archive,acceptedPath:path.join(path.dirname(path.dirname(archive)),'capture-accepted.json'),receiptPath:path.join(path.dirname(archive),'receipt.json'),attestationPath:path.join(path.dirname(archive),'attestation.json'),inventoryPath:path.join(path.dirname(path.dirname(archive)),'inventory-verification.json'),sourceSha:'c18340f1886d0b796717bee91333fc1c8533b993',runId:42319740724,artifactId:13552701819,sha256:'00577928145323f8c964dda8cc1e47668d1f58b7d571ec9f7fb896aadc277708',runtimeFiles:54},
        media:{path:mediaArchive,acceptedPath:path.join(path.dirname(path.dirname(mediaArchive)),'linux-accepted.json'),receiptPath:path.join(path.dirname(mediaArchive),'receipt.json'),attestationPath:path.join(path.dirname(mediaArchive),'attestation.json'),inventoryPath:path.join(path.dirname(path.dirname(mediaArchive)),'inventory-verification.json'),sourceSha:'9915e8d343a4feafbeb31b4cea6bed43f2cb93a1',manifestSourceSha:'cf8356eb869038f6737a8e83c1cb4ab655fa0237',runId:42314800490,artifactId:13550580801,sha256:'cd7346951646f440c0d67e1fe10a8bc44dfde4329e3a9a3cc7d6f0d4297604f4',runtimeFiles:186},
        runner:frozenScript,runnerSha256:createHash('sha256').update(nativeScript).digest('hex'),syntaxChecked:true,nativeExecuted:false,resourcesAllocated:false,
        nextRun:{explicitOptIn:'APPARENT_CAPTURE2_QUALIFY=1',attempts:1,deadlineMs:240000,deadlineSemantics:'observation budget only; it is not capture cancel/end intent and does not guarantee bounded cleanup of an unresolved owning peer',
          executionBlockedUntil:'a child-to-parent coordinator shutdown/drain and positive-clearance protocol plus safe SDK owning-handle release are available',
          display:'inspect existing process inventory, then owned synthetic Xvfb only',runtime:'one canonical private 0700 capture/media runtime',
          flow:'public Record pill start/finish -> standard coordinator -> authenticated Saved -> same-binding restart resolver',copy:'not exercised',
          preflight:'verify current Xvfb/process inventory and all frozen artifacts before a separately authorized run',
          cleanup:'request coordinator admission closure and local drain; close owning clients only after closed admission, complete drain, clear runtime ownership and disposal; if unresolved, preserve private authority/runtime and peers without process-group signals',
          stopOn:'archive or attestation mismatch, non-native handshake, unknown media/publication result, cleanup error; retain the private root'}};
      await writeFile(path.join(output,'preparation.json'),JSON.stringify(packet,null,2)+'\n',{mode:0o600});
      console.log('offline qualification preparation retained: '+output+' (native execution skipped)');
    } else {
      // SDK 0.4.4 exposes recovery release, not safe release of an unresolved
      // owning capture/media handle. Do not start native peers until the
      // supervisor can request coordinator admission closure/drain and observe
      // positive clearance before any process-group cleanup.
      assertQualificationShutdownProtocol(false);
      const capture2Result = await runQualification(process.execPath, ['--experimental-strip-types', '--input-type=module', '-e', nativeScript], { cwd: capture2, timeout: 240000, env: { ...process.env, APPARENT_CAPTURE2_ARCHIVE: archive, APPARENT_MEDIA2_ARCHIVE: mediaArchive, APPARENT_CAPTURE2_SDK_TARBALL: qualificationSdk, APPARENT_QUALIFICATION_HELPERS: path.join(root, 'test/native-qualification/native-qualification.ts'), APPARENT_QUALIFICATION_SCRIPT_PATH: qualificationScript } });
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
