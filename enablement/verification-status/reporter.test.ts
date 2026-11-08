import {test} from 'node:test';
import assert from 'node:assert/strict';
import {evaluate, inventory, pages, repo, scope} from './reporter.ts';
const identity = {path: '.github/workflows/verification.yml', sha: 'a'.repeat(40), branch: 'feature'};
const run = {id: 4, repository: {full_name: repo}, head_repository: {full_name: repo}, path: identity.path, head_sha: identity.sha, head_branch: identity.branch, event: 'workflow_dispatch', run_attempt: 2, status: 'completed', conclusion: 'success'};
const job = {name: 'verify / rust', run_id: 4, status: 'completed', conclusion: 'success'};
test('missing, skipped, failed and cancelled proof cannot pass', () => {
 assert.equal(evaluate(run, [], identity, [job.name]), 'failure');
 for (const conclusion of ['skipped', 'failure', 'cancelled', 'neutral', null]) {
  assert.equal(evaluate({...run, conclusion}, [job], identity, [job.name]), 'failure');
  assert.equal(evaluate(run, [{...job, conclusion}], identity, [job.name]), 'failure');
 }
 assert.equal(evaluate({...run, status: 'in_progress'}, [], identity, [job.name]), 'pending');
 assert.equal(evaluate(run, [job], identity, [job.name]), 'success');
});
test('wrong workflow, fork, sha, branch, event and attempt fail closed', () => {
 for (const patch of [{path:'wrong'}, {head_repository:{full_name:'fork/repo'}}, {head_sha:'old'}, {head_branch:'main'}, {event:'pull_request'}, {run_attempt:0}]) assert.equal(evaluate({...run,...patch},[job],identity,[job.name]),'failure');
});
test('complete pagination or error', () => {
 let calls = 0;
 assert.equal(pages(() => (++calls === 1 ? {jobs:Array(100).fill(job), total_count:101} : {jobs:[job],total_count:101}), '/jobs', 'jobs').length,101);
 assert.equal(calls,2);
 assert.throws(() => pages(() => ({jobs:[],total_count:1}),'/jobs','jobs'));
});
test('current and complete parallel inventory; unknown fails closed', () => {
 const current = ['select','desktop-ui','rust','release-ready'];
 const source = (ids: string[]) => ids.map(id=>`  ${id}:`).join('\n');
 assert.equal(inventory(source(current),'linux').length,3);
 const parallel = [...current,'connect-client','connect-browser'];
 assert.equal(inventory(source(parallel),'linux').length,5);
 assert.throws(()=>inventory(source([...parallel,'unknown']),'linux'));
 assert.throws(()=>inventory(source([...current,'connect-client']),'linux'));
});
test('only bounded docs/public exemptions, unknown and shared remain broad', () => {
 assert.equal(scope(['enablement/docs/continuous-delivery.md']),'docs');
 assert.equal(scope(['public/site/src/main.tsx']),'public');
 for (const paths of [[], ['shared/ui/theme.ts'], ['unknown.md'], ['Cargo.toml']]) assert.equal(scope(paths),'native');
});
test('native assertions cannot disappear or skip in a green Mac run', () => {
 const mac = {...identity,path:'.github/workflows/verification-mac.yml'};
 assert.equal(evaluate({...run,path:mac.path},[job],mac,[job.name]),'failure');
 const steps = ['Native Mac Storage','Native GPUI Copy','Native Electron selected-reference'].map(name=>({name,conclusion:'success'}));
 assert.equal(evaluate({...run,path:mac.path},[{...job,steps}],mac,[job.name]),'success');
 steps[0].conclusion='skipped';
 assert.equal(evaluate({...run,path:mac.path},[{...job,steps}],mac,[job.name]),'failure');
});
test('actual trusted platform workflow inventory parses only jobs', async () => {
 const {readFileSync} = await import('node:fs');
 assert.equal(inventory(readFileSync('.github/workflows/verification-platform.yml','utf8'),'ubuntu-24.04').length,3);
});
test('reconciliation preserves docs exemption and discards moved candidates', async () => {
 const {reconcile} = await import('./reporter.ts');
 const pr = {number:1,state:'open',html_url:'https://github.com/tryapparent/apparent/pull/1',base:{ref:'main',sha:'b',repo:{full_name:repo}},head:{sha:'a',ref:'feature',repo:{full_name:repo}}};
 for (const moved of [false,true]) {
  let reads=0; const writes:any[]=[];
  reconcile((path,body) => {
   if(body) {writes.push(body);return {};}
   if(path.endsWith('/pulls/1')) return ++reads > 1 && moved ? {...pr,head:{...pr.head,sha:'new'}} : pr;
   if(path.includes('/compare/')) return {merge_base_commit:{sha:'b'},files:[{filename:'enablement/docs/continuous-delivery.md'}]};
   throw Error(path);
  },1);
  assert.equal(writes.length,moved?0:1);
  if(!moved) assert.equal(writes[0].state,'success');
 }
});
test('latest attempt, newer run, relevant base invalidation and docs base reuse', async () => {
 const {reconcile} = await import('./reporter.ts');
 const {readFileSync} = await import('node:fs');
 for (const mode of ['success','rerun','new-run','base-docs','base-code','failed','missing']) {
  const pr = {number:1,state:'open',html_url:'https://github.com/tryapparent/apparent/pull/1',base:{ref:'main',sha:'b',repo:{full_name:repo}},head:{sha:'a',ref:'feature',repo:{full_name:repo}}};
  const writes:any[]=[]; let runReads=0; let listReads=0;
  const result = (id:number) => ({...run,id,head_sha:'a',head_branch:'feature',path: id===1?'.github/workflows/verification.yml':'.github/workflows/verification-mac.yml',conclusion:mode==='failed'?'failure':'success',run_attempt:mode==='rerun' && runReads>2?3:2});
  reconcile((path,body) => {
   if(body){writes.push(body);return {};}
   if(path.endsWith('/pulls/1')) return pr;
   if(path.includes('/compare/')) return path.includes('/compare/old') ? {files:[{filename:mode==='base-code'?'Cargo.toml':'enablement/docs/test.md'}]} : {merge_base_commit:{sha:mode.startsWith('base-')?'old':'b'},files:[{filename:'capture/engine/main.rs'}]};
   if(path.includes('/contents/')) {const filename=path.split('/contents/')[1].split('?')[0];return {content:Buffer.from(readFileSync(filename)).toString('base64')};}
   if(path.includes('/workflows/')) {listReads++;const id=path.includes('verification-mac')?2:1;return {total_count:mode==='missing'?0:1,workflow_runs:mode==='missing'?[]:[{id:mode==='new-run' && listReads>2?3:id}]};}
   const id=Number(path.match(/runs\/(\d+)/)?.[1]);
   if(path.includes('/jobs?')) {
    const runner=id===1?'ubuntu-24.04':'macos-15-intel';
    const steps=['Execute the same changed-tree checks','Actual Electron to Rust Settings integration','Native Mac Storage','Native GPUI Copy','Native Electron selected-reference'].map(name=>({name,conclusion:'success'}));
    return {total_count:3,jobs:['verify / select',`verify / desktop-ui (${runner})`,`verify / rust (${runner})`].map(name=>({...job,name,run_id:id,steps}))};
   }
   if(path.includes('/actions/runs/')) {runReads++;return result(id);}
   throw Error(path);
  },1);
  assert.equal(writes[0].state,mode==='failed'?'failure':['success','base-docs'].includes(mode)?'success':'pending',mode);
 }
});
