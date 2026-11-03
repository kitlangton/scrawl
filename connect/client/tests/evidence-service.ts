// Production CLI ↔ local Worker/D1/R2 integration. No desktop, real identity,
// captures, permission changes or deployed service access.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {spawn, spawnSync} from 'node:child_process';
import {mkdir, mkdtemp, readFile, writeFile, readdir, realpath, rm, rename, copyFile} from 'node:fs/promises';
import {join, resolve, dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {tmpdir} from 'node:os';
import {fixture} from '../../service/test/helpers.mjs';

process.umask(0o077);
const binary = await realpath(process.argv[2] ?? resolve(dirname(fileURLToPath(import.meta.url)), '../../../target/debug/apparent'));
const serviceDir = resolve(dirname(fileURLToPath(import.meta.url)), '../../service');
process.chdir(serviceDir);
const root = await realpath(await mkdtemp(join(tmpdir(), 'apparent-evidence-service-')));
const children = new Set<ReturnType<typeof spawn>>();
let service: Awaited<ReturnType<typeof fixture>>;
let failMedia = true;
const server = createServer(async (req, res) => {
  try {
    assert(req.url?.startsWith('/v1/'));
    if (failMedia && req.method === 'PUT' && req.url.includes('/files/media/')) {
      req.resume(); res.writeHead(503); res.end('{"error":{"code":"temporary_unavailable"}}'); return;
    }
    const chunks: Buffer[] = []; let size = 0;
    for await (const chunk of req) { size += chunk.length; assert(size < 1024 * 1024); chunks.push(chunk); }
    const response = await service.mf.dispatchFetch(origin + req.url, {method:req.method, headers:req.headers,
      ...(chunks.length ? {body:Buffer.concat(chunks)} : {})});
    res.writeHead(response.status, Object.fromEntries(response.headers));
    res.end(Buffer.from(await response.arrayBuffer()));
  } catch { res.writeHead(500); res.end('{"error":{"code":"fixture_error"}}'); }
});
await new Promise<void>(r => server.listen(0, '127.0.0.1', r));
const address = server.address(); assert(address && typeof address !== 'string');
const origin = `http://127.0.0.1:${address.port}`;
const sender = join(root, 'sender'), receiver = join(root, 'receiver');
async function cli(home: string, args: string[], onData = (_: string) => {}) {
  return new Promise<string>((resolveResult, reject) => {
    const child = spawn(binary, args, {cwd:home, env:{...process.env, HOME:home,
      APPARENT_CONFIG:join(home, 'config.json'), XDG_CONFIG_HOME:join(home, '.config')}, stdio:['ignore','pipe','pipe']});
    children.add(child); let out = '', err = '';
    const timer = setTimeout(() => child.kill('SIGKILL'), 30000);
    child.stdout!.on('data', b => { out += b; if(out.length > 65536) child.kill('SIGKILL'); else onData(out); });
    child.stderr!.on('data', b => { err += b; if(err.length > 65536) child.kill('SIGKILL'); });
    child.on('error', reject);
    child.on('close', code => { clearTimeout(timer); children.delete(child);
      if(code === 0) resolveResult(out); else reject(new Error(`CLI ${args[0]} failed: ${err}`)); });
  });
}
try {
  service = await fixture({API_ORIGIN:origin, SITE_ORIGIN:origin});
  const account = await service.identity('evidence-cli-synthetic');
  for (const home of [sender, receiver]) {
    await mkdir(home, {mode:0o700});
    await writeFile(join(home, 'config.json'), JSON.stringify({cloud:{server:origin,enabled:false,uploadConsent:false}}));
  }
  async function login(home: string, scope: 'read' | 'upload') {
    let approval: Promise<void> | undefined;
    await cli(home, ['connect','login','--server',origin,'--scope',scope,'--no-browser',
      ...(scope === 'upload' ? ['--consent-upload'] : [])], text => {
      const code = /Verification code: ([A-Za-z0-9-]+)/.exec(text)?.[1];
      if(!code || approval) return;
      approval = service.call('/v1/device/approve', {...account, origin, body:{userCode:code}, method:'POST'})
        .then(response => assert.equal(response.status,200));
      approval.catch(() => {});
    });
    assert(approval); await approval;
  }
  await login(receiver, 'read'); await login(sender, 'upload');
  const bundle = join(sender, 'synthetic.apparent');
  await mkdir(join(bundle, 'frames'), {recursive:true,mode:0o700});
  const metadata = {version:1,id:'synthetic',createdAtMs:1,durationMs:200,
    target:{mode:'display',displayId:1,frame:{x:0,y:0,width:64,height:64,scale:1}},
    transcript:{source:'synthetic',text:'Inspect this gold square',segments:[]},
    strokes:[],marks:[],events:[],keyframes:[{index:1,t:0,reason:'initial',file:'frames/frame-01.png',event:'Synthetic gold square'}],
    contactSheet:'contact-sheet.png',video:'recording.mp4',audio:'audio.wav'};
  const wav=Buffer.alloc(3244);wav.write('RIFF');wav.writeUInt32LE(3236,4);wav.write('WAVEfmt ',8);
  wav.writeUInt32LE(16,16);wav.writeUInt16LE(1,20);wav.writeUInt16LE(1,22);wav.writeUInt32LE(16000,24);
  wav.writeUInt32LE(32000,28);wav.writeUInt16LE(2,32);wav.writeUInt16LE(16,34);wav.write('data',36);wav.writeUInt32LE(3200,40);
  await writeFile(join(bundle,'audio.wav'),wav);
  await writeFile(join(bundle,'apparent.json'),JSON.stringify(metadata));
  await writeFile(join(bundle,'summary.md'),'Synthetic evidence: inspect this gold square.');
  assert.equal(spawnSync('ffmpeg',['-v','error','-f','lavfi','-i','color=c=gold:s=64x64:r=10','-t','0.2',
    '-c:v','libx264','-pix_fmt','yuv420p',join(bundle,'recording.mp4')],{timeout:15000}).status,0);
  assert.equal(spawnSync('ffmpeg',['-v','error','-i',join(bundle,'recording.mp4'),'-frames:v','1',
    join(bundle,'contact-sheet.png')],{timeout:15000}).status,0);
  await copyFile(join(bundle,'contact-sheet.png'),join(bundle,'frames/frame-01.png'));
  await cli(sender,['connect','upload',bundle]);
  const jobs = (await readdir(join(sender,'hosted'))).filter(n=>n.endsWith('.job'));
  assert.equal(jobs.length,1);
  const before = JSON.parse(await readFile(join(sender,'hosted',jobs[0]),'utf8'));
  assert.equal(before.status,'pending'); assert(before.grouped);
  const unavailable = join(root,'source-offline'); await rename(bundle,unavailable);
  const summary = await cli(receiver,['inspect',before.id]);
  assert(summary.includes('gold square'));
  assert(!summary.includes('No microphone audio was saved'));
  const evidence = join(receiver,`${before.id}.evidence.apparent`);
  assert.equal(await readFile(join(evidence,'summary.md'),'utf8'),'Synthetic evidence: inspect this gold square.');
  assert.deepEqual(await readFile(join(evidence,'frames/frame-01.png')),await readFile(join(unavailable,'frames/frame-01.png')));
  await assert.rejects(readFile(join(evidence,'recording.mp4')));
  assert(JSON.parse(await readFile(join(evidence,'.evidence-only.json'),'utf8')).captureId===before.id);
  assert((await cli(receiver,['inspect',evidence])).includes('gold square'));
  await assert.rejects(cli(receiver,['zoom',evidence,'--at','0:00']),/Evidence snapshot.*fetch/);
  await assert.rejects(cli(receiver,['fetch',before.id,'--media','--out','pending.apparent','--wait','0']),/pending/);
  failMedia = false; await cli(sender,['connect','retry']);
  const after = JSON.parse(await readFile(join(sender,'hosted',jobs[0]),'utf8'));
  assert.equal(after.status,'ready'); assert.equal(after.id,before.id); assert.deepEqual(after.files,before.files);
  await cli(receiver,['fetch',before.id,'--media','--out','complete.apparent']);
  for(const name of ['apparent.json','summary.md','contact-sheet.png','frames/frame-01.png','recording.mp4','audio.wav']) {
    assert.deepEqual(await readFile(join(receiver,'complete.apparent',name)),await readFile(join(unavailable,name)));
  }
  if(process.argv[3]) {
    const evidenceOut = resolve(process.argv[3]);
    await mkdir(evidenceOut,{mode:0o700});
    for(const name of ['summary.md','contact-sheet.png','apparent.json','.evidence-only.json']) {
      await copyFile(join(evidence,name),join(evidenceOut,name));
    }
    await writeFile(join(evidenceOut,'inspect.txt'),summary);
  }
  console.log('PASS: real CLI/Worker evidence before media, offline source, same-ID retry, complete byte equality');
} finally {
  for(const child of children) child.kill('SIGKILL');
  server.closeAllConnections(); await new Promise<void>(r=>server.close(()=>r()));
  await service?.close(); await rm(root,{recursive:true,force:true});
}
