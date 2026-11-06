/** Verify the packed artifact in consumers that cannot see repository siblings. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmod, mkdtemp, mkdir, readFile, writeFile, copyFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { checkGeneratedNodeScript, runQualification } from './native-qualification/native-qualification.ts';

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
    import { createRecordCapabilityProvider, hasConfirmedArtifactOpen, type DurableNoteSaveOutcome, type ScreenStrokeCursor, type ShortcutTrialFact } from '@apparent/record/capabilities';
    import { createRecordNativePort, type NativeHostPortInput } from '@apparent/record/host/native';
    import { createRecordFixture } from '@apparent/record/fixtures';
    declare const cursor: ScreenStrokeCursor;
    declare const trial: ShortcutTrialFact;
    declare const note: DurableNoteSaveOutcome;
    void [cursor, trial, note];
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
    console.log('headless packed runtime and declaration consumer passed');
  `], { cwd: headless });
  process.stdout.write(runtime.stdout);

  // The canonical bundle subpath uses only the packed public SDK contract.
  const sdkArtifact = process.env.APPARENT_ENGINE_SDK_TARBALL ?? path.join(root, 'vendor/engine-sdk-0.4.1.tgz');
  const sdkBytes = await readFile(sdkArtifact);
  assert.equal(createHash('sha256').update(sdkBytes).digest('hex'), 'f0c4b9777033bcde0463e0442074e5af714cddaed1c45452802012e66d21c156');
  await run('npm', ['install', '--no-audit', '--no-fund', tarball, sdkArtifact], { cwd: bundle });
  const bundleDeclarations = path.join(bundle, 'consumer.ts');
  await writeFile(bundleDeclarations, `
    import { RecordMedia2Job, type RecordMedia2JobState } from '@apparent/record/processing/media2-consumer';
    import { readRetainedDerivativeArtifacts, type ReadRetainedDerivativeArtifactsInput } from '@apparent/record/processing/retained-artifacts';
    import { Media2Client, preflightMedia2Dispatch, type Media2JobInput } from '@apparent/engine-sdk/media2';
    import { assembleCanonicalBundle, type BundleAssemblyInput, type BundleValidators, type CanonicalStroke } from '@apparent/record/processing/bundle';
    import type { PreparePublicationInput } from '@apparent/record/publication';
    import { validateMaterialsDocument, validateFactFile, type MaterialsDocument, type ContextFacts, type InteractionFacts, type TranscriptFacts, type StrokeFacts, type StrokeRgb } from '@apparent/engine-sdk/capture';
    import {readCaptureMaterials, type CaptureMaterialsInput} from '@apparent/record/host/materials';
    import {createCapture2RecordRuntime, type Capture2RecordRuntimeOptions} from '@apparent/record/host/capture2-runtime';
    import {createRecordCaptureCoordinator, type RecordCaptureCoordinator, type RecordCaptureCoordinatorShutdownReport, type RecordCaptureProcessingSelection} from '@apparent/record/host/capture2-coordinator';
    declare const capture2RuntimeOptions: Capture2RecordRuntimeOptions;
    void createCapture2RecordRuntime(capture2RuntimeOptions);
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
    void [RecordMedia2Job, Media2Client, preflightMedia2Dispatch, media2Input, media2State, createMedia2Job, consumeRetainedArtifacts, createRecordCaptureCoordinator, factsPort, handoffResolver, processingSelection];
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
    const {readCaptureMaterials} = await import('@apparent/record/host/materials');
    assert.equal(typeof readCaptureMaterials, 'function');
    assert.equal(typeof assembleCanonicalBundle, 'function');
    assert.equal(typeof validateMaterialsDocument, 'function');
    assert.equal(typeof validateFactFile, 'function');
    for (const name of ['react','react-dom','electron']) assert.throws(()=>createRequire(import.meta.url).resolve(name));
    assert.deepEqual([RecordMedia2Job, readRetainedDerivativeArtifacts, Media2Client, preflightMedia2Dispatch].map(value => typeof value), ['function','function','function','function']);
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
  const sdkTarball = path.join(scratch, 'engine-sdk-0.4.1.tgz'); await copyFile(sdkArtifact, sdkTarball);
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
    import {createRecordCaptureCoordinator} from '@apparent/record/host/capture2-coordinator';
    const client = new CaptureClient(new ReferenceCapturePeer(), {allowTestDouble:true});
    assert.equal((await client.handshake()).readiness, 'unavailable');
    const recovery = createRecordPublicationRecovery(client, process.cwd() + '/unused-private-root', preflightCapturePublication);
    assert.equal(typeof recovery.publish, 'function');
    assert.equal(typeof createCapture2Host, 'function');
    assert.equal(typeof createCapture2RecordRuntime, 'function');
    assert.equal(typeof createRecordCaptureCoordinator, 'function');
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

  // The bounded Linux native qualification is opt-in. It installs the packed
  // Record artifact and published SDK into a separate consumer, verifies and
  // extracts the admitted capture/media artifacts into one private proof root,
  // then exercises capture/2 and media/2 on owned synthetic Xvfb.
  if (process.env.APPARENT_CAPTURE2_QUALIFY === '1') {
    const archive = process.env.APPARENT_CAPTURE2_ARCHIVE;
    const mediaArchive = process.env.APPARENT_MEDIA2_ARCHIVE;
    assert.ok(archive && path.isAbsolute(archive), 'APPARENT_CAPTURE2_ARCHIVE must name the admitted archive');
    assert.ok(mediaArchive && path.isAbsolute(mediaArchive), 'APPARENT_MEDIA2_ARCHIVE must name the admitted media archive');
    const capture2 = path.join(scratch, 'capture2');
    await initConsumer(capture2);
    await run('npm', ['install', '--no-audit', '--no-fund', tarball, sdkArtifact, 'react@19.2.0', 'react-dom@19.2.0'], { cwd: capture2 });
    const nativeScript = `
      import assert from 'node:assert/strict';
      import {createHash} from 'node:crypto';
      import {spawn} from 'node:child_process';
      import {appendFile, chmod, copyFile, lstat, mkdir, mkdtemp, readFile, readdir, realpath, rm, stat, writeFile} from 'node:fs/promises';
      import {tmpdir} from 'node:os';
      import path from 'node:path';
      import {pathToFileURL} from 'node:url';
      import {CaptureClient, launchCaptureEngine} from '@apparent/engine-sdk/capture';
      import {Media2Client, launchMedia2Worker, prepareMedia2Request, encodeContactSheetLayout} from '@apparent/engine-sdk/media2';
      import {createCapture2Host} from '@apparent/record/host/capture2';
      import {createCapture2RecordRuntime} from '@apparent/record/host/capture2-runtime';
      import {createRecordCapabilityProvider} from '@apparent/record/capabilities';
      import {createRecordClientPillHost,disposeWorkflowPillHost} from '@apparent/record/ui';
      import {AbsentDestinationAllocator, SqliteCaptureMappingStore} from '@apparent/record/host';
      import {readCaptureMaterials} from '@apparent/record/host/materials';
      import {RecordMedia2Job} from '@apparent/record/processing/media2-consumer';
      import {readRetainedDerivativeArtifacts} from '@apparent/record/processing/retained-artifacts';
      import {assembleCanonicalBundle} from '@apparent/record/processing/bundle';
      import {prepareRecordPublication} from '@apparent/record/publication';
      import {preflightCapturePublication} from '@apparent/engine-sdk/capture';
      const {reconcileAndCancel,runBounded,runCleanupSteps,trackChild,waitForExit} = await import(pathToFileURL(process.env.APPARENT_QUALIFICATION_HELPERS).href);

      const archive = path.resolve(process.env.APPARENT_CAPTURE2_ARCHIVE);
      const root = await realpath(await mkdtemp(path.join(tmpdir(), 'record-capture2-native-')));
      await chmod(root, 0o700);
      console.log('private native proof root: ' + root);
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
      const sourceSha = '0570f77576be2f2b552710a3710cd345444cfdce';
      const acceptance = JSON.parse(await readFile(path.join(path.dirname(path.dirname(archive)), 'acceptance.json'), 'utf8'));
      assert.equal(acceptance.sourceSha, sourceSha); assert.equal(acceptance.runId, '42252644242'); assert.equal(acceptance.conclusion, 'success'); assert.equal(acceptance.fresh, true);
      const inputEvidence = path.join(root, 'native-inputs'); await mkdir(inputEvidence,{mode:0o700});
      await copyFile(path.join(path.dirname(path.dirname(archive)),'acceptance.json'),path.join(inputEvidence,'acceptance.json'));
      await chmod(path.join(inputEvidence,'acceptance.json'),0o600);
      const fileHash = bytes => createHash('sha256').update(bytes).digest('hex');
      async function admitArtifact(kind, archivePath, expected) {
        const evidence = path.dirname(archivePath), archiveBytes = await readFile(archivePath);
        assert.equal(archiveBytes.byteLength, expected.size, kind + ' archive size');
        assert.equal(fileHash(archiveBytes), expected.sha256, kind + ' archive hash');
        const receipt = JSON.parse(await readFile(path.join(evidence,'receipt.json'),'utf8'));
        const externalManifestBytes = await readFile(path.join(evidence,'manifest.json'));
        const inventory = JSON.parse(await readFile(path.join(evidence,'inventory-verification.json'),'utf8'));
        const attestation = JSON.parse(await readFile(path.join(evidence,'attestation.json'),'utf8'));
        assert.equal(receipt.sourceSha,sourceSha); assert.equal(receipt.sha256,expected.sha256); assert.equal(receipt.size,expected.size); assert.equal(receipt.runId,'42252644242');
        assert.equal(inventory.receipt.sha256,receipt.sha256); assert.equal(inventory.verifiedRegularFiles,expected.files);
        const bundle=attestation[0]?.attestation?.bundle; assert.ok(bundle,'signed GitHub attestation bundle exists');
        const statement=JSON.parse(Buffer.from(bundle.dsseEnvelope.payload,'base64').toString('utf8'));
        assert.equal(statement.subject[0].digest.sha256,expected.sha256);
        assert.equal(statement.predicate.buildDefinition.resolvedDependencies[0].digest.gitCommit,sourceSha);
        assert.equal(statement.predicate.buildDefinition.externalParameters.workflow.path,'.github/workflows/stream-engine.yml');
        const bundlePath=path.join(inputEvidence,kind+'.attestation.bundle.json');
        await writeFile(bundlePath,JSON.stringify(bundle),{mode:0o600});
        const verification=await runBounded('gh',['attestation','verify',archivePath,'--bundle',bundlePath,'--repo','tryapparent/apparent','--signer-workflow','tryapparent/apparent/.github/workflows/stream-engine.yml','--source-digest',sourceSha,'--source-ref','refs/heads/main'],30000);
        await writeFile(path.join(inputEvidence,kind+'.attestation-verification.log'),verification.stdout+verification.stderr,{mode:0o600});
        for(const name of ['receipt.json','manifest.json','inventory-verification.json','attestation.json']) {
          const retained=path.join(inputEvidence,kind+'-'+name);
          await copyFile(path.join(evidence,name),retained); await chmod(retained,0o600);
        }
        const extraction=path.join(root,'runtime-artifacts',kind); await mkdir(extraction,{recursive:true,mode:0o700});
        await runBounded('tar',['-xzf',archivePath,'-C',extraction],30000);
        const payload=path.join(extraction,'stream-payload','payload');
        const manifestBytes=await readFile(path.join(payload,'manifest.json'));
        assert.equal(fileHash(manifestBytes),fileHash(externalManifestBytes),kind+' archived and admitted manifests match');
        const manifest=JSON.parse(manifestBytes.toString('utf8'));
        assert.equal(manifest.sourceSha,sourceSha); assert.deepEqual(manifest.runtimeFiles.length,expected.files);
        const listed=new Map(manifest.runtimeFiles.map(file=>[file.path,file])), seen=new Set();
        async function walk(dir) {
          for(const entry of await readdir(dir,{withFileTypes:true})) {
            const full=path.join(dir,entry.name), relative=path.relative(payload,full), info=await lstat(full);
            assert.equal(info.isSymbolicLink(),false,kind+' payload symlink: '+relative);
            if(info.isDirectory()) await walk(full);
            else {
              assert.equal(info.isFile(),true,kind+' payload non-regular file: '+relative);
              if(relative==='manifest.json') continue;
              const declared=listed.get(relative); assert.ok(declared,kind+' unlisted payload file: '+relative); seen.add(relative);
              assert.equal(info.mode&0o777,declared.mode,kind+' mode: '+relative);
              assert.equal(fileHash(await readFile(full)),declared.sha256,kind+' file hash: '+relative);
            }
          }
        }
        await walk(payload); assert.equal(seen.size,listed.size,kind+' runtime closure is complete');
        if(kind==='media') {
          assert.ok(manifest.capabilities.includes('media/2'));
          assert.deepEqual(manifest.media2Launch.argv,['--media-runtime-directory','<capture-runtime-dir>','--media-channel','media','--media-protocol-version','2']);
          assert.equal(manifest.media2Launch.sharesCaptureRuntime,true);
          assert.equal(manifest.launch.entry,'apparent-engine-frames'); assert.equal(manifest.launch.pathLookup,false); assert.equal(manifest.launch.runtimeBuilds,false);
          assert.equal(listed.get('apparent-engine-frames').sha256,'c1301674edc6db8a3de7d4e0b02b50997a99a7d7880b0d280f7f0e5a55ae713c');
          assert.equal(listed.get('libexec/apparent-engine-frames').sha256,'0713b448920446e1fd1f1a06bc75124d4b04af13793bd1ad8912e0a78ea3a037');
        } else {
          assert.ok(manifest.capabilities.includes('capture/2'));
          assert.deepEqual(manifest.launch.argv,['--runtime-directory','<private-runtime-dir>','--capture-channel','capture','--capture-protocol-version','2']);
          assert.equal(manifest.launch.entry,'apparent-engine-capture-launch'); assert.equal(manifest.launch.worker,'libexec/apparent-engine-capture');
          assert.equal(manifest.launch.pathLookup,false); assert.equal(manifest.launch.runtimeBuilds,false);
          assert.equal(listed.get('apparent-engine-capture-launch').sha256,'524f26ff629e8a116c041700f42708a2993a15e423c771b98d8b18b55f15a1f1');
          assert.equal(listed.get('libexec/apparent-engine-capture').sha256,'10ccdb950f9c6f72b3898b7bfa150fae3264bcbe8d90ca23a4064918dba94d07');
        }
        await recordLog(kind+' admitted archive',{path:archivePath,sha256:expected.sha256,size:expected.size,sourceSha,runtimeFiles:seen.size,payload});
        return {payload,manifest};
      }
      const captureArtifact=await admitArtifact('capture',path.resolve(process.env.APPARENT_CAPTURE2_ARCHIVE),{sha256:'21c8fd7eb3da9f5d9841ac75188e66df0da24225e4e4f892a65b86f6be87284e',size:10398173,files:54});
      const mediaArtifact=await admitArtifact('media',path.resolve(process.env.APPARENT_MEDIA2_ARCHIVE),{sha256:'17a5f71442dc3920a7be5bc4836d186b2da212840ee3f2a2bd811f508ee4625b',size:100853159,files:186});
      assert.deepEqual(acceptance.inventoriesVerified,{media:186,capture:54});
      const capturePayload=captureArtifact.payload, captureManifest=captureArtifact.manifest, mediaPayload=mediaArtifact.payload, mediaManifest=mediaArtifact.manifest;
      let xvfb, display = '', scene, client, mediaClient, host, recordRuntime, recordClient, pillHost, mappings;
      let clientClosed = false;
      let qualificationError;
      let cleanupErrors = [];
      const previousDisplay = process.env.DISPLAY, previousWayland = process.env.WAYLAND_DISPLAY;
      try {
        xvfb = trackChild(spawn('Xvfb', ['-displayfd','1','-screen','0','683x480x24','-nolisten','tcp'], {stdio:['ignore','pipe','pipe']}));
        display = await new Promise((resolve, reject) => {
          const timer = setTimeout(() => reject(new Error('owned Xvfb startup deadline')), 5000);
          xvfb.stdout.setEncoding('utf8'); xvfb.stdout.once('data', value => { clearTimeout(timer); resolve(':' + value.trim()); });
          xvfb.once('exit', code => { clearTimeout(timer); reject(new Error('owned Xvfb exited: ' + code)); });
          xvfb.once('error', reject);
        });
        const env = {...process.env, DISPLAY:display}; delete env.WAYLAND_DISPLAY;
        process.env.DISPLAY=display; delete process.env.WAYLAND_DISPLAY;
        scene = trackChild(spawn('xmessage', ['-geometry','+170+120','-timeout','240','Record capture2 synthetic Xvfb scene'], {env, stdio:'ignore'}));
        let sceneSpawnError;
        scene.once('error', error => { sceneSpawnError = error; });
        const runtime = path.join(root, 'runtime'), control = path.join(root, 'control'), bundles = path.join(root, 'bundles');
        await Promise.all([runtime,control,bundles].map(dir => mkdir(dir,{mode:0o700})));
        const transport = launchCaptureEngine(path.join(capturePayload, captureManifest.launch.entry), runtime, 40000);
        client = new CaptureClient(transport);
        mappings = new SqliteCaptureMappingStore(control);
        const destinations = new AbsentDestinationAllocator(control,bundles);
        host = createCapture2Host({client,mappings,destinations});
          if (sceneSpawnError) throw sceneSpawnError;
          const deadline = Date.now()+150000;
          const waitFor = async (fn, label) => {
            while (Date.now()<deadline) { const value=await fn(); if (value) return value; await new Promise(r=>setTimeout(r,250)); }
            throw new Error('qualification deadline waiting for '+label);
          };
          const ready = await waitFor(async()=>{const value=await host.readiness();return value.ready?value:null;},'native capture readiness');
          const discovery = await host.discoverSources(); assert.equal(discovery.state,'available');
          const displaySource = discovery.sources.find(s=>s.kind==='display' && s.bounds.width===683 && s.bounds.height===480);
          const areaSource = discovery.sources.find(s=>s.kind==='area' && s.displayId===displaySource?.displayId);
          assert.ok(displaySource,'683x480 owned Xvfb display was not discovered'); assert.ok(areaSource,'advertised center-half area was not discovered');
          assert.deepEqual(areaSource.bounds,{x:170,y:120,width:341,height:240,scale:1});
          const options={audio:'none',localOnly:true};
          const invocation={kind:'explicit',invocationId:'a'.repeat(32),observedAtMs:Date.now()};
          recordRuntime = await createCapture2RecordRuntime({host,pollIntervalMs:200,createStartIntent(requestId,fresh) {
            assert.equal(fresh.state,'available');
            const source=fresh.sources.find(s=>s.id===displaySource.id); assert.ok(source,'explicit selection must match fresh discovery');
            const digest=createHash('sha256').update(requestId,'utf8').digest('hex');
            return {source,options,invocation:{...invocation,observedAtMs:Date.now()},destinationPath:path.join(bundles,'record-'+digest+'.apparent')};
          }});
          recordClient=createRecordCapabilityProvider({runtime:recordRuntime,capture2Start:recordRuntime,providerEpoch:'packed-native-provider',native:{readFacts:async()=>({version:1,providerEpoch:'packed-native-native',revision:1,readiness:[],shortcut:{effective:null,label:null,registration:'unknown',externallyManaged:false,draft:null}})}});
          const requestId = 'packed-native-display-'+Date.now();
          let before=recordClient.current();
          const localOnly=await recordClient.execute({type:'start-capture',delivery:'local-only',requestId:'packed-native-local-only-'+Date.now(),expected:{providerEpoch:before.providerEpoch,revision:before.revision}});
          assert.equal(localOnly.status,'refused','native remote-delivery refusal is not Record pipeline local-only enforcement');
          let actionIds=0;
          pillHost=createRecordClientPillHost(recordClient,{createRequestId:()=>actionIds++===0?requestId:'packed-pill-action-'+actionIds});
          const initialPill=await pillHost.read(); assert.ok(initialPill.actions.some(action=>action.id==='record'&&action.enabled));
          await pillHost.act(initialPill.revision,'record');
          const active=await waitFor(async()=>recordClient.current().captures.find(x=>x.captureId===requestId&&x.phase==='recording'),'public RecordClient recording fact');
          await new Promise(resolve=>setTimeout(resolve,1800));
          const mapping = await mappings.getByRequestId(requestId); assert.ok(mapping?.capture2Intent);
          assert.equal(mapping.bundlePath,path.join(bundles,path.basename(mapping.bundlePath)));
          await assert.rejects(lstat(mapping.bundlePath),e=>e.code==='ENOENT','Record must reserve, never create, the final destination');
          const recording = await waitFor(async()=>{const x=await host.inspect(requestId);return x.phase==='recording'?x:null;},'real recording receipt');
          assert.equal(recording.requestId,requestId);
          const recordingPill=await pillHost.read(); assert.equal(recordingPill.phase,'recording');
          await pillHost.act(recordingPill.revision,'finish');
          const stopReceipt = await host.inspect(requestId);
          assert.ok(['stopping','materials-ready'].includes(stopReceipt.phase));
          const stopped = stopReceipt.phase==='materials-ready' ? stopReceipt : await waitFor(async()=>{const value=await host.inspect(requestId);return value.phase==='materials-ready'?value:null;},'retained materials ready');
          assert.ok(stopped.materials?.leaseId);
          const manifestBeforeRestart = stopped.materials;
          host.dispose(); host = createCapture2Host({client,mappings,destinations});
          const cold = await host.inspect(requestId);
          assert.equal(cold.phase,'materials-ready'); assert.deepEqual(cold.materials,manifestBeforeRestart);
          const loaded = await readCaptureMaterials({manifest:cold.materials,expected:{key:cold.materials.key,leaseId:cold.materials.leaseId,bundleId:mapping.bundleId},trustedRoot:cold.materials.root});
          assert.deepEqual(Buffer.from(loaded.payloadBytes),await readFile(path.join(cold.materials.root,cold.materials.payloadFile.relativePath)));
          assert.equal(createHash('sha256').update(loaded.payloadBytes).digest('hex'),cold.materials.key.revision);
          assert.equal(loaded.materials.payload.safety.nativeRemoteDelivery,'forbidden');
          assert.equal(loaded.materials.payload.safety.safeForProcessing,true);
          assert.ok(loaded.materials.payload.clock.durationMs>0);
          assert.deepEqual(loaded.materials.payload.collection,{context:'unavailable',interactions:'unavailable',candidates:'unavailable',transcript:'unavailable',acceptedStrokes:'unavailable'});
          assert.equal(loaded.materials.payload.confirmedSource.id,displaySource.id);
          assert.deepEqual(loaded.materials.payload.confirmedSource.rectangle,displaySource.bounds);
          await recordLog('retained capture materials',{requestId,leaseId:cold.materials.leaseId,materials:cold.materials.key,root:cold.materials.root,
            durationMs:loaded.materials.payload.clock.durationMs,clock:loaded.materials.payload.clock,video:loaded.materials.payload.video,collection:loaded.materials.payload.collection});
          const movie = path.join(cold.materials.root,loaded.materials.payload.video.file);
          const probe = await runBounded('ffprobe',['-v','error','-count_frames','-show_entries','stream=codec_name,width,height,nb_read_frames','-of','json',movie],15000);
          const stream = JSON.parse(probe.stdout).streams.find(x=>x.codec_type==='video') ?? JSON.parse(probe.stdout).streams[0];
          assert.ok(stream && Number(stream.nb_read_frames)>0,'saved capture must contain decoded video frames');
          assert.equal(stream.codec_name,'h264');
          assert.equal(stream.width,682,'683px Xvfb capture encodes the actual even-width 682px stream');
          await runBounded('ffmpeg',['-v','error','-i',movie,'-f','null','-'],20000);
          await recordLog('complete capture decode',{display:displaySource.bounds,area:areaSource.bounds,frameWidth:stream.width,frameHeight:stream.height,frames:stream.nb_read_frames,
            clock:loaded.materials.payload.clock,collection:loaded.materials.payload.collection,requestId,leaseId:cold.materials.leaseId,movie});

          const mediaEntry = mediaManifest.runtimeFiles.find(file=>file.path==='apparent-engine-frames');
          assert.ok(mediaEntry);
          const mediaRuntime = path.join(root,'runtime');
          mediaClient = new Media2Client(launchMedia2Worker(path.join(mediaPayload,mediaEntry.path),mediaRuntime,40000));
          const mediaHandshake = await mediaClient.handshake(5000);
          assert.equal(mediaHandshake.implementation,'native');
          assert.deepEqual(mediaHandshake.capabilities,{retainedFrames:true,contactSheet:true,nativeJobAuthority:true});
          const selectedTimes = [0,750,750];
          const layout = {schema:'apparent.contact-sheet-layout/1',width:960,height:180,backgroundRgb:[16,20,28],fit:'stretch',sampling:'legacy-bilinear',clipping:'reject',
            tiles:[{requestIndex:1,x:0,y:0,width:320,height:180},{requestIndex:2,x:320,y:0,width:320,height:180},{requestIndex:3,x:640,y:0,width:320,height:180}]};
          assert.deepEqual(layout.tiles.map(tile=>tile.requestIndex),[1,2,3],'Record explicitly supplies one sheet tile for each ordered slot');
          assert.deepEqual(selectedTimes,[0,750,750],'Record preserves duplicate caller-selected capture times as distinct slots');
          assert.ok(loaded.materials.payload.clock.durationMs>1000,'capture must cover each caller-selected timestamp');
          const jobId=createHash('sha256').update('record-media2:'+requestId).digest('hex').slice(0,32);
          const derivativeRoot=path.join(root,'derivative-output',jobId);
          await mkdir(path.dirname(derivativeRoot),{mode:0o700});
          const request=prepareMedia2Request({schema:'apparent.retained-frame-request/1',materials:loaded.materials.manifest.key,leaseId:loaded.materials.payload.leaseId,
            jobId,outputDirectory:derivativeRoot,requestedBundleMs:selectedTimes,limits:{maxFrames:3,maxEdge:320,maxTotalPixels:2_000_000,deadlineMs:45_000},contactSheet:layout},loaded.materials).request;
          const layoutBytes=encodeContactSheetLayout(layout);
          assert.deepEqual(Buffer.from(layoutBytes),Buffer.from(encodeContactSheetLayout(request.contactSheet)));
          const mediaInput={ownerToken:mapping.ownerToken,request,materials:loaded.materials};
          const mediaJournal=path.join(root,'media2-jobs',jobId+'.sqlite');
          await mkdir(path.dirname(mediaJournal),{mode:0o700});
          let mediaJob=await RecordMedia2Job.create(mediaClient,mediaJournal,mediaInput);
          assert.equal(mediaJob.state,'ready');
          let mediaStatus;
          try { mediaStatus=await mediaJob.derive(mapping.ownerToken,60_000); }
          catch(error) {
            await recordLog('media2 derive reply',{state:mediaJob.state,name:error?.name,message:error?.message,dispatched:error?.dispatched??'unknown'});
            if(mediaJob.state!=='unknown') throw error;
            mediaStatus=await mediaJob.inspect(mapping.ownerToken,60_000);
          }
          await recordLog('media2 derive status',{state:mediaStatus.state,jobId:mediaStatus.jobId,requestSha256:mediaStatus.requestSha256});
          const mediaDeadline=Date.now()+50_000;
          while(mediaStatus.state==='running'&&Date.now()<mediaDeadline) {
            await new Promise(resolve=>setTimeout(resolve,250));
            mediaStatus=await mediaJob.inspect(mapping.ownerToken,60_000);
            await recordLog('media2 inspect status',{state:mediaStatus.state,jobId:mediaStatus.jobId,requestSha256:mediaStatus.requestSha256});
          }
          assert.equal(mediaStatus.state,'complete','native derivative must reach a validated complete job');
          assert.equal(mediaJob.state,'complete');
          const originalTuple={ownerToken:mapping.ownerToken,request,materials:loaded.materials};
          await mediaClient.close(); mediaClient=undefined;
          mediaClient=new Media2Client(launchMedia2Worker(path.join(mediaPayload,mediaEntry.path),mediaRuntime,40000));
          const restartedHandshake=await mediaClient.handshake(5000);
          assert.deepEqual(restartedHandshake,mediaHandshake,'restarted media/2 client negotiates the same native capabilities');
          mediaJob=await RecordMedia2Job.reopen(mediaClient,mediaJournal,originalTuple);
          assert.equal(mediaJob.state,'complete');
          const inspected=await mediaJob.inspect(mapping.ownerToken,60_000);
          assert.equal(inspected.state,'complete','cold native inspection reconciles the same completed request');
          const derivative=mediaJob.derivative; assert.ok(derivative,'Record retains the SDK-validated derivative document');
          const envelope=derivative.envelope;
          assert.deepEqual(envelope.payload.frames.map(frame=>[frame.requestIndex,frame.requestedBundleMs]),[[1,0],[2,750],[3,750]]);
          assert.deepEqual(envelope.payload.frames.map(frame=>frame.effectiveVideoMs),selectedTimes.map(time=>Math.max(0,time-loaded.materials.payload.clock.videoOffsetMs)));
          assert.deepEqual(envelope.payload.frames.map(frame=>frame.clampedToStart),selectedTimes.map(time=>time<loaded.materials.payload.clock.videoOffsetMs));
          assert.ok(envelope.payload.frames.every(frame=>Number.isSafeInteger(frame.actualVideoMs)&&frame.actualVideoMs<=loaded.materials.payload.clock.durationMs&&typeof frame.clampedToEnd==='boolean'));
          assert.equal(envelope.payload.contactSheet.width,layout.width); assert.equal(envelope.payload.contactSheet.height,layout.height);
          assert.equal(envelope.payload.contactSheet.layoutSha256,createHash('sha256').update(layoutBytes).digest('hex'));
          const bundlePath=mapping.bundlePath;
          const imageMappings=[...selectedTimes.map((_,index)=>({bundlePath:'frames/frame-'+String(index+1).padStart(2,'0')+'.png',selector:{kind:'frame',requestIndex:index+1}})),
            {bundlePath:'contact-sheet.png',selector:{kind:'contact-sheet'}}];
          const retained=await readRetainedDerivativeArtifacts({document:derivative,materials:loaded.materials,trustedRoot:derivativeRoot,mappings:imageMappings,
            limits:{maxFileBytes:16*1024*1024,maxAggregateBytes:40*1024*1024}});
          assert.equal(retained.artifacts.length,4);
          const frameHashes=[];
          for(const item of retained.artifacts) {
            const png=Buffer.from(item.bytes);
            assert.equal(png.subarray(0,8).toString('hex'),'89504e470d0a1a0a');
            assert.equal(createHash('sha256').update(png).digest('hex'),item.sha256);
            const expected=item.role==='contact-sheet'?envelope.payload.contactSheet:envelope.payload.frames[imageMappings.find(binding=>binding.bundlePath===item.relativePath).selector.requestIndex-1];
            assert.deepEqual([png.readUInt32BE(16),png.readUInt32BE(20)],[expected.width,expected.height]);
            if(item.role==='frame') frameHashes.push(item.sha256);
          }
          assert.notEqual(envelope.payload.frames[0].file.relativePath,envelope.payload.frames[1].file.relativePath);
          await recordLog('native media2 derivative',{jobId,requestSha256:mediaJob.requestSha256,receiptId:envelope.receiptId,root:envelope.root,
            request:request.requestedBundleMs,frames:envelope.payload.frames.map(frame=>({requestIndex:frame.requestIndex,requestedBundleMs:frame.requestedBundleMs,
              effectiveVideoMs:frame.effectiveVideoMs,actualVideoMs:frame.actualVideoMs,clampedToStart:frame.clampedToStart,clampedToEnd:frame.clampedToEnd,
              width:frame.width,height:frame.height,sha256:frame.file.sha256,path:frame.file.relativePath})),contactSheet:{width:envelope.payload.contactSheet.width,
              height:envelope.payload.contactSheet.height,sha256:envelope.payload.contactSheet.file.sha256,path:envelope.payload.contactSheet.file.relativePath},frameHashes});

          const canonicalReasons=['initial','initial','initial'];
          const frames=envelope.payload.frames.map((frame,index)=>({index:frame.requestIndex,timeMs:frame.requestedBundleMs,absolutePath:path.join(bundlePath,imageMappings[index].bundlePath),
            artifactPath:imageMappings[index].bundlePath,canonicalReason:canonicalReasons[index]}));
          const confirmed=loaded.materials.payload.confirmedSource;
          const assemblyInput={manifest:loaded.materials.manifest,payloadBytes:loaded.payloadBytes,facts:loaded.assemblyFacts,
            admission:{captureId:loaded.materials.manifest.key.captureId,materialsId:loaded.materials.manifest.key.materialsId,createdAtMs:loaded.materials.payload.invocation.observedAtMs,
              bundleId:loaded.materials.payload.bundleId,bundlePath,target:{mode:'display',displayId:confirmed.displayId,
                frame:{x:confirmed.rectangle.x,y:confirmed.rectangle.y,width:confirmed.rectangle.width,height:confirmed.rectangle.height,scale:confirmed.rectangle.scale}},
              source:confirmed.kind},frames,marks:[],artifacts:[...retained.artifacts],materialReferences:[{role:'video',relativePath:loaded.materials.payload.video.file}],
            processingWarnings:[{kind:'videoOnlyInteractionTelemetry'},{kind:'noMicrophoneAudio'},{kind:'noTranscript',message:'This synthetic Xvfb qualification did not request audio.'}]};
          const assembly=assembleCanonicalBundle(assemblyInput);
          const apparent=JSON.parse(new TextDecoder().decode(assembly.apparentJson));
          assert.equal(apparent.video,loaded.materials.payload.video.file);
          assert.equal(apparent.contactSheet,'contact-sheet.png');
          assert.deepEqual(apparent.keyframes.map(frame=>[frame.index,frame.t,frame.reason,frame.file]),frames.map(frame=>[frame.index,frame.timeMs,frame.canonicalReason,frame.artifactPath]));
          assert.equal(assembly.inventory.entries.find(entry=>entry.path===loaded.materials.payload.video.file)?.provenance,'material-reference');
          const publicationId=createHash('sha256').update('record-media2-publication:'+requestId).digest('hex').slice(0,32);
          const derivativeBindings=retained.derivative2Artifacts;
          const prepared=await prepareRecordPublication({ownerToken:mapping.ownerToken,publicationId,stagingRoot:path.join(root,'staging'),destinationPath:bundlePath,
            materialsManifest:loaded.materials.manifest,materialsPayloadBytes:loaded.payloadBytes,assembly,
            artifactBytes:Object.fromEntries(retained.artifacts.map(item=>[item.relativePath,item.bytes])),derivatives2:[derivative],derivative2Artifacts:derivativeBindings});
          assert.equal(preflightCapturePublication(prepared.input).plan.publicationId,publicationId);
          for(const binding of imageMappings) {
            const entry=prepared.plan.entries.find(item=>item.relativePath===binding.bundlePath);
            assert.equal(entry?.input.kind,'derivative2');
            assert.deepEqual(entry.input.selector,binding.selector);
          }
          assert.equal(prepared.plan.entries.find(item=>item.role==='video')?.input.kind,'material');
          assert.equal(prepared.plan.entries.find(item=>item.role==='video')?.sha256,loaded.materials.payload.files.find(file=>file.relativePath===loaded.materials.payload.video.file).sha256);
          assert.equal(prepared.plan.entries.find(item=>item.role==='video')?.byteLength,loaded.materials.payload.files.find(file=>file.relativePath===loaded.materials.payload.video.file).identity.size);
          assert.equal(Buffer.from(prepared.input.planBytes).includes(Buffer.from(mapping.ownerToken)),false,'private plan must not contain owner token');
          assert.equal((await readFile(mediaJournal)).includes(Buffer.from(mapping.ownerToken)),false,'private media journal must not contain owner token');
          assert.equal((await readdir(path.join(root,'staging',publicationId),{recursive:true})).some(file=>file==='capture.mp4'),false,'material-reference media is not copied into JavaScript staging');
          await assert.rejects(lstat(bundlePath),error=>error.code==='ENOENT','processing plan preparation must not create a Saved bundle');
          await recordLog('Record native processing plan',{materials:loaded.materials.manifest.key,requestId,jobId,receiptId:envelope.receiptId,
            bundlePath,publicationId,planPath:path.join(root,'staging',publicationId,'publication-plan.json'),planSha256:prepared.input.planFile.planSha256,
            entries:prepared.plan.entries.map(entry=>({path:entry.relativePath,role:entry.role,input:entry.input.kind,sha256:entry.sha256,byteLength:entry.byteLength,
              selector:entry.input.kind==='derivative2'?entry.input.selector:undefined})),saved:false,published:false});

          const cancelId='packed-native-cancel-'+Date.now();
          await host.discoverSources();
          const areaStarted=await host.start({requestId:cancelId,source:areaSource,options,invocation:{...invocation,invocationId:'b'.repeat(32),observedAtMs:Date.now()}});
          assert.ok(['starting','recording'].includes(areaStarted.phase));
          const areaMapping=await mappings.getByRequestId(cancelId); assert.ok(areaMapping?.capture2Intent);
          assert.equal((await waitFor(async()=>{const x=await host.inspect(cancelId);return x.phase==='recording'?x:null;},'area recording receipt')).phase,'recording');
          host.dispose();
          assert.equal((await client.inspect({captureId:areaMapping.captureId,ownerToken:areaMapping.ownerToken})).state,'recording','dispose must detach without stopping native capture');
          host=createCapture2Host({client,mappings,destinations});
          const reconciled=await host.inspect(cancelId); assert.equal(reconciled.phase,'recording');
          const cancelReceipt=await host.cancel(cancelId); assert.ok(['recording','cancelled'].includes(cancelReceipt.phase));
          const nativeCancelled=await waitFor(async()=>{const value=await client.inspect({captureId:areaMapping.captureId,ownerToken:areaMapping.ownerToken});return value.state==='cancelled'?value:null;},'native cancel completion');
          assert.equal(nativeCancelled.state,'cancelled');
          assert.equal((await host.inspect(cancelId)).phase,'cancelled');
          assert.equal(await lstat(areaMapping.bundlePath).then(()=>true,()=>false),false,'cancel must not create final destination');
          console.log('packed Record capture/2 native Xvfb consumer passed');
      } catch (error) {
        qualificationError = error;
      } finally {
        const cleanupSteps = [];
        if (mappings && client) cleanupSteps.push({label:'capture reconciliation',run:()=>reconcileAndCancel(mappings,client,Date.now()+15000)});
        if (mediaClient) cleanupSteps.push({label:'media/2 client close',run:async()=>{await mediaClient.close();mediaClient=undefined;}});
        if (pillHost) cleanupSteps.push({label:'pill host disposal',run:()=>disposeWorkflowPillHost(pillHost)});
        if (recordClient) cleanupSteps.push({label:'Record client detach',run:()=>recordClient.detach()});
        if (recordRuntime) cleanupSteps.push({label:'Record runtime disposal',run:()=>recordRuntime.dispose()});
        if (host) cleanupSteps.push({label:'capture host disposal',run:()=>host.dispose()});
        if (client && !clientClosed) cleanupSteps.push({label:'capture client close',run:async()=>{await client.close();clientClosed=true;}});
        if (scene) cleanupSteps.push({label:'synthetic scene exit',run:()=>waitForExit(scene,1000)});
        if (xvfb) cleanupSteps.push({label:'owned Xvfb exit',run:()=>waitForExit(xvfb,1000)});
        cleanupErrors = await runCleanupSteps(cleanupSteps);
        for (const failure of cleanupErrors) console.error(failure.stack);
        if(previousDisplay===undefined)delete process.env.DISPLAY;else process.env.DISPLAY=previousDisplay;
        if(previousWayland===undefined)delete process.env.WAYLAND_DISPLAY;else process.env.WAYLAND_DISPLAY=previousWayland;
        console.error('private native proof retained for inspection and recovery: '+root);
      }
      if (qualificationError || cleanupErrors.length) throw new AggregateError([...(qualificationError?[qualificationError]:[]),...cleanupErrors],'native qualification failed; inspect retained proof and cleanup results',{cause:qualificationError});
    `;
    // The consumer scratch is removed in the outer finally; the native proof
    // root retains its own script copy independently.
    const qualificationPacket = path.join(scratch,'qualification-script');
    await mkdir(qualificationPacket,{mode:0o700});
    const qualificationScript = path.join(qualificationPacket,'qualification.mjs');
    await writeFile(qualificationScript,nativeScript,{mode:0o600});
    await checkGeneratedNodeScript(nativeScript);
    console.log('private generated native script packet: '+qualificationScript);
    const capture2Result = await runQualification(process.execPath, ['--experimental-strip-types', '--input-type=module', '-e', nativeScript], { cwd: capture2, timeout: 240000, env: { ...process.env, APPARENT_CAPTURE2_ARCHIVE: archive, APPARENT_MEDIA2_ARCHIVE: mediaArchive, APPARENT_QUALIFICATION_HELPERS: path.join(root, 'test/native-qualification/native-qualification.ts'), APPARENT_QUALIFICATION_SCRIPT_PATH: qualificationScript } });
    process.stdout.write(capture2Result.stdout);
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
