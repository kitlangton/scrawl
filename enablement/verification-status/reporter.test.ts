import {test} from 'node:test';
import assert from 'node:assert/strict';
import {YAML} from 'bun';
import {evaluate, inventory, policy, pages, repo, scope} from './reporter.ts';
const identity = {path: '.github/workflows/verification.yml', sha: 'a'.repeat(40), branch: 'feature'};
const run = {id: 4, repository: {full_name: repo}, head_repository: {full_name: repo}, path: identity.path, head_sha: identity.sha, head_branch: identity.branch, event: 'workflow_dispatch', triggering_actor: {login:'dispatcher'}, run_attempt: 2, status: 'completed', conclusion: 'success'};
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
 const names=inventory(readFileSync('.github/workflows/verification-platform.yml','utf8'),'ubuntu-24.04');
 assert.ok(names.includes('verify / rust (ubuntu-24.04)'));
 assert.ok([3,5].includes(names.length));
});
test('reconciliation preserves docs exemption and discards moved candidates', async () => {
 const {reconcile} = await import('./reporter.ts');
 const pr = {number:1,state:'open',html_url:'https://github.com/tryapparent/apparent/pull/1',base:{ref:'main',sha:'b',repo:{full_name:repo}},head:{sha:'a',ref:'feature',repo:{full_name:repo}}};
 for (const moved of [false,true]) {
  let reads=0; const writes:any[]=[];
  reconcile((path,body) => {
   if(body) {writes.push(body);return {};}
   if(path.endsWith('/git/ref/heads/main')) return {object:{sha:'b'}};
   if(path.endsWith('/pulls/1')) return ++reads > 1 && moved ? {...pr,head:{...pr.head,sha:'new'}} : pr;
   if(path.includes('/compare/')) return {merge_base_commit:{sha:'b'},files:[{filename:'enablement/docs/continuous-delivery.md'}]};
   throw Error(path);
  },1);
  assert.equal(writes.length,moved?1:2);
  if(!moved) assert.equal(writes.at(-1).state,'success');
 }
});
test('latest attempt, newer run, relevant base invalidation and docs base reuse', async () => {
 const {reconcile} = await import('./reporter.ts');
 const {readFileSync} = await import('node:fs');
 for (const mode of ['success','rerun','new-run','base-docs','base-code','failed','missing','policy-approved']) {
  const pr = {number:1,state:'open',html_url:'https://github.com/tryapparent/apparent/pull/1',base:{ref:'main',sha:'b',repo:{full_name:repo}},head:{sha:'a',ref:'feature',repo:{full_name:repo}}};
  const writes:any[]=[]; let runReads=0; let listReads=0;
  const result = (id:number) => ({...run,id,head_sha:'a',head_branch:'feature',path: id===1?'.github/workflows/verification.yml':'.github/workflows/verification-mac.yml',conclusion:mode==='failed'?'failure':'success',run_attempt:mode==='rerun' && runReads>2?3:2});
  reconcile((path,body) => {
   if(body){writes.push(body);return {};}
   if(path.endsWith('/git/ref/heads/main')) return {object:{sha:'b'}};
   if(path.endsWith('/pulls/1')) return pr;
   if(path.includes('/permission')) return {permission:'write'};
   if(path.includes('/compare/')) return path.includes('/compare/old') ? {files:[{filename:mode==='base-code'?'Cargo.toml':'enablement/docs/test.md'}]} : {merge_base_commit:{sha:mode.startsWith('base-')?'old':'b'},files:[{filename:mode.startsWith('policy-') || mode==='review-race'?'.github/workflows/verification-platform.yml':'capture/engine/main.rs'}]};
   if(path.includes('/contents/')) {const filename=path.split('/contents/')[1].split('?')[0];return {content:Buffer.from(readFileSync(filename)).toString('base64')};}
   if(path.includes('/workflows/')) {listReads++;const id=path.includes('verification-mac')?2:1;return {total_count:mode==='missing'?0:1,workflow_runs:mode==='missing'?[]:[{id:mode==='new-run' && listReads>2?3:id}]};}
   const id=Number(path.match(/runs\/(\d+)/)?.[1]);
   if(path.includes('/jobs?')) {
    const runner=id===1?'ubuntu-24.04':'macos-15-intel';
    const admitted=policy(readFileSync('.github/workflows/verification-platform.yml','utf8'),runner);
    const document=YAML.parse(readFileSync('.github/workflows/verification-platform.yml','utf8'));
    return {total_count:admitted.names.length,jobs:admitted.names.map(name=>{
     const key=name==='verify / select'?'select':name.split(' / ')[1].split(' ')[0];
     return {...job,name,run_id:id,steps:document.jobs[key].steps.filter((s:any)=>s.run).map((s:any)=>({name:s.name || `Run ${s.run.split('\n')[0]}`,conclusion:id===1 && s.if?.includes("runner.os == 'macOS'") || id===2 && s.if?.includes("runner.os == 'Linux'")?'skipped':'success'}))};
    })};
   }
   if(path.includes('/actions/runs/')) {runReads++;return result(id);}
   throw Error(path);
  },1);
  assert.equal(writes.at(-1).state,mode==='failed'?'failure':['success','base-docs','policy-approved'].includes(mode)?'success':'pending',mode);
 }
});
test('policy maintenance admits additions without byte equality and requires real command anchors', async () => {
 const {readFileSync}=await import('node:fs');
 const source=readFileSync('.github/workflows/verification-platform.yml','utf8');
 const changed=source.replace('name: Platform verification','name: Reviewed platform maintenance');
 assert.deepEqual(policy(changed,'ubuntu-24.04'),policy(source,'ubuntu-24.04'));
 for(const command of ['enablement/verification.py','_electron_rust_roundtrip']) assert.throws(()=>policy(source.replaceAll(command,'removed'),'ubuntu-24.04'));
 for(const prefix of ['Native Mac Storage','Native GPUI Copy','Native Electron selected-reference']) assert.throws(()=>policy(source.replaceAll(prefix,'removed'),'macos-15-intel'));
});
test('required steps must succeed in owning job and candidate order', () => {
 const name='verify / rust (ubuntu-24.04)';
 const expected={[name]:['Actual Electron to Rust Settings integration','Execute the same changed-tree checks']};
 const steps=['Actual Electron to Rust Settings integration','Execute the same changed-tree checks'].map(name=>({name,conclusion:'success'}));
 const proof={...job,name,steps};
 assert.equal(evaluate(run,[proof],identity,[name],expected),'success');
 assert.equal(evaluate(run,[{...proof,steps:steps.toReversed()}],identity,[name],expected),'failure');
 assert.equal(evaluate(run,[{...proof,steps:[]},{...job,name:'other',steps}],identity,[name],expected),'failure');
 assert.equal(evaluate(run,[{...proof,steps:[steps[0],{...steps[1],conclusion:'skipped'}]}],identity,[name],expected),'failure');
});
test('manual dispatch authority is authenticated without requiring an extra approver', async () => {
 const {dispatchAuthority}=await import('./reporter.ts');
 for(const permission of ['admin','maintain','write']) assert.ok(dispatchAuthority(()=>({permission}),run));
 for(const permission of ['read','triage','none']) assert.throws(()=>dispatchAuthority(()=>({permission}),run));
 assert.throws(()=>dispatchAuthority(()=>({permission:'write'}),{...run,triggering_actor:undefined}));
 assert.throws(()=>dispatchAuthority(()=>({permission:'write'}),{...run,event:'pull_request'}));
});

