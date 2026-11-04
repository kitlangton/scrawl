import {test} from 'node:test';
import assert from 'node:assert/strict';
import {compatible,select} from './select.ts';
test('manual force never consults prior artifacts',async()=>assert.equal(await select(true),''));
test('production environment changes invalidate reuse',()=>{
 assert(compatible({token:'a'},{token:'a'}));
 assert(!compatible({token:'a'},{token:'b'}));
 assert(!compatible({token:'a'},{token:'a',added:true}));
 assert(!compatible({token:'a',removed:true},{token:'a'}));
});

test('authenticated pagination reaches direct producer beyond thirty receipt-only runs',async()=>{
 const {execFileSync}=await import('node:child_process');
 const {mkdtemp,mkdir,writeFile,readFile,rm}=await import('node:fs/promises');
 const {tmpdir}=await import('node:os');const {join}=await import('node:path');
 const {createHash}=await import('node:crypto');const {seal,ROUTES}=await import('./artifacts.ts');
 const {downloadWorker}=await import('./provenance.ts');
 const root=await mkdtemp(join(tmpdir(),'selection-api-')),old=process.cwd(),fetchBefore=globalThis.fetch,token=process.env.GH_TOKEN;
 const git=(...args:string[])=>execFileSync('git',args,{cwd:root,encoding:'utf8'}).trim();
 try{
 for(const p of ['connect/web/scripts','connect/service','connect/delivery','shared/ui','shared/analytics','shared/assets','public/site/public/assets','.github/workflows','.github/actions/runtime-budget','enablement/verification'])await mkdir(join(root,p),{recursive:true});
 for(const p of ['connect/service/source.ts','connect/delivery/source.ts','shared/ui/source.ts','shared/analytics/source.ts','shared/assets/source.ts','.github/workflows/connect-delivery.yml','.github/actions/runtime-budget/source.ts','enablement/verification/workflow-budget.ts'])await writeFile(join(root,p),'fixture');
 await writeFile(join(root,'connect/web/scripts/assets.mjs'),Array.from({length:5},(_,i)=>`['${i}.svg', 'unused']`).join('\n'));
 for(let i=0;i<5;i++)await writeFile(join(root,`public/site/public/assets/${i}.svg`),'<svg/>');
 git('init');git('add','.');git('-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','source');const sha=git('rev-parse','HEAD');process.chdir(root);process.env.GH_TOKEN='synthetic';
 const blobs=new Map<number,Buffer>();const artifacts:any[]=[];
 for(const [i,kind] of ['connect-browser','connect-worker'].entries()){
 const dir=join(root,`payload-${i}`);await mkdir(join(dir,'dist'),{recursive:true});
 if(i===0)for(const route of ROUTES){await mkdir(join(dir,'dist',route),{recursive:true});await writeFile(join(dir,'dist',route,'index.html'),'<html>fixture</html>');}else await writeFile(join(dir,'dist/worker.js'),'export default {}');
 const m=await seal(root,dir,kind);m.build.node='22.23.0';m.build.bun='1.3.10';await writeFile(join(dir,'manifest.json'),JSON.stringify(m));
 const zip=join(root,`${i}.zip`);execFileSync('zip',['-qrD',zip,'manifest.json','dist'],{cwd:dir});const bytes=await readFile(zip);blobs.set(i+1,bytes);
 artifacts.push({id:i+1,name:`${kind}-${sha}`,workflow_run:{id:1,head_sha:sha,head_branch:'main'},expired:false,expires_at:'2099-01-01',digest:`sha256:${createHash('sha256').update(bytes).digest('hex')}`});
 }
 const path='.github/workflows/connect-delivery.yml';const run=(id:number)=>({id,workflow_id:2,path,repository:{full_name:'tryapparent/apparent'},head_repository:{full_name:'tryapparent/apparent'},head_branch:'main',head_sha:sha,event:'push',status:'completed',conclusion:'success',created_at:id===1?'2026-01-01':'2026-02-01'});
 await writeFile(join(root,'website.txt'),'descendant');git('add','website.txt');git('-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','website');
 const receiptDir=join(root,'receipt');await mkdir(receiptDir);await writeFile(join(receiptDir,'connect-selection.json'),JSON.stringify({schema:1,selectedRun:'1'}));execFileSync('zip',['-q',join(root,'receipt.zip'),'connect-selection.json'],{cwd:receiptDir});const receipt=await readFile(join(root,'receipt.zip'));blobs.set(3,receipt);
 const receiptArtifact={...artifacts[0],id:3,name:`connect-selection-${sha}`,workflow_run:{id:200,head_sha:sha,head_branch:'main'},digest:`sha256:${createHash('sha256').update(receipt).digest('hex')}`};
 const pages:number[]=[];
 globalThis.fetch=async(input:any,options:any)=>{assert.equal(options.headers.Authorization,'Bearer synthetic');const p=String(input).split('/apparent/')[1];let value:any;
 if(p==='actions/workflows/connect-delivery.yml')value={id:2,path,state:'active'};
 else if(p.includes('/runs?')){const page=Number(new URL(String(input)).searchParams.get('page'));pages.push(page);value={workflow_runs:page===1?Array.from({length:100},(_,i)=>run(199-i)):page===2?[run(1)]:[]};}
 else if(p.endsWith('/artifacts?per_page=100'))value={total_count:p.includes('/1/')?2:p.includes('/200/')?1:0,artifacts:p.includes('/1/')?artifacts:p.includes('/200/')?[receiptArtifact]:[]};
 else if(p==='actions/runs/200')value=run(200);else if(p==='actions/runs/1')value=run(1);
 else if(/actions\/artifacts\/\d+\/zip/.test(p))return new Response(blobs.get(Number(p.split('/')[2])));
 else throw Error(`unexpected API ${p}`);return Response.json(value);};
 assert.equal(await select(),'1');assert.deepEqual(pages,[1,2]);
 const downloaded=await downloadWorker('200',join(root,'download'));assert.equal(downloaded.sourceSha,sha);
 for(const patch of [{expired:true},{digest:'sha256:'+'0'.repeat(64)},{workflow_run:{id:1,head_sha:sha,head_branch:'foreign'}}]){
 const original={...artifacts[0]};Object.assign(artifacts[0],patch);assert.equal(await select(),'');Object.assign(artifacts[0],original);
 }
 const receiptOriginal=blobs.get(3)!;blobs.set(3,Buffer.from('tampered'));await assert.rejects(downloadWorker('200',join(root,'rejected')));blobs.set(3,receiptOriginal);
 const priorToken=process.env.PUBLIC_POSTHOG_TOKEN;process.env.PUBLIC_POSTHOG_TOKEN='phc_changed';assert.equal(await select(),'');if(priorToken===undefined)delete process.env.PUBLIC_POSTHOG_TOKEN;else process.env.PUBLIC_POSTHOG_TOKEN=priorToken;
 await writeFile(join(root,'connect/web/added.ts'),'new');assert.equal(await select(),'');await rm(join(root,'connect/web/added.ts'));
 await rm(join(root,'shared/ui/source.ts'));assert.equal(await select(),'');await writeFile(join(root,'shared/ui/source.ts'),'fixture');
 await writeFile(join(root,'shared/ui/source.ts'),'changed');assert.equal(await select(),'');
 assert.equal(await select(true),'');
 }finally{globalThis.fetch=fetchBefore;process.chdir(old);if(token===undefined)delete process.env.GH_TOKEN;else process.env.GH_TOKEN=token;await rm(root,{recursive:true,force:true});}
});
