import { mkdtemp, mkdir, rm, writeFile, truncate, readFile, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { PNG } from 'pngjs';

const target = {mode:'display',app:null,bundleId:null,windowTitle:null,windowId:null,displayId:0,frame:{x:0,y:0,width:800,height:600,scale:1}};
import { recordWithEngine, type RecordingClient } from '../src/recording.ts';

test('local bundle commands preserve raw JSON, summary, transcript, marks, and reject unsafe references', async () => {
  const root = await mkdtemp(join(tmpdir(), 'apparent-agent-'));
  try {
    const bundle = join(root, "Sam's review 👋.apparent"); await mkdir(bundle);
    await mkdir(join(bundle,'frames')); await mkdir(join(bundle,'directory-reference'));
    const png=new PNG({width:1,height:1});png.data.set([1,2,3,255]);const image=PNG.sync.write(png);
    await writeFile(join(bundle,'contact-sheet.png'),image); await writeFile(join(bundle,'frames/001.png'),image);
    const raw = JSON.stringify({version:1,id:'fixture',createdAtMs:10,durationMs:3,target,transcript:{text:'héllo',segments:[],words:[]},strokes:[],marks:[{id:'m1',strokes:[0],t0:1,t1:2,boxNorm:[0,0,1,1],shape:'circle',description:'mark'}],events:[],keyframes:[{index:1,t:0,reason:'mark',event:'marked',said:'héllo',markId:'m1',file:'frames/001.png'},{index:2,t:1,file:'frames/missing.png'},{index:3,t:2,file:'directory-reference'},{index:4,t:2,file:'frames/001.png'}],contactSheet:'contact-sheet.png'});
    await writeFile(join(bundle, 'apparent.json'), raw); await writeFile(join(bundle, 'summary.md'), '# Stored\n');
    await writeFile(join(bundle,'.evidence-only.json'),JSON.stringify({version:1,captureId:'a'.repeat(32),server:'https://connect.example',media:'not downloaded',guidance:'Use apparent fetch <id> --media --out <new-directory> for the complete recording'}));
    const cli = new URL('../dist/cli.js', import.meta.url).pathname;
    const run = (...args: string[]) => spawnSync(process.execPath, [cli, 'inspect', ...args], { encoding: 'utf8' });
    const json = run(join(bundle, 'apparent.json'), '--json'); assert.equal(json.stdout, raw);
    assert.match(run(bundle).stdout, /apparent 0:00/); assert.match(run(bundle).stdout,/“héllo”/);
    assert.equal(run(bundle, '--stored-summary').stdout, '# Stored\n');
    assert.equal(run(bundle, '--transcript').stdout, 'héllo\n');
    assert.equal(JSON.parse(run(bundle, '--marks', '--json').stdout)[0].id, 'm1');
    const imageOutput=run(bundle,'--images'); assert.equal(imageOutput.status,0);
    const images=JSON.parse(imageOutput.stdout);
    assert.deepEqual(images.references,['contact-sheet.png','frames/001.png','frames/missing.png','directory-reference']);
    assert.deepEqual(images.paths,[join(bundle,'contact-sheet.png'),join(bundle,'frames/001.png')]);
    assert.deepEqual(images.missingReferences,['frames/missing.png','directory-reference']);
    assert.deepEqual(await readFile(join(bundle,'apparent.json')),Buffer.from(raw));
    assert.equal(run(bundle,'--transcript').stdout,'héllo\n');
    await writeFile(join(bundle, 'apparent.json'), raw.replace('frames/001.png', '../secret.png'));
    assert.equal(run(bundle, '--images').status, 2);
    await writeFile(join(bundle, 'apparent.json'),raw);
    await symlink(join(bundle,'frames/001.png'),join(bundle,'frames/linked.png'));
    const symlinkManifest=JSON.parse(raw);symlinkManifest.keyframes.push({index:5,t:2,file:'frames/linked.png'});
    await writeFile(join(bundle,'apparent.json'),JSON.stringify(symlinkManifest));
    assert.equal(run(bundle,'--images').status,2);
    await writeFile(join(bundle, 'apparent.json'), raw.replace('"segments":[]', '"segments":[{"t0":"bad","t1":2,"text":"x"}]'));
    assert.equal(run(bundle).status, 2);
    await writeFile(join(bundle, 'apparent.json'),raw);
    assert.equal(run('https://example.com/id').status, 2);
    assert.equal(run(bundle, '--json', '--marks').status, 0);
    assert.equal(run(bundle, '--experiment', 'agentContext').status, 0);
    assert.equal(run(bundle, '--experiment').status, 2);
    assert.match(spawnSync(process.execPath, [cli, '--help'], { encoding: 'utf8' }).stdout, /Usage:/);
    assert.equal(spawnSync(process.execPath, [cli, '--help'], { encoding: 'utf8' }).status, 0);
    await writeFile(join(bundle, 'apparent.json'), '{'); assert.equal(run(bundle).status, 2);
    await writeFile(join(bundle, 'apparent.json'), '{"version":99}'); assert.equal(run(bundle).status, 2);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('explicit extensionless evidence directories and apparent.json paths work through inspect, frames, and zoom',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-explicit-bundle-'));
  try{
    const bundle=join(root,"Review with spaces 👋"),frames=join(bundle,'frames');await mkdir(frames,{recursive:true});
    const image=new PNG({width:2,height:1});image.data.set([10,20,30,255,40,50,60,255]);const png=PNG.sync.write(image);
    await writeFile(join(bundle,'contact-sheet.png'),png);await writeFile(join(frames,'scene one.png'),png);
    const manifest={version:1,id:'extensionless-snapshot',createdAtMs:1,durationMs:2000,target,transcript:{text:'The screen changed.',segments:[{t0:900,t1:1400,text:'The screen changed.'}],words:[{t0:900,t1:1100,text:'The'},{t0:1100,t1:1250,text:'screen'},{t0:1250,t1:1400,text:'changed.'}]},strokes:[],marks:[{id:'mark-one',strokes:[],t0:1000,t1:1200,boxNorm:[0,0,1,1],shape:'circle',description:'screen change',frame:'frames/scene one.png'}],events:[],keyframes:[{index:1,t:1000,file:'frames/scene one.png'}],contactSheet:'contact-sheet.png'};
    const manifestBytes=Buffer.from(JSON.stringify(manifest));await writeFile(join(bundle,'apparent.json'),manifestBytes);
    const receipt=Buffer.from(JSON.stringify({version:1,captureId:'e'.repeat(32),server:'https://connect.example',media:'not downloaded',guidance:'Use apparent fetch <id> --media --out <new-directory> for the complete recording'}));
    await writeFile(join(bundle,'.evidence-only.json'),receipt);
    const cli=new URL('../dist/cli.js',import.meta.url).pathname;
    const run=(...args:string[])=>spawnSync(process.execPath,[cli,...args],{encoding:'utf8',env:{...process.env,APPARENT_ENGINE_FRAMES_EXECUTABLE:'/missing/provider',APPARENT_ENGINE_RUNTIME:'/missing/runtime'}});
    const images=run('inspect',bundle,'--images');assert.equal(images.status,0,images.stderr);
    const imageData=JSON.parse(images.stdout);assert.deepEqual(imageData.references,['contact-sheet.png','frames/scene one.png']);
    assert.deepEqual(imageData.paths,[join(bundle,'contact-sheet.png'),join(frames,'scene one.png')]);assert.deepEqual(imageData.missingReferences,[]);
    assert.equal(run('inspect',join(bundle,'apparent.json'),'--transcript').stdout,'The screen changed.\n');
    assert.match(run('inspect',bundle,'--marks').stdout,/mark-one/);
    const frameResult=run('frames',bundle,'--from','1','--to','1.5','--json');assert.equal(frameResult.status,0,frameResult.stderr);
    const selected=JSON.parse(frameResult.stdout);assert.equal(selected.mode,'saved-keyframes');assert.equal(selected.frames[0].timestampMs,1000);assert.equal(selected.frames[0].path,join(frames,'scene one.png'));
    const zoomOut=join(root,'zoom-result');
    const zoomResult=run('zoom',join(bundle,'apparent.json'),'--at','screen changed','--frames','1','--out',zoomOut,'--json');assert.equal(zoomResult.status,0,zoomResult.stderr);
    const zoom=JSON.parse(zoomResult.stdout);assert.equal(zoom.sourceKind,'saved-keyframes');assert.equal(zoom.cells[0].timestampMs,1000);assert.equal(zoom.cells[0].framePath,join(frames,'scene one.png'));
    assert.deepEqual(await readFile(join(bundle,'apparent.json')),manifestBytes);assert.deepEqual(await readFile(join(bundle,'.evidence-only.json')),receipt);assert.deepEqual(await readFile(join(frames,'scene one.png')),png);
    const directJson=run('inspect',join(bundle,'apparent.json'),'--json');assert.equal(directJson.stdout,manifestBytes.toString());
  }finally{await rm(root,{recursive:true,force:true});}
});

test('explicit paths require a bounded valid v1 manifest and reject invalid UTF8 or symlink manifests',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-invalid-explicit-'));
  try{
    const cli=new URL('../dist/cli.js',import.meta.url).pathname;
    const run=(path:string)=>spawnSync(process.execPath,[cli,'inspect',path],{encoding:'utf8'});
    const empty=join(root,'ordinary-folder');await mkdir(empty);
    let result=run(empty);assert.equal(result.status,2);assert.equal(result.stdout,'');assert.match(result.stderr,/Cannot read regular apparent\.json/);
    const manifestPath=join(empty,'apparent.json');
    await writeFile(manifestPath,'{');result=run(empty);assert.equal(result.status,2);assert.match(result.stderr,/apparent\.json is not valid JSON/);
    await writeFile(manifestPath,Buffer.from([0xff,0xfe]));result=run(manifestPath);assert.equal(result.status,2);assert.match(result.stderr,/not valid UTF-8/);
    await writeFile(manifestPath,JSON.stringify({version:99}));result=run(empty);assert.equal(result.status,2);assert.match(result.stderr,/Unsupported bundle version: 99/);
    const targetPath=join(root,'manifest-target');await writeFile(targetPath,JSON.stringify({version:1}));await rm(manifestPath);await symlink(targetPath,manifestPath);
    result=run(manifestPath);assert.equal(result.status,2);assert.match(result.stderr,/Cannot read regular apparent\.json/);
  }finally{await rm(root,{recursive:true,force:true});}
});

test('latest skips incomplete manifests; bounded strict reads and option parsing fail safely', async () => {
  const root = await mkdtemp(join(tmpdir(), 'apparent-agent-latest-'));
  try {
    const good = join(root, 'good.apparent'); const broken = join(root, 'broken.apparent');
    await mkdir(good); await mkdir(broken);
    const manifest = {version:1,id:'good',createdAtMs:1,durationMs:1,target,transcript:{text:'',segments:[]},
      strokes:[],marks:[null],events:[],keyframes:[],contactSheet:'contact.png'};
    const valid = {...manifest, marks: []};
    await writeFile(join(good, 'apparent.json'), JSON.stringify(valid)); await writeFile(join(good, 'summary.md'), 'ok');
    await writeFile(join(good, 'contact.png'), 'synthetic');
    await writeFile(join(broken, 'apparent.json'), JSON.stringify({...manifest, createdAtMs:99}));
    const cli = new URL('../dist/cli.js', import.meta.url).pathname;
    const run = (...args: string[]) => spawnSync(process.execPath, [cli, 'inspect', ...args], { encoding: 'utf8', env: {...process.env, APPARENT_CAPTURES_DIR: root} });
    assert.match(run('latest').stdout, /apparent 0:00/);
    assert.equal(run('latest', '--stored-summary').stdout, 'ok\n');
    assert.equal(run('--', '-bundle').status, 2);
    assert.equal(run(good, '--transcript', '--images').status, 2);
    await writeFile(join(good, 'apparent.json'), Buffer.from([0xff, 0xfe]));
    assert.match(run(good, '--json').stderr, /UTF-8/);
    await writeFile(join(good, 'apparent.json'), '{}'); await truncate(join(good, 'apparent.json'), 64 * 1024 * 1024 + 1);
    assert.match(run(good, '--json').stderr, /64 MiB/);
    await writeFile(join(good, 'apparent.json'), JSON.stringify(valid)); await rm(join(good, 'summary.md'));
    assert.equal(run(good).status, 0); // Missing producer text is generated from the manifest.
    assert.equal(run('latest').status, 0); // latest no longer requires stored summary.md.
    assert.notEqual(run(good, '--stored-summary').status, 0);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('latest honors read-only capturesDir config and an unavailable explicit location errors', async () => {
  const root = await mkdtemp(join(tmpdir(), 'apparent-agent-config-'));
  try {
    const captures = join(root, 'captures with spaces'); await mkdir(captures);
    const bundle = join(captures, 'saved.apparent'); await mkdir(bundle);
    const manifest = {version:1,id:'saved',createdAtMs:2,durationMs:0,target,transcript:{text:'legacy',segments:[]},
      strokes:[],marks:[],events:[],keyframes:[],contactSheet:'sheet.png'};
    await writeFile(join(bundle, 'apparent.json'), JSON.stringify(manifest)); await writeFile(join(bundle, 'summary.md'), 'configured');
    await writeFile(join(bundle, 'sheet.png'), 'synthetic');
    const configDir = join(root, '.config', 'apparent'); await mkdir(configDir, {recursive:true});
    await writeFile(join(configDir, 'config.json'), JSON.stringify({capturesDir:'~/captures with spaces', unrelated:'kept'}));
    const configBytes = await readFile(join(configDir, 'config.json'));
    const cli = new URL('../dist/cli.js', import.meta.url).pathname;
    const configured = spawnSync(process.execPath, [cli, 'inspect', 'latest', '--stored-summary'], {encoding:'utf8', env:{...process.env, HOME:root}, cwd:root});
    assert.equal(configured.stdout, 'configured\n');
    const shortId=spawnSync(process.execPath,[cli,'inspect','saved','--stored-summary'],{encoding:'utf8',env:{...process.env,HOME:root},cwd:root});
    assert.equal(shortId.status,0,shortId.stderr);assert.equal(shortId.stdout,'configured\n');
    assert.deepEqual(await readFile(join(configDir, 'config.json')), configBytes);
    const empty = spawnSync(process.execPath, [cli, 'inspect', 'latest'], {encoding:'utf8', env:{...process.env, HOME:root, APPARENT_CAPTURES_DIR:''}});
    assert.equal(empty.status, 2); assert.match(empty.stderr, /non-empty path/);
    const missing = spawnSync(process.execPath, [cli, 'inspect', 'latest'], {encoding:'utf8', env:{...process.env, APPARENT_CAPTURES_DIR:join(root, 'missing'), HOME:root}});
    assert.equal(missing.status, 2); assert.match(missing.stderr, /Cannot read captures directory/);
    const operand = spawnSync(process.execPath, [cli, 'inspect', '--', bundle], {encoding:'utf8', cwd:root});
    assert.equal(operand.status, 0);
  } finally { await rm(root, {recursive:true, force:true}); }
});

test('record refuses to imply readiness without explicit Engine paths', async () => {
  const cli = new URL('../dist/cli.js', import.meta.url).pathname;
  const result = spawnSync(process.execPath, [cli, 'record', '--duration', '1', '--out', '/tmp/apparent-agent-never-created.apparent'], {
    encoding: 'utf8', env: {...process.env, APPARENT_ENGINE_EXECUTABLE: '', APPARENT_ENGINE_RUNTIME: ''},
  });
  assert.equal(result.status, 2); assert.equal(result.stdout, '');
  assert.match(result.stderr, /explicit absolute paths/);
});

test('record defaults duration to five seconds and rejects invalid or duplicate values before effects', async () => {
  const root = await mkdtemp(join(tmpdir(), 'apparent-agent-record-default-'));
  try {
    const cli = new URL('../dist/cli.js', import.meta.url).pathname;
    const out = join(root, 'omitted.apparent');
    const env = { ...process.env, APPARENT_ENGINE_EXECUTABLE: '', APPARENT_ENGINE_RUNTIME: '' };
    const omitted = spawnSync(process.execPath, [cli, 'record', '--out', out], { encoding: 'utf8', env });
    assert.equal(omitted.status, 2);
    assert.match(omitted.stderr, /Set APPARENT_ENGINE_EXECUTABLE and APPARENT_ENGINE_RUNTIME/);
    assert.equal(omitted.stdout, '');
    const maximum = spawnSync(process.execPath, [cli, 'record', '--duration', '3600', '--out', join(root, 'maximum.apparent')], { encoding: 'utf8', env });
    assert.equal(maximum.status, 2);
    assert.match(maximum.stderr, /Set APPARENT_ENGINE_EXECUTABLE and APPARENT_ENGINE_RUNTIME/);
    const invalidArgs = [
      ['--duration', '0'], ['--duration', '3601'], ['--duration', 'invalid'],
      ['--duration'], ['--duration', '2', '--duration', '3'],
    ];
    for (const [index, invalid] of invalidArgs.entries()) {
      const destination = join(root, `invalid-${index}.apparent`);
      const result = spawnSync(process.execPath, [cli, 'record', ...invalid, '--out', destination], { encoding: 'utf8', env });
      assert.equal(result.status, 2);
      assert.equal(result.stdout, '');
      assert.equal(result.stderr.includes('Set APPARENT_ENGINE_EXECUTABLE'), false);
      await assert.rejects(readFile(destination));
    }
    const missing = spawnSync(process.execPath, [cli, 'record', '--duration', '2'], { encoding: 'utf8', env });
    assert.equal(missing.status, 2); assert.equal(missing.stderr.includes('Set APPARENT_ENGINE_EXECUTABLE'), false);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('foreground adapter starts only when ready and returns only a saved native receipt', async () => {
  const calls: string[] = []; let state = 'recording';
  const client = {
    async handshake() { return {protocolVersion:1, implementation:'native', readiness:'ready', capabilities:{screenCapture:true,audioCapture:false,windowCapture:false}}; },
    prepareStart() { return {captureId:'capture', ownerToken:'owner'}; },
    async start() { calls.push('start'); return {captureId:'capture', ownerToken:'owner', receipt:{state:'recording'}}; },
    async control(method: string) {
      calls.push(method);
      if (method === 'stop') state = 'stopping';
      if (method === 'inspect' && state === 'stopping') state = 'saved';
      return {state, bundlePath:state === 'saved' ? '/tmp/synthetic.apparent' : undefined, artifactProvenance:'native', artifactVersion:1};
    },
  } as unknown as RecordingClient;
  const saved = await recordWithEngine(client, '/tmp/synthetic.apparent', 0.001);
  assert.equal(saved, '/tmp/synthetic.apparent'); assert.equal(calls[0], 'start'); assert.equal(calls.filter(c => c === 'stop').length, 1); assert.equal(calls.at(-1), 'inspect');
});

test('foreground adapter cancels a start that never becomes ready', async () => {
  const calls: string[] = []; let state = 'starting'; let checks = 0;
  const client = {
    async handshake() { return {protocolVersion:1, implementation:'native', readiness:'ready', capabilities:{screenCapture:true,audioCapture:false,windowCapture:false}}; },
    prepareStart() { return {captureId:'capture', ownerToken:'owner'}; },
    async start() { return {captureId:'capture', ownerToken:'owner', receipt:{state:'starting'}}; },
    async control(method: string) {
      calls.push(method);
      if (method === 'inspect' && checks++ === 0) return {state:'starting'};
      if (method === 'stop') throw Object.assign(new Error('not ready'), {code:'not_ready'});
      if (method === 'cancel') state = 'cancelled';
      return {state};
    },
  } as unknown as RecordingClient;
  await assert.rejects(recordWithEngine(client, '/tmp/synthetic.apparent', 1, () => checks > 0), /state cancelled/);
  assert.deepEqual(calls, ['inspect', 'cancel']);
});