test('Public command steps must succeed and cannot tolerate failures', async () => {
 const {publicPolicy}=await import('./reporter.ts');
 const source='jobs:\n  verify:\n    steps:\n      - name: Public assertions\n        run: npm run verify\n';
 const expected=publicPolicy(source);
 const publicRun={...run,path:'.github/workflows/stream-public.yml'};
 const publicIdentity={...identity,path:publicRun.path};
 const proof={...job,name:'verify',steps:[{name:'Public assertions',conclusion:'success'}]};
 assert.equal(evaluate(publicRun,[proof],publicIdentity,['verify'],expected),'success');
 for(const conclusion of ['skipped','failure']) assert.equal(evaluate(publicRun,[{...proof,steps:[{name:'Public assertions',conclusion}]}],publicIdentity,['verify'],expected),'failure');
 assert.throws(()=>publicPolicy(source.replace('run: npm run verify','continue-on-error: true\n        run: npm run verify')));
});
test('known old green becomes pending on initial and final read failure; later PRs still reconcile', async () => {
 const {reconcileAll}=await import('./reporter.ts');
 const make=(number:number)=>({number,state:'open',html_url:`https://github.com/${repo}/pull/${number}`,base:{ref:'main',sha:'base',repo:{full_name:repo}},head:{sha:`head${number}`,ref:'feature',repo:{full_name:repo}}});
 for(const failAt of [1,2,3]) {
  const states=new Map([['head1','success'],['head2','success']]); let reads=0;
  assert.throws(()=>reconcileAll((path,body)=>{
   if(body){states.set(path.split('/').at(-1)!,body.state);return {};}
   if(path.endsWith('/git/ref/heads/main')) return {object:{sha:'base'}};
   if(path.includes('/pulls?')) return [make(1),make(2)];
   if(path.endsWith('/pulls/1')) {if(++reads===failAt) throw Error('API unavailable');return make(1);}
   if(path.endsWith('/pulls/2')) return make(2);
   if(path.includes('/compare/')) return {merge_base_commit:{sha:'base'},files:[{filename:'enablement/docs/check.md'}]};
   throw Error(path);
  }));
  assert.equal(states.get('head1'),'pending');
  assert.equal(states.get('head2'),'success');
 }
});
test('PR lifecycle hint clears old green even when PR enumeration fails', async () => {
 const {reconcileAll}=await import('./reporter.ts');let state='success';
 assert.throws(()=>reconcileAll((path,body)=>{if(body){state=body.state;return {};}throw Error('listing unavailable');}, {pull_request:{number:1,head:{sha:'head'},base:{ref:'main',repo:{full_name:repo}},html_url:'https://github.com'}}));
 assert.equal(state,'pending');
});

