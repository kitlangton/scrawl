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
