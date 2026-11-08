import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {YAML} from 'bun';
import {eligibleEvent, reconcileAll, repo} from './reporter.ts';
import {ReporterHalt} from './api.ts';
const candidate=(number:number)=>({number,state:'open',html_url:`https://github.com/${repo}/pull/${number}`,base:{ref:'main',sha:'base',repo:{full_name:repo}},head:{sha:`head${number}`,ref:`feature-${number}`,repo:{full_name:repo}}});
const manual={id:4,event:'workflow_dispatch',path:'.github/workflows/verification-mac.yml',head_repository:{full_name:repo},head_sha:'head1',head_branch:'feature-1',run_attempt:1,status:'queued',pull_requests:[]};
function docsAPI(paths:string[], writes:any[]) {
 return (path:string,body?:any):any=>{
  paths.push(path);
  if(body){writes.push(body);return {};}
  if(path.endsWith('/git/ref/heads/main')) return {object:{sha:'base'}};
  if(path.endsWith('/pulls/1')) return candidate(1);
  if(path.includes('/compare/')) return {merge_base_commit:{sha:'base'},files:[{filename:'enablement/docs/probe.md'}]};
  throw Error('Unexpected request '+path);
 };
}
test('PR lifecycle reconciles only its own candidate, without enumerating unrelated PRs',()=>{
 const paths:string[]=[];const writes:any[]=[];
 reconcileAll(docsAPI(paths,writes),{pull_request:candidate(1)},'pull_request_target');
 assert.ok(!paths.some(p=>p.includes('/pulls?')));
 assert.ok(paths.every(p=>!p.includes('/pulls/2')));
 assert.deepEqual(writes.map(w=>w.state),['pending','success']);
 assert.ok(paths.length<=9);
});
test('manual recovery targets one validated PR number',()=>{
 const paths:string[]=[];const writes:any[]=[];
 reconcileAll(docsAPI(paths,writes),{inputs:{pull_request:'1'}},'workflow_dispatch');
 assert.ok(paths.includes(`repos/${repo}/pulls/1`));assert.ok(!paths.some(p=>p.includes('/pulls?')));
 for(const number of ['-1','1/../../other','all','1.5']) assert.throws(()=>reconcileAll(()=>{throw Error('must not request');},{inputs:{pull_request:number}},'workflow_dispatch'),/Invalid targeted/);
});
test('automatic, fork and irrelevant workflow events perform no API work',()=>{
 for(const run of [{...manual,event:'pull_request'},{...manual,event:'push'},{...manual,head_repository:{full_name:'fork/repo'}},{...manual,path:'.github/workflows/other.yml'}]) {
  assert.equal(eligibleEvent({workflow_run:run},'workflow_run'),false);
  reconcileAll(()=>{throw Error('must not request');},{workflow_run:run},'workflow_run');
 }
 assert.equal(eligibleEvent({workflow_run:manual,action:'in_progress'},'workflow_run'),false);
 assert.equal(eligibleEvent({workflow_run:{...manual,run_attempt:2},action:'in_progress'},'workflow_run'),true);
});
test('requested manual runs invalidate only their matching head with no evidence scan',()=>{
 const paths:string[]=[];const writes:any[]=[];
 reconcileAll((path,body)=>{
  paths.push(path);if(body){writes.push({path,...body});return {};}
  assert.ok(path.includes('&head=tryapparent:feature-1'));
  return [candidate(1),candidate(2),{...candidate(1),head:{...candidate(1).head,sha:'new-head'}}];
 },{workflow_run:manual,action:'requested'},'workflow_run');
 assert.equal(paths.length,2);assert.equal(writes.length,1);assert.equal(writes[0].state,'pending');
 assert.ok(writes[0].path.endsWith('/head1'));
});
test('completed workflow scopes reconciliation to matching open PR, not the global set',()=>{
 const paths:string[]=[];const writes:any[]=[];const docs=docsAPI(paths,writes);
 reconcileAll((path,body)=>{
  if(!body && path.includes('/pulls?')) {paths.push(path);assert.ok(path.includes('&head=tryapparent:feature-1'));return [candidate(1),candidate(2)];}
  return docs(path,body);
 },{workflow_run:{...manual,status:'completed'},action:'completed'},'workflow_run');
 assert.deepEqual(writes.map(w=>w.state),['pending','success']);
 assert.ok(!paths.some(p=>p.endsWith('/pulls/2')));
});
test('global events invalidate all known heads before evidence; quota errors stop immediately',()=>{
 const writes:string[]=[];const paths:string[]=[];
 assert.throws(()=>reconcileAll((path,body)=>{
  paths.push(path);if(body){writes.push(path);return {};}
  if(path.includes('/pulls?'))return [candidate(1),candidate(2)];
  if(path.endsWith('/pulls/1')){assert.equal(writes.length,2);return candidate(1);}
  if(path.endsWith('/git/ref/heads/main'))throw new ReporterHalt('quota exhausted');
  throw Error('Unexpected request after quota '+path);
 },{},'push'),ReporterHalt);
 assert.ok(!paths.some(p=>p.endsWith('/pulls/2')));
 assert.equal(paths.at(-1),`repos/${repo}/git/ref/heads/main`);
});
test('workflow keeps global main invalidation separate from per-branch coalescing and tests every reporter module',()=>{
 const workflow=YAML.parse(readFileSync('.github/workflows/checks.yml','utf8'));
 assert.equal(workflow.on.workflow_dispatch.inputs.pull_request.type,'string');
 const condition=workflow.jobs['verification-status'].if;
 assert.ok(condition.includes("github.event.workflow_run.event == 'workflow_dispatch'"));
 assert.ok(condition.includes('github.event.workflow_run.run_attempt > 1'));
 assert.ok(workflow.jobs['verification-status'].concurrency.group.includes('github.event.workflow_run.head_branch'));
 assert.ok(workflow.jobs['verification-status'].concurrency.group.includes('github.event.pull_request.head.ref'));
 assert.ok(workflow.jobs.contracts.steps.some((s:any)=>s.run==='bun test enablement/verification-status/*.test.ts'));
});
