import {execFileSync} from 'node:child_process';
import {mkdtemp,writeFile,rm,mkdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {admitWorker,verifyArchive} from './provenance.ts';
import {validate,effectiveEnvironment} from './artifacts.ts';
import {selection} from '../../enablement/verification/web/baseline.ts';
export function compatible(actual:unknown,expected:unknown){const canonical=(v:unknown)=>JSON.stringify(v,(_,x)=>x&&typeof x==='object'&&!Array.isArray(x)?Object.fromEntries(Object.entries(x).sort()):x);return canonical(actual)===canonical(expected);}
export async function select(force=false){
 if(force)return '';
 const token=process.env.GH_TOKEN;if(!token)throw Error('missing GitHub token');
 async function api(path:string){const r=await fetch(`https://api.github.com/repos/tryapparent/apparent/${path}`,{headers:{Authorization:`Bearer ${token}`,Accept:'application/vnd.github+json'}});if(!r.ok)throw Error(`GitHub ${r.status}`);return r;}
 const workflow=await(await api('actions/workflows/connect-delivery.yml')).json();
 for(let page=1;;page++){
 const listing=await(await api(`actions/workflows/connect-delivery.yml/runs?branch=main&status=success&per_page=100&page=${page}`)).json();
 if(!Array.isArray(listing.workflow_runs))throw Error('invalid run listing');
 if(listing.workflow_runs.length===0)break;
 for(const run of listing.workflow_runs){
 const dir=await mkdtemp(join(tmpdir(),'connect-selection-'));
 try{
 execFileSync('git',['merge-base','--is-ancestor',run.head_sha,'HEAD']);
 const artifacts=await(await api(`actions/runs/${run.id}/artifacts?per_page=100`)).json();if(artifacts.total_count>100)throw Error('artifact limit');
 for(const kind of ['connect-browser','connect-worker']){
 const matches=artifacts.artifacts.filter((a:any)=>a.name===`${kind}-${run.head_sha}`);if(matches.length!==1)throw Error('missing artifact');const artifact=matches[0];
 admitWorker(run,workflow,{...artifact,name:`connect-worker-${run.head_sha}`},run.head_sha);
 const bytes=new Uint8Array(await(await api(`actions/artifacts/${artifact.id}/zip`)).arrayBuffer());verifyArchive(bytes,artifact.digest);
 const zip=join(dir,`${kind}.zip`);await writeFile(zip,bytes);
 const names=execFileSync('unzip',['-Z1',zip],{encoding:'utf8'}).trim().split('\n');
 if(new Set(names).size!==names.length||names.some(n=>n!=='manifest.json'&&(!n.startsWith('dist/')||n.split('/').some(s=>s==='..'||s==='.')||n.includes('\\'))))throw Error('unsafe archive');
 const payload=join(dir,kind);await mkdir(payload);
 for(const name of names){if(name.endsWith('/'))continue;await mkdir(resolve(payload,name,'..'),{recursive:true});await writeFile(join(payload,name),execFileSync('unzip',['-p',zip,name],{maxBuffer:64*1024*1024}),{flag:'wx'});}
 const manifest=await validate(process.cwd(),payload,kind);if(manifest.build.node!=='22.23.0'||manifest.build.bun!=='1.3.10'||manifest.sourceSha!==run.head_sha||!compatible(manifest.build.environment,effectiveEnvironment(kind)))throw Error('production environment changed');
 }
 return String(run.id);
 }catch{ /* Missing, stale or incompatible evidence requires a producer. */ }finally{await rm(dir,{recursive:true,force:true});}
 }
 }
 return '';
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href){
 const impact=await selection('connect',process.env.GITHUB_SHA??'',process.env.GH_TOKEN??'',process.env.GITHUB_EVENT_NAME==='workflow_dispatch');
 const selected=await select(impact.required);const {appendFile}=await import('node:fs/promises');
 await appendFile(process.env.GITHUB_OUTPUT!,`selected-run=${selected}\nbuild=${selected===''}\napplicable=true\n`);
 console.log(JSON.stringify({...impact,selectedRun:selected,build:selected===''}));
}
