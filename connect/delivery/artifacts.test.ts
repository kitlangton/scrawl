import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,writeFile,readFile,cp,rm,symlink} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';
import {seal,validate,ROUTES} from './artifacts.ts';
test('isolated git provenance and hostile payloads',async()=>{
 const root=await mkdtemp(join(tmpdir(),'delivery-'));const dir=join(root,'artifact');
 const put=async(p:string,s='fixture')=>{await mkdir(join(root,p,'..'),{recursive:true});await writeFile(join(root,p),s)};
 try{
 for(const p of ['connect/web/package.json','connect/service/package.json','shared/ui/a','shared/analytics/a','shared/assets/a','connect/delivery/a','.github/workflows/connect-delivery.yml','.github/actions/runtime-budget/action.yml','enablement/verification/workflow-budget.ts','public/site/package.json','public/site/bun.lock','public/site/src/a','public/site/scripts/a'])await put(p);
 await put('connect/delivery/recipient/source.ts');
 await mkdir(join(root,'connect/web/scripts'),{recursive:true});await cp(new URL('../web/scripts/assets.mjs',import.meta.url),join(root,'connect/web/scripts/assets.mjs'));
 for(const p of ['hosted/source-pond-v4.png','hosted/agent-pond-v4.png','hosted/agent-reading-v4.png','fonts/HostGrotesk-Variable.ttf','brand/apparent-wordmark.svg'])await put('public/site/public/assets/'+p);
 await put('worker-artifact/dist/worker.js','export default {}');
 for(const r of ROUTES)await put(`artifact/dist/${r}/index.html`,'<html>production</html>');
 execFileSync('git',['init'],{cwd:root});execFileSync('git',['add','.'],{cwd:root});execFileSync('git',['-c','user.name=Test','-c','user.email=test@example.invalid','commit','-m','fixture'],{cwd:root});
 process.env.PUBLIC_POSTHOG_TOKEN='phc_production_public_key';
 await seal(root,dir);delete process.env.PUBLIC_POSTHOG_TOKEN;await validate(root,dir);
 const worker=join(root,'worker-artifact');await seal(root,worker,'connect-worker');await put('connect/web/package.json','browser-only change');await validate(root,worker,'connect-worker');await put('connect/web/package.json');
 // A historical broader inventory is not reinterpreted as a new baseline.
 for(const [payload,kind] of [[dir,'connect-browser'],[worker,'connect-worker']]) {
 const original=await readFile(join(payload,'manifest.json'),'utf8');const broad=JSON.parse(original);
 broad.inputs['connect/delivery/recipient/source.ts']=createHash('sha256').update('fixture').digest('hex');
 await writeFile(join(payload,'manifest.json'),JSON.stringify(broad));
 await assert.rejects(validate(root,payload,kind),/input mismatch/);
 await writeFile(join(payload,'manifest.json'),original);
 }
 for(const edit of ['change','add','delete']) {
 if(edit==='change')await put('connect/delivery/recipient/source.ts','native change');
 if(edit==='add')await put('connect/delivery/recipient/added.ts','native addition');
 if(edit==='delete')await rm(join(root,'connect/delivery/recipient/source.ts'));
 await validate(root,dir);await validate(root,worker,'connect-worker');
 execFileSync('git',['add','.'],{cwd:root});execFileSync('git',['-c','user.name=Test','-c','user.email=test@example.invalid','commit','-m',edit+' native delivery'],{cwd:root});
 await validate(root,dir);await validate(root,worker,'connect-worker');
 }
 for(const path of ['connect/delivery/recipient-neighbor/source.ts','connect/delivery/provenance.ts','connect/delivery/select.ts','connect/delivery/artifacts.ts','enablement/verification/workflow-budget.ts']) {
 await put(path,'changed admission contract');
 await assert.rejects(validate(root,dir));await assert.rejects(validate(root,worker,'connect-worker'));
 await rm(join(root,path));if(path.startsWith('enablement/'))await put(path);
 }
 await assert.rejects(seal(root,dir));
 await put('.github/workflows/unrelated-public.yml');await put('capture/native.rs');await put('public/site/src/unrelated.ts');await put('connect/service/package.json','changed');await validate(root,dir);
 execFileSync('git',['add','.'],{cwd:root});execFileSync('git',['-c','user.name=Test','-c','user.email=test@example.invalid','commit','-m','unrelated Public change'],{cwd:root});await validate(root,dir);
 for(const p of ['.github/workflows/connect-delivery.yml','shared/ui/new','shared/analytics/new','connect/web/new-config','connect/web/bun.lock','public/site/public/assets/fonts/HostGrotesk-Variable.ttf']){await put(p,'changed');await assert.rejects(validate(root,dir));await rm(join(root,p));if(p.endsWith('.ttf')||p.endsWith('.yml'))await put(p);}
 await rm(join(root,'connect/web/package.json'));await assert.rejects(validate(root,dir));await put('connect/web/package.json');await validate(root,dir);
 await put('shared/ui/added','new tracked input');execFileSync('git',['add','shared/ui/added'],{cwd:root});execFileSync('git',['-c','user.name=Test','-c','user.email=test@example.invalid','commit','-m','shared addition'],{cwd:root});await assert.rejects(validate(root,dir));
 await rm(join(root,'shared/ui/added'));execFileSync('git',['add','-u'],{cwd:root});execFileSync('git',['-c','user.name=Test','-c','user.email=test@example.invalid','commit','-m','restore shared inputs'],{cwd:root});await validate(root,dir);
 await put('artifact/dist/extra','attack');await assert.rejects(validate(root,dir));await rm(join(dir,'dist/extra'));
 await rm(join(dir,'dist/connect/index.html'));await assert.rejects(seal(root,dir));await put('artifact/dist/connect/index.html','http://127.0.0.1');await assert.rejects(seal(root,dir));await rm(join(dir,'dist/connect/index.html'));await symlink('/etc/passwd',join(dir,'dist/connect/index.html'));await assert.rejects(seal(root,dir));
 }finally{await rm(root,{recursive:true,force:true})}
});