test('manual lifecycle hint clears known head when listing fails', async () => {
 const {reconcileAll}=await import('./reporter.ts');let state='success';
 assert.throws(()=>reconcileAll((path,body)=>{if(body){state=body.state;return {};}throw Error('listing unavailable');}, {workflow_run:{id:1,event:'workflow_dispatch',path:'.github/workflows/verification.yml',head_sha:'head',head_repository:{full_name:repo},pull_requests:[{head:{sha:'head'},base:{ref:'main'}}]}}));
 assert.equal(state,'pending');
});

test('base rename cannot hide removed native source behind a documentation destination', async () => {
 const {reconcile}=await import('./reporter.ts');
 const candidate={number:1,state:'open',html_url:'https://github.com/'+repo+'/pull/1',base:{ref:'main',sha:'base',repo:{full_name:repo}},head:{sha:'head',ref:'feature',repo:{full_name:repo}}};
 const writes:any[]=[];
 reconcile((path,body)=>{
  if(body){writes.push(body);return {};}
  if(path.endsWith('/git/ref/heads/main')) return {object:{sha:'base'}};
  if(path.endsWith('/pulls/1'))return candidate;
  if(path.includes('/compare/base...head'))return {merge_base_commit:{sha:'old'},files:[{filename:'enablement/docs/check.md'}]};
  if(path.includes('/compare/old...base'))return {files:[{filename:'capture/engine/docs/retired.md',previous_filename:'capture/engine/runtime/retired.rs',status:'renamed'}]};
  throw Error(path);
 },1);
 assert.equal(writes.at(-1).state,'pending');
 assert.equal(writes.at(-1).description,'Integrate changed base inputs');
});


test('stale PR base metadata cannot admit proof against old main', async () => {
 const {reconcile}=await import('./reporter.ts');
 const candidate={number:1,state:'open',html_url:'https://github.com/'+repo+'/pull/1',base:{ref:'main',sha:'old',repo:{full_name:repo}},head:{sha:'head',ref:'feature',repo:{full_name:repo}}};
 for(const mode of ['code','docs','advance','unavailable']) {
  const writes:any[]=[];let reads=0;
  reconcile((path,body)=>{
   if(body){writes.push(body);return {};}
   if(path.endsWith('/pulls/1')) return candidate;
   if(path.endsWith('/git/ref/heads/main')) {
    if(mode==='unavailable') throw Error('main ref unavailable');
    return {object:{sha:++reads>1 && mode==='advance'?'new-main':'live-main'}};
   }
   // The stale PR base still produces a valid-looking docs-only comparison.
   if(path.includes('/compare/old...head')) return {merge_base_commit:{sha:'old'},files:[{filename:'enablement/docs/check.md'}]};
   if(path.includes('/compare/live-main...head')) return {merge_base_commit:{sha:'old'},files:[{filename:'enablement/docs/check.md'}]};
   if(path.includes('/compare/old...live-main')) return {files:[{filename:mode==='code'?'capture/engine/runtime/capture.rs':'enablement/docs/base.md'}]};
   throw Error('Unexpected stale-base query: '+path);
  },1);
  assert.equal(writes.at(-1).state,mode==='docs'?'success':'pending',mode);
  if(mode==='code') assert.equal(writes.at(-1).description,'Integrate changed base inputs');
  if(mode==='advance') assert.equal(writes.at(-1).description,'Main advanced during reconciliation');
 }
});
