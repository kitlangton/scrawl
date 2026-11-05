#!/usr/bin/env node
/** Offline installer for an admitted Agent tarball and complete Linux media runtime. */
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { constants } from 'node:fs';
import { lstat, mkdir, open, readFile, readdir, copyFile, chmod, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';

const AGENT_SHA = /^[a-f0-9]{64}$/;
const MAX_FILES = 4096, MAX_BYTES = 1_500_000_000, MAX_FILE = 256_000_000;
function fail(message: string): never { throw new Error(message); }
async function hashFile(path: string, limit = MAX_FILE): Promise<string> {
  const st = await lstat(path); if (!st.isFile() || st.isSymbolicLink() || st.size > limit) fail(`unsafe or oversized file: ${path}`);
  const h = createHash('sha256'); const f = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try { const b = Buffer.alloc(128 * 1024); let pos = 0; while (true) { const r = await f.read(b, 0, b.length, pos); if (!r.bytesRead) break; pos += r.bytesRead; h.update(b.subarray(0, r.bytesRead)); } } finally { await f.close(); }
  return h.digest('hex');
}
type Item = {path:string; mode:number; sha256:string};
async function inventory(root: string): Promise<Item[]> {
  const result: Item[] = []; let bytes = 0, entries = 0;
  async function walk(dir: string, prefix = ''): Promise<void> {
    for (const name of await readdir(dir)) {
      if (!prefix && name === 'manifest.json') continue;
      if (!name || name === '.' || name === '..' || name.includes('/') || name.includes('\\')) fail('invalid native path entry');
      const rel = prefix ? `${prefix}/${name}` : name, full = join(dir, name), st = await lstat(full);
      if (++entries > MAX_FILES * 2 || rel.length > 1024) fail('native tree exceeds inventory limits');
      if (st.isSymbolicLink()) fail(`native tree contains symlink: ${rel}`);
      if (st.isDirectory()) { await walk(full, rel); continue; }
      if (!st.isFile()) fail(`native tree contains special entry: ${rel}`);
      if (++result.length > MAX_FILES || st.size > MAX_FILE || (bytes += st.size) > MAX_BYTES) fail('native tree exceeds inventory limits');
      const mode = st.mode & 0o777;
      if (mode !== 0o644 && mode !== 0o755) fail(`invalid native file mode: ${rel}`);
      result[result.length - 1] = { path: rel, mode, sha256: await hashFile(full) };
    }
  }
  await walk(root); return result.sort((a,b) => a.path.localeCompare(b.path));
}
function safeQuote(s: string) { return `'${s.replaceAll("'", "'\\''")}'`; }
function usage(): never { fail('usage: apparent-agent-install-local --agent TARBALL --agent-sha256 HEX --native-root DIR --manifest-sha256 HEX --prefix FRESH_ABSOLUTE_DIR'); }
async function main(args: string[]) {
  const opts: Record<string,string> = {}; for (let i=0;i<args.length;i+=2) { if (!args[i]?.startsWith('--') || !args[i+1]) usage(); opts[args[i].slice(2)] = args[i+1]; }
  const {agent, 'agent-sha256': agentHash, 'native-root': nativeRootArg, 'manifest-sha256': manifestHash, prefix: prefixArg} = opts;
  if (!agent || !AGENT_SHA.test(agentHash ?? '') || !nativeRootArg || !AGENT_SHA.test(manifestHash ?? '') || !prefixArg || !isAbsolute(prefixArg) || resolve(prefixArg)!==prefixArg) usage();
  const [nodeMajor,nodeMinor]=process.versions.node.split('.').map(Number);
  if (process.platform !== 'linux' || process.arch !== 'x64' || nodeMajor < 22 || (nodeMajor === 22 && nodeMinor < 6)) fail('requires Linux x64 and Node.js 22.6+');
  const agentPath=resolve(agent), nativeRoot=resolve(nativeRootArg), prefix=prefixArg;
  if (await hashFile(agentPath, MAX_BYTES) !== agentHash) fail('Agent tarball SHA-256 mismatch');
  const st=await lstat(nativeRoot); if (!st.isDirectory() || st.isSymbolicLink()) fail('native root must be a real directory');
  const manifestPath=join(nativeRoot,'manifest.json'), manifestStat=await lstat(manifestPath); if(!manifestStat.isFile()||manifestStat.isSymbolicLink()||manifestStat.size>16*1024*1024)fail('native manifest must be a bounded regular file');
  const raw=await readFile(manifestPath,{flag:constants.O_RDONLY|constants.O_NOFOLLOW}); if (createHash('sha256').update(raw).digest('hex')!==manifestHash) fail('native manifest SHA-256 mismatch');
  const manifest=JSON.parse(raw.toString('utf8'));
  const expectedKeys=['schema','sourceSha','target','rustTriple','capabilities','launch','environment','runtimeFiles'];
  if (JSON.stringify(Object.keys(manifest).sort())!==JSON.stringify(expectedKeys.slice().sort()) || JSON.stringify(Object.keys(manifest.launch ?? {}).sort())!==JSON.stringify(['entry','argv','worker','pathLookup','runtimeBuilds'].sort()) || manifest.schema!==1 || !/^[a-f0-9]{40}$/.test(manifest.sourceSha) || manifest.target!=='linux-x64' || manifest.rustTriple!=='x86_64-unknown-linux-gnu' || JSON.stringify(manifest.capabilities)!==JSON.stringify(['media/1']) || manifest.launch?.entry!=='apparent-engine-frames' || manifest.launch?.worker!=='libexec/apparent-engine-frames' || manifest.launch?.pathLookup!==false || manifest.launch?.runtimeBuilds!==false || JSON.stringify(manifest.launch?.argv)!==JSON.stringify(['--media-runtime-directory','<private-runtime-dir>','--media-channel','media','--media-protocol-version','1'])) fail('native media manifest contract rejected');
  const before=await inventory(nativeRoot), declared=(manifest.runtimeFiles as Item[]).slice().sort((a,b)=>a.path.localeCompare(b.path));
  if (JSON.stringify(before)!==JSON.stringify(declared) || !before.some(x=>x.path==='apparent-engine-frames') || !before.some(x=>x.path==='libexec/apparent-engine-frames')) fail('native root inventory does not exactly match manifest');
  // Reject symlinked ancestors and any existing destination before creating anything.
  let cur='/'; for (const part of prefix.slice(1).split('/').filter(Boolean)) { cur=join(cur,part); try { const p=await lstat(cur); if (p.isSymbolicLink() || !p.isDirectory()) fail(`unsafe prefix parent: ${cur}`); } catch(e:any) { if(e.code!=='ENOENT') throw e; } }
  try { await lstat(prefix); fail('prefix already exists; choose a fresh path'); } catch(e:any) { if(e.code!=='ENOENT') throw e; }
  await mkdir(prefix,{mode:0o700}); await chmod(prefix,0o700);
  const root=join(prefix,'apparent-agent-private'); await mkdir(root,{mode:0o700});
  try {
    execFileSync('npm',['install','--offline','--ignore-scripts','--no-audit','--no-fund','--prefix',root,agentPath],{stdio:'pipe',timeout:120000,env:{PATH:process.env.PATH ?? '',HOME:process.env.HOME ?? '',npm_config_update_notifier:'false'}});
    const pkg=JSON.parse(await readFile(join(root,'node_modules/@apparent/agent/package.json'),'utf8'));
    if(pkg.version!=='0.1.0'||pkg.bin?.['apparent-agent']!=='./dist/cli.js'||pkg.dependencies?.['@apparent/engine-sdk']!=='file:../engine/sdk') fail('installed Agent package metadata mismatch');
    const sdk=JSON.parse(await readFile(join(root,'node_modules/@apparent/agent/node_modules/@apparent/engine-sdk/package.json'),'utf8'));
    if(sdk.version!=='0.2.0'||sdk.exports?.['./media']===undefined) fail('installed SDK media export/version mismatch');
    const runtime=join(prefix,'native-media'); await mkdir(runtime,{mode:0o700});
    for(const item of before){ const from=join(nativeRoot,item.path), to=join(runtime,item.path); const rel=relative(nativeRoot,from); if(rel.startsWith('..'+sep)||isAbsolute(rel)) fail('native path escape');
      let d=dirname(to), chain:string[]=[]; while(d!==runtime){if(!d.startsWith(runtime+sep))fail('native target escape');chain.push(d);d=dirname(d);}
      for(const dir of chain.reverse()) { try {const q=await lstat(dir);if(!q.isDirectory()||q.isSymbolicLink())fail('unsafe native target directory');}catch(e:any){if(e.code!=='ENOENT')throw e;await mkdir(dir,{mode:0o700});} }
      await copyFile(from,to); await chmod(to,item.mode); if(await hashFile(to)!==item.sha256) fail(`native copy verification failed: ${item.path}`);
    }
    await copyFile(manifestPath,join(runtime,'manifest.json')); await chmod(join(runtime,'manifest.json'),0o600);
    if(await hashFile(join(runtime,'manifest.json'))!==manifestHash||JSON.stringify(await inventory(nativeRoot))!==JSON.stringify(declared)) fail('native source changed during installation');
    if(JSON.stringify(await inventory(runtime))!==JSON.stringify(before)) fail('copied native inventory changed');
    const entry=join(runtime,'apparent-engine-frames'), cli=join(root,'node_modules/@apparent/agent/dist/cli.js');
    const wrapper=`#!/bin/sh\nexport APPARENT_ENGINE_FRAMES_EXECUTABLE=${safeQuote(entry)}\nexport APPARENT_ENGINE_RUNTIME=${safeQuote(join(prefix,'runtime'))}\nexec ${safeQuote(process.execPath)} ${safeQuote(cli)} "$@"\n`;
    await mkdir(join(prefix,'runtime'),{mode:0o700}); await writeFile(join(prefix,'apparent-agent'),wrapper,{mode:0o700});
    execFileSync(process.execPath,[cli,'--version'],{stdio:'pipe',timeout:15000,env:{PATH:'/usr/bin:/bin',HOME:prefix}});
    const mediaModule=pathToFileURL(join(root,'node_modules/@apparent/agent/node_modules/@apparent/engine-sdk/dist/src/media.js')).href;
    const smoke = `const {MediaClient,launchMediaWorker}=await import(${JSON.stringify(mediaModule)}); const c=new MediaClient(launchMediaWorker(${JSON.stringify(entry)},${JSON.stringify(join(prefix,'runtime'))},3000)); let ok=false; try { const h=await c.handshake(); if(h.channel!=='media'||h.protocolVersion!==1||h.implementation!=='native'||h.capabilities?.decodeFrames!==true) throw Error('native media handshake rejected'); ok=true; } finally { const result=await c.close(); if(!['already-exited','eof-exited'].includes(result)) throw Error('native worker close uncertain'); } if(!ok) process.exit(2);`;
    execFileSync(process.execPath,['--input-type=module','-e',smoke],{cwd:root,stdio:'pipe',timeout:20000,env:{PATH:'/usr/bin:/bin',HOME:prefix}});
    const receipt={schema:1,kind:'offline-local-agent-install',sourceSha:manifest.sourceSha,target:manifest.target,agentTarballSha256:agentHash,nativeManifestSha256:manifestHash,agentVersion:pkg.version,sdkVersion:sdk.version,platform:'linux-x64',cliPath:cli,launcher:join(prefix,'apparent-agent'),nativeEntry:entry,nativeWorker:join(runtime,manifest.launch.worker),runtime:join(prefix,'runtime'),claims:['local Agent consumer installed','native media/1 handshake and confirmed close passed; no codec claim','does not imply current-main artifact admission or global routing']};
    await writeFile(join(prefix,'install-receipt.json'),JSON.stringify(receipt,null,2)+'\n',{mode:0o600});
  } catch(e) { throw new Error(`Installation incomplete; private partial prefix retained at ${prefix}: ${(e as Error).message}`); }
}
main(process.argv.slice(2)).catch(e=>{process.stderr.write(`Apparent local installation failed: ${(e as Error).message}\n`);process.exitCode=1;});
