import {createHash} from 'node:crypto';

// Metadata must come directly from authenticated GitHub API responses, never
// from the downloaded archive or workflow event alone.
export function admitWorker(run:any, workflow:any, artifact:any, candidate:string, now=Date.now()) {
 const repo='tryapparent/apparent';
 if(!/^[0-9a-f]{40}$/.test(candidate)||run.repository?.full_name!==repo||run.head_repository?.full_name!==repo||run.head_branch!=='main'||run.head_sha!==candidate||!['push','workflow_dispatch'].includes(run.event)||run.status!=='completed'||run.conclusion!=='success')throw Error('untrusted producer run');
 if(workflow.path!=='.github/workflows/connect-delivery.yml'||workflow.state!=='active'||workflow.id!==run.workflow_id||run.path!==workflow.path)throw Error('untrusted producer workflow');
 if(artifact.workflow_run?.id!==run.id||artifact.workflow_run?.head_sha!==candidate||artifact.workflow_run?.head_branch!=='main'||artifact.name!==`connect-worker-${candidate}`||artifact.expired!==false||!Number.isFinite(Date.parse(artifact.expires_at))||Date.parse(artifact.expires_at)<=now||!Number.isSafeInteger(artifact.id)||artifact.id<=0||!/^sha256:[a-f0-9]{64}$/.test(artifact.digest))throw Error('untrusted worker artifact');
 return {id:artifact.id,digest:artifact.digest,sourceSha:candidate};
}
export function verifyArchive(bytes:Uint8Array,expected:string){
 if(`sha256:${createHash('sha256').update(bytes).digest('hex')}`!==expected)throw Error('Actions archive digest mismatch');
}

export async function downloadWorker(runId:string, destination:string) {
 const {execFileSync}=await import('node:child_process');
 const {writeFile,mkdir}=await import('node:fs/promises');
 const {join}=await import('node:path');
 if(!/^[1-9][0-9]*$/.test(runId))throw Error('invalid run ID');
 const token=process.env.GH_TOKEN;if(!token)throw Error('missing GitHub token');
 async function api(path:string){const r=await fetch(`https://api.github.com/repos/tryapparent/apparent/${path}`,{headers:{Authorization:`Bearer ${token}`,Accept:'application/vnd.github+json'}});if(!r.ok)throw Error(`GitHub ${r.status}`);return r;}
 let run=await (await api(`actions/runs/${runId}`)).json();
 if(run.id!==Number(runId)||!Number.isSafeInteger(Number(runId)))throw Error('run identity mismatch');
 const workflow=await (await api('actions/workflows/connect-delivery.yml')).json();
 const listing=await (await api(`actions/runs/${runId}/artifacts?per_page=100`)).json();
 const receipts=listing.artifacts.filter((a:any)=>a.name===`connect-selection-${run.head_sha}`);
 if(listing.total_count>100||receipts.length!==1)throw Error('missing or ambiguous selection');
 const receipt=receipts[0];
 admitWorker(run,workflow,{...receipt,name:`connect-worker-${run.head_sha}`},run.head_sha);
 execFileSync('git',['merge-base','--is-ancestor',run.head_sha,'HEAD']);
 const receiptBytes=new Uint8Array(await(await api(`actions/artifacts/${receipt.id}/zip`)).arrayBuffer());verifyArchive(receiptBytes,receipt.digest);
 const {mkdtemp,rm}=await import('node:fs/promises');const {tmpdir}=await import('node:os');
 const temporary=await mkdtemp(join(tmpdir(),'connect-receipt-'));
 let selected:string;
 try{
 const archive=join(temporary,'receipt.zip');await writeFile(archive,receiptBytes);
 if(execFileSync('unzip',['-Z1',archive],{encoding:'utf8'}).trim()!=='connect-selection.json')throw Error('unsafe selection archive');
 const raw=execFileSync('unzip',['-p',archive,'connect-selection.json'],{maxBuffer:4096,encoding:'utf8'});
 const value=JSON.parse(raw);
 if(value.schema!==1||Object.keys(value).sort().join(',')!=='schema,selectedRun'||typeof value.selectedRun!=='string'||(value.selectedRun!==''&&(!/^[1-9][0-9]*$/.test(value.selectedRun)||!Number.isSafeInteger(Number(value.selectedRun)))))throw Error('invalid selection receipt');
 selected=value.selectedRun;
 }finally{await rm(temporary,{recursive:true,force:true});}
 let workerListing=listing;
 if(selected!==''){
 if(Number(selected)>=Number(runId))throw Error('selection must reference prior run');
 const trigger=run;run=await(await api(`actions/runs/${selected}`)).json();
 if(run.id!==Number(selected)||Date.parse(run.created_at)>=Date.parse(trigger.created_at))throw Error('invalid prior run');
 execFileSync('git',['merge-base','--is-ancestor',run.head_sha,trigger.head_sha]);
 workerListing=await(await api(`actions/runs/${selected}/artifacts?per_page=100`)).json();
 }
 const matches=workerListing.artifacts.filter((a:any)=>a.name===`connect-worker-${run.head_sha}`);
 if(matches.length!==1||workerListing.total_count>100)throw Error('ambiguous artifact');
 const admitted=admitWorker(run,workflow,matches[0],run.head_sha);
 const bytes=new Uint8Array(await (await api(`actions/artifacts/${admitted.id}/zip`)).arrayBuffer());verifyArchive(bytes,admitted.digest);
 await mkdir(destination,{recursive:false});const zip=join(destination,'archive.zip');await writeFile(zip,bytes);
 // Inspect the complete central directory before extracting. Only the two
 // declared files are accepted, so links, traversal and duplicate paths fail.
 const names=execFileSync('unzip',['-Z1',zip],{encoding:'utf8'}).trim().split('\n');
 validateArchivePaths(names);

 for(const name of names){const data=execFileSync('unzip',['-p',zip,name],{maxBuffer:64*1024*1024});if(name.startsWith('dist/'))await mkdir(join(destination,'dist'));await writeFile(join(destination,name),data,{flag:'wx'});}
 const {unlink}=await import('node:fs/promises');await unlink(zip);
 const {validate}=await import('./artifacts.ts');const manifest=await validate(process.cwd(),destination,'connect-worker');if(manifest.build.node!=='22.23.0'||manifest.build.bun!=='1.3.10'||manifest.sourceSha!==admitted.sourceSha)throw Error('artifact source mismatch');return manifest;
}

export function validateArchivePaths(names:string[]){if(names.length!==2||new Set(names).size!==2||!names.includes('manifest.json')||!names.includes('dist/worker.js'))throw Error('unsafe archive paths');}
