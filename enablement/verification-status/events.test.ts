import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {YAML} from 'bun';
import {eligibleEvent, reconcileAll, repo} from './reporter.ts';
import {ReporterHalt} from './api.ts';
const candidate=(number:number)=>({number,state:'open',html_url:`https://github.com/${repo}/pull/${number}`,base:{ref:'main',sha:'base',repo:{full_name:repo}},head:{sha:`head${number}`,ref:`feature-${number}`,repo:{full_name:repo}}});
const manual={id:4,repository:{full_name:repo},event:'workflow_dispatch',path:'.github/workflows/verification-mac.yml',head_repository:{full_name:repo},head_sha:'head1',head_branch:'feature-1',run_attempt:1,status:'queued',pull_requests:[]};
function docsAPI(paths:string[], writes:any[]) {
 return (path:string,body?:any):any=>{
  paths.push(path);
  if(body){writes.push(body);return {};}
  if(path.endsWith('/git/ref/heads/main')) return {object:{sha:'base'}};
  if(path.endsWith('/pulls/1')) return candidate(1);
  if(path.includes('/actions/workflows/')) return {total_count:1,workflow_runs:[{id:manual.id}]};
  if(path.endsWith('/actions/runs/4')) return {...manual,status:'completed'};
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
  if(path.includes('/actions/workflows/')) return {total_count:1,workflow_runs:[{id:manual.id}]};
  if(path.endsWith('/actions/runs/4')) return manual;
  assert.ok(path.includes('&head=tryapparent:feature-1'));
  return [candidate(1),candidate(2),{...candidate(1),head:{...candidate(1).head,sha:'new-head'}}];
 },{workflow_run:manual,action:'requested'},'workflow_run');
 assert.equal(paths.length,4);assert.equal(writes.length,1);assert.equal(writes[0].state,'pending');
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
// Evaluate the actual YAML expressions for representative event contexts. These
// groups use only JavaScript-compatible GitHub operators and format(); checking
// substrings alone missed collisions between targeted/global and ignored events.
const workflow=YAML.parse(readFileSync('.github/workflows/checks.yml','utf8'));
function context(name:string,event:any={},runId=1) {
 return {event_name:name,run_id:runId,ref:'refs/heads/main',repository:repo,event:{pull_request:{number:0,head:{ref:'',repo:{full_name:''}},base:{ref:''}},workflow_run:{head_branch:'',head_repository:{full_name:''}},...event}};
}
function expression(value:string,github:any) {
 const format=(template:string,...values:any[])=>template.replace(/\{(\d+)\}/g,(_,index)=>String(values[Number(index)]));
 return Function('github','inputs','format',`return (${value})`)(github,github.event.inputs??{pull_request:''},format);
}
function key(value:string,github:any) {
 return value.replace(/\$\{\{(.*?)\}\}/g,(_,source)=>String(expression(source,github))).toLowerCase();
}
const topKey=(github:any)=>key(workflow.concurrency.group,github);
const jobKey=(github:any)=>key(workflow.jobs['verification-status'].concurrency.group,github);
test('targeted recovery and global reconciliation cannot evict one another at either level',()=>{
 const push=context('push',{},1),global=context('workflow_dispatch',{},2),target=context('workflow_dispatch',{inputs:{pull_request:'1094'}},3),other=context('workflow_dispatch',{inputs:{pull_request:'1096'}},4);
 for(const globalEvent of [push,global]) {
  assert.notEqual(topKey(globalEvent),topKey(target));
  assert.notEqual(jobKey(globalEvent),jobKey(target));
 }
 assert.equal(jobKey(push),jobKey(global));
 assert.notEqual(jobKey(target),jobKey(other));
});
test('ignored events cannot replace eligible work before job-level rejection',()=>{
 const requested=context('workflow_run',{workflow_run:manual,action:'requested'},1);
 const completed=context('workflow_run',{workflow_run:{...manual,status:'completed'},action:'completed'},2);
 const automatic=context('workflow_run',{workflow_run:{...manual,event:'pull_request'},action:'completed'},3);
 const ignored=context('workflow_run',{workflow_run:manual,action:'in_progress'},4);
 for(const eligible of [requested,completed]) for(const rejected of [automatic,ignored]) {
  assert.notEqual(topKey(eligible),topKey(rejected));
  assert.equal(Boolean(expression(workflow.jobs['verification-status'].if,rejected)),false);
 }
 assert.equal(jobKey(requested),jobKey(completed));
 const pr=context('pull_request_target',{pull_request:candidate(1)},5);
 assert.equal(jobKey(pr),jobKey(completed));
 const fork=context('pull_request_target',{pull_request:{...candidate(1),head:{...candidate(1).head,repo:{full_name:'fork/apparent'}}}},6);
 assert.notEqual(jobKey(pr),jobKey(fork));
});
test('non-main PR events do no API work and are excluded by workflow policy',()=>{
 const pr={...candidate(1),base:{...candidate(1).base,ref:'release'}};
 reconcileAll(()=>{throw Error('must not scan');},{pull_request:pr},'pull_request_target');
 assert.equal(eligibleEvent({pull_request:pr},'pull_request_target'),false);
 assert.deepEqual(workflow.on.pull_request_target.branches,['main']);
 assert.equal(Boolean(expression(workflow.jobs['verification-status'].if,context('pull_request_target',{pull_request:pr}))),false);
});
test('completed then delayed-start events restore current evidence instead of leaving stale pending',()=>{
 for(const [latestId,attempt] of [[4,1],[4,2],[5,1]]) {
  let state='pending';const writes:string[]=[];
  const fetch=(path:string,body?:any):any=>{
   if(body){state=body.state;writes.push(state);return {};}
   if(path.includes('/pulls?'))return [candidate(1)];
   if(path.includes('/actions/workflows/'))return {total_count:1,workflow_runs:[{id:latestId}]};
   if(path.endsWith('/actions/runs/'+latestId))return {...manual,id:latestId,run_attempt:attempt,status:'completed',conclusion:'success'};
   return docsAPI([],[])(path,body);
  };
  reconcileAll(fetch,{workflow_run:{...manual,status:'completed'},action:'completed'},'workflow_run');
  assert.equal(state,'success');
  reconcileAll(fetch,{workflow_run:manual,action:'requested'},'workflow_run');
  assert.equal(state,'success',`latest ${latestId} attempt ${attempt}`);
  assert.deepEqual(writes,['pending','success','pending','success']);
 }
});
test('pending-only invalidation refuses stale attempt or mismatched current identity',()=>{
 for(const patch of [{run_attempt:0},{head_sha:'another-head'},{head_repository:{full_name:'fork/repo'}}]) {
  const writes:string[]=[];
  assert.throws(()=>reconcileAll((path,body)=>{
   if(body){writes.push(body.state);return {};}
   if(path.includes('/pulls?'))return [candidate(1)];
   if(path.includes('/actions/workflows/'))return {total_count:1,workflow_runs:[{id:manual.id}]};
   if(path.endsWith('/actions/runs/4'))return {...manual,...patch};
   throw Error(path);
  },{workflow_run:manual,action:'requested'},'workflow_run'),/identity or attempt/);
  assert.deepEqual(writes,['pending']);
 }
});
test('workflow tests every reporter module and keeps targeted recovery input',()=>{
 assert.equal(workflow.on.workflow_dispatch.inputs.pull_request.type,'string');
 assert.ok(workflow.jobs.contracts.steps.some((s:any)=>s.run==='bun test enablement/verification-status/*.test.ts'));
});
