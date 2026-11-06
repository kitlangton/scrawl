/** Verify the packed artifact in consumers that cannot see repository siblings. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, writeFile, copyFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { runQualification } from './native-qualification/native-qualification.ts';

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
    console.log('headless packed runtime and declaration consumer passed');
  `], { cwd: headless });
  process.stdout.write(runtime.stdout);

  // The canonical bundle subpath uses only the packed public SDK contract.
  const sdkArtifact = process.env.APPARENT_ENGINE_SDK_TARBALL ?? path.join(root, 'vendor/engine-sdk-0.3.2.tgz');
  const sdkBytes = await readFile(sdkArtifact);
  assert.equal(createHash('sha256').update(sdkBytes).digest('hex'), '3b539f7d978d692a5d7fca9f358047bc197d415f6764c33707cff5a710119d01');
  await run('npm', ['install', '--no-audit', '--no-fund', tarball, sdkArtifact], { cwd: bundle });
  const bundleDeclarations = path.join(bundle, 'consumer.ts');
  await writeFile(bundleDeclarations, `
    import { assembleCanonicalBundle, type BundleAssemblyInput, type BundleValidators, type CanonicalStroke } from '@apparent/record/processing/bundle';
    import { validateMaterialsDocument, validateFactFile, type MaterialsDocument, type ContextFacts, type InteractionFacts, type TranscriptFacts, type StrokeFacts, type StrokeRgb } from '@apparent/engine-sdk/capture';
    import {readCaptureMaterials, type CaptureMaterialsInput} from '@apparent/record/host/materials';
    import {createCapture2RecordRuntime, type Capture2RecordRuntimeOptions} from '@apparent/record/host/capture2-runtime';
    declare const capture2RuntimeOptions: Capture2RecordRuntimeOptions;
    void createCapture2RecordRuntime(capture2RuntimeOptions);
    const validators: BundleValidators = { materials: validateMaterialsDocument, fact: validateFactFile };
    const consume = (input: BundleAssemblyInput) => assembleCanonicalBundle(input, validators);
    const loadAndAssemble = async (input: CaptureMaterialsInput, policy: Omit<BundleAssemblyInput, 'manifest' | 'payloadBytes' | 'facts'>) => {
      const loaded = await readCaptureMaterials(input);
      return assembleCanonicalBundle({ ...policy, manifest: loaded.materials.manifest, payloadBytes: loaded.payloadBytes, facts: loaded.assemblyFacts }, validators);
    };
    const rgb: StrokeRgb = [12, 34, 56];
    const canonical: CanonicalStroke = { id: 'record-mapping', t0: 0, t1: 1, fw: 1, fh: 1, pts: [0, 0, 0], rgb };
    void canonical; void loadAndAssemble; void consume; void (null as unknown as MaterialsDocument | ContextFacts | InteractionFacts | TranscriptFacts | StrokeFacts);
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
  const sdkTarball = path.join(scratch, 'engine-sdk-0.3.2.tgz'); await copyFile(sdkArtifact, sdkTarball);
  await run('npm', ['install', '--no-audit', '--no-fund', sdkTarball, tarball], { cwd: publication });
  const publicationResult = await run(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import {createRequire} from 'node:module';
    import {CaptureClient, preflightCapturePublication} from '@apparent/engine-sdk/capture';
    import {ReferenceCapturePeer} from '@apparent/engine-sdk/capture/testing';
    import {createRecordPublicationRecovery, createCompletedCapturePublisher, prepareRecordPublication} from '@apparent/record/publication';
    import {createCapture2Host} from '@apparent/record/host/capture2';
    import {createCapture2RecordRuntime} from '@apparent/record/host/capture2-runtime';
    const client = new CaptureClient(new ReferenceCapturePeer(), {allowTestDouble:true});
    assert.equal((await client.handshake()).readiness, 'unavailable');
    const recovery = createRecordPublicationRecovery(client, process.cwd() + '/unused-private-root', preflightCapturePublication);
    assert.equal(typeof recovery.publish, 'function');
    assert.equal(typeof createCapture2Host, 'function');
    assert.equal(typeof createCapture2RecordRuntime, 'function');
    assert.equal(typeof prepareRecordPublication, 'function');
    assert.equal(typeof createCompletedCapturePublisher, 'function');
    assert.equal(typeof createCompletedCapturePublisher(client, process.cwd() + '/unused-private-root', preflightCapturePublication).publishCompletedCapture, 'function');
    for (const name of ['react','react-dom','electron','@apparent/ui']) assert.throws(()=>createRequire(import.meta.url).resolve(name));
    await client.close();
    console.log('packed publication profile passed without UI peers');
  `], { cwd: publication });
  process.stdout.write(publicationResult.stdout);

  // The bounded Linux native qualification is opt-in. It installs the packed
  // Record artifact and published SDK into a separate consumer, then launches
  // only the explicitly supplied, preverified capture/2 artifact on owned
  // synthetic Xvfb. It never installs or changes a host application route.
  if (process.env.APPARENT_CAPTURE2_QUALIFY === '1') {
    const payload = process.env.APPARENT_CAPTURE2_PAYLOAD;
    const archive = process.env.APPARENT_CAPTURE2_ARCHIVE;
    assert.ok(payload && path.isAbsolute(payload), 'APPARENT_CAPTURE2_PAYLOAD must name the extracted payload directory');
    assert.ok(archive && path.isAbsolute(archive), 'APPARENT_CAPTURE2_ARCHIVE must name the admitted archive');
    const admittedArchive = await readFile(archive);
    assert.equal(admittedArchive.length, 10371005, 'admitted archive size');
    assert.equal(createHash('sha256').update(admittedArchive).digest('hex'), 'f1cbdf70853e7d10bf9c9ac60c0c0bdef28aaff8d455b4fc8da0f7b28cf4907d', 'admitted archive identity');
    const archiveRoot = path.join(scratch, 'admitted');
    await mkdir(archiveRoot, { mode: 0o700 });
    await run('tar', ['-xzf', archive, '-C', archiveRoot], { timeout: 15000, killSignal: 'SIGTERM' });
    const admittedPayload = path.join(archiveRoot, 'stream-payload/payload');
    assert.equal(createHash('sha256').update(await readFile(path.join(admittedPayload, 'manifest.json'))).digest('hex'), createHash('sha256').update(await readFile(path.join(payload, 'manifest.json'))).digest('hex'), 'external extracted payload matches archive manifest');
    const capture2 = path.join(scratch, 'capture2');
    await initConsumer(capture2);
    await run('npm', ['install', '--no-audit', '--no-fund', tarball, sdkArtifact, 'react@19.2.0', 'react-dom@19.2.0'], { cwd: capture2 });
    const capture2Result = await runQualification(process.execPath, ['--experimental-strip-types', '--input-type=module', '-e', `
      import assert from 'node:assert/strict';
      import {createHash} from 'node:crypto';
      import {spawn} from 'node:child_process';
      import {chmod, lstat, mkdir, mkdtemp, readFile, readdir, realpath, rm, stat} from 'node:fs/promises';
      import {tmpdir} from 'node:os';
      import path from 'node:path';
      import {pathToFileURL} from 'node:url';
      import {CaptureClient, launchCaptureEngine} from '@apparent/engine-sdk/capture';
      import {createCapture2Host} from '@apparent/record/host/capture2';
      import {createCapture2RecordRuntime} from '@apparent/record/host/capture2-runtime';
      import {createRecordCapabilityProvider} from '@apparent/record/capabilities';
      import {createRecordClientPillHost,disposeWorkflowPillHost} from '@apparent/record/ui';
      import {AbsentDestinationAllocator, SqliteCaptureMappingStore} from '@apparent/record/host';
      import {readCaptureMaterials} from '@apparent/record/host/materials';
      const {reconcileAndCancel,runBounded,trackChild,waitForExit} = await import(pathToFileURL(process.env.APPARENT_QUALIFICATION_HELPERS).href);

      const payload = path.resolve(process.env.APPARENT_CAPTURE2_PAYLOAD);
      const archive = path.resolve(process.env.APPARENT_CAPTURE2_ARCHIVE);
      const archiveBytes = await readFile(archive);
      assert.equal(archiveBytes.length, 10371005, 'admitted archive size');
      assert.equal(createHash('sha256').update(archiveBytes).digest('hex'), 'f1cbdf70853e7d10bf9c9ac60c0c0bdef28aaff8d455b4fc8da0f7b28cf4907d', 'admitted archive identity');
      const manifest = JSON.parse(await readFile(path.join(payload, 'manifest.json'), 'utf8'));
      assert.deepEqual(manifest.capabilities, ['capture/2','x11-display','x11-area','retained-materials']);
      assert.deepEqual(manifest.launch.argv, ['--runtime-directory','<private-runtime-dir>','--capture-channel','capture','--capture-protocol-version','2']);
      assert.equal(manifest.launch.entry, 'apparent-engine-capture-launch');
      assert.equal(manifest.launch.worker, 'libexec/apparent-engine-capture');
      assert.equal(manifest.launch.pathLookup, false);
      assert.equal(manifest.launch.runtimeBuilds, false);
      const listed = new Map(manifest.runtimeFiles.map(file => [file.path, file]));
      const seen = new Set();
      async function walk(dir) {
        for (const name of await readdir(dir, {withFileTypes:true})) {
          const full = path.join(dir, name.name), rel = path.relative(payload, full);
          if (name.isSymbolicLink()) throw new Error('payload contains a symlink');
          if (name.isDirectory()) await walk(full);
          else if (rel !== 'manifest.json') {
            const entry = listed.get(rel); assert.ok(entry, 'unlisted payload file: ' + rel); seen.add(rel);
            const info = await lstat(full); assert.equal(info.mode & 0o777, entry.mode, 'mode: ' + rel);
            assert.equal(createHash('sha256').update(await readFile(full)).digest('hex'), entry.sha256, 'hash: ' + rel);
          }
        }
      }
      await walk(payload); assert.equal(seen.size, listed.size, 'runtime closure must have no missing files');

      const root = await realpath(await mkdtemp(path.join(tmpdir(), 'record-capture2-native-')));
      await chmod(root, 0o700);
      console.log('private native proof root: ' + root);
      const xvfb = trackChild(spawn('Xvfb', ['-displayfd','1','-screen','0','683x480x24','-nolisten','tcp'], {stdio:['ignore','pipe','pipe']}));
      let display = '';
      let scene, client;
      let clientClosed = false;
      const previousDisplay = process.env.DISPLAY, previousWayland = process.env.WAYLAND_DISPLAY;
      try {
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
        const transport = launchCaptureEngine(path.join(payload, manifest.launch.entry), runtime, 40000);
        client = new CaptureClient(transport);
        const mappings = new SqliteCaptureMappingStore(control);
        const destinations = new AbsentDestinationAllocator(control,bundles);
        let host = createCapture2Host({client,mappings,destinations});
        let recordRuntime, recordClient, pillHost;
        try {
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
          const movie = path.join(cold.materials.root,loaded.materials.payload.video.file);
          const probe = await runBounded('ffprobe',['-v','error','-count_frames','-show_entries','stream=codec_name,width,height,nb_read_frames','-of','json',movie],15000);
          const stream = JSON.parse(probe.stdout).streams.find(x=>x.codec_type==='video') ?? JSON.parse(probe.stdout).streams[0];
          assert.ok(stream && Number(stream.nb_read_frames)>0,'saved capture must contain decoded video frames');
          assert.equal(stream.codec_name,'h264');
          assert.equal(stream.width,682,'683px Xvfb capture encodes the actual even-width 682px stream');
          await runBounded('ffmpeg',['-v','error','-i',movie,'-f','null','-'],20000);
          console.log(JSON.stringify({display:displaySource.bounds,area:areaSource.bounds,frameWidth:stream.width,frameHeight:stream.height,frames:stream.nb_read_frames,clock:loaded.materials.payload.clock,collection:loaded.materials.payload.collection,requestId,leaseId:cold.materials.leaseId}));

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
        } finally {
          await reconcileAndCancel(mappings,client,Date.now()+15000);
          if(pillHost)disposeWorkflowPillHost(pillHost);
          recordClient?.detach(); recordRuntime?.dispose();
          host.dispose();
          await client.close();
          clientClosed = true;
          await waitForExit(scene,1000);
          await waitForExit(xvfb,1000);
          console.error('private native proof retained for inspection and recovery: '+root);
        }
      } finally {
        try {
          if (client && !clientClosed) await client.close();
          if (scene) await waitForExit(scene,1000);
          await waitForExit(xvfb,1000);
        } finally {
          if(previousDisplay===undefined)delete process.env.DISPLAY;else process.env.DISPLAY=previousDisplay;
          if(previousWayland===undefined)delete process.env.WAYLAND_DISPLAY;else process.env.WAYLAND_DISPLAY=previousWayland;
        }
      }
    `], { cwd: capture2, timeout: 240000, env: { ...process.env, APPARENT_CAPTURE2_PAYLOAD: admittedPayload, APPARENT_CAPTURE2_ARCHIVE: archive, APPARENT_QUALIFICATION_HELPERS: path.join(root, 'test/native-qualification/native-qualification.ts') } });
    process.stdout.write(capture2Result.stdout);
  }
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
