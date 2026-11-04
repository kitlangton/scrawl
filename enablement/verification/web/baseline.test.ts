import assert from 'node:assert/strict'
import test from 'node:test'
import {mkdtempSync,writeFileSync,mkdirSync,rmSync} from 'node:fs'
import {execFileSync} from 'node:child_process'
import {tmpdir} from 'node:os'
import {join} from 'node:path'
import {selection,verifiedJobs} from './baseline.ts'
const sha='a'.repeat(40),candidate='b'.repeat(40),repo='tryapparent/apparent'
const job=(name:string,conclusion='success')=>({name,conclusion,status:'completed'})
const publicJobs=[job('verify'),job('release'),job('Workflow runtime budget')]
const nativeJobs=[job('recipient-release'),job('Workflow runtime budget'),...['x86_64-unknown-linux-gnu','aarch64-unknown-linux-gnu','x86_64-apple-darwin','aarch64-apple-darwin'].map(target=>job(`recipient (runner, ${target})`))]
const run=(id=1,head_sha=sha)=>({id,head_sha,run_attempt:1,path:'.github/workflows/web-delivery.yml',head_branch:'main',repository:{full_name:repo},head_repository:{full_name:repo},event:'push',status:'completed',conclusion:'success'})
function api(runs:any[],jobs:Record<number,any[]>){
 return (async(url:any,options:any)=>{
  assert.equal(options.redirect,'error');assert.equal(options.headers.Authorization,'Bearer synthetic')
  const path=String(url)
  if(path.includes('/runs?'))return Response.json({workflow_runs:path.endsWith('page=1')?runs:[]})
  const id=Number(/runs\/(\d+)\/attempts\/1\/jobs/.exec(path)?.[1]);assert.ok(jobs[id],path)
  return Response.json({total_count:jobs[id].length,jobs:jobs[id]})
 }) as typeof fetch
}
test('selector-only successes never become baselines; same-SHA completed verification avoids duplicate followers',async()=>{
 const calls:string[][]=[]
 const git=(...args:string[])=>{calls.push(args);return args[0]==='diff'?'capture/engine/file.rs\0':''}
 const result=await selection('public',candidate,'synthetic',false,api([run(2),run(1)],{2:[job('verify','skipped'),job('release'),job('Workflow runtime budget')],1:publicJobs}),git)
 assert.equal(result.required,false);assert.equal(result.baselineSha,sha)
 assert.deepEqual(calls.at(-1),['diff','--no-renames','--name-only','-z',sha,candidate])
 assert.equal((await selection('public',sha,'synthetic',false,api([run()],{1:publicJobs}),()=>'' )).required,false)
})
test('missing, failed, nonancestor and uncertain proof selects full checks; explicit refresh bypasses lookup',async()=>{
 const missing=api([],{});assert.equal((await selection('public',candidate,'synthetic',false,missing)).required,true)
 assert.equal((await selection('public',candidate,'synthetic',false,api([{...run(),conclusion:'failure'}],{}))).required,true)
 assert.equal((await selection('public',candidate,'synthetic',false,api([run()],{1:publicJobs}),()=>{throw Error('nonancestor')})).required,true)
 assert.equal((await selection('public',candidate,'synthetic',false,(async()=>new Response('',{status:403})) as typeof fetch)).required,true)
 const noLookup=(async()=>{throw Error('must not query')}) as typeof fetch
 assert.equal((await selection('public',candidate,'synthetic',true,noLookup)).required,true)
 assert.equal((await selection('public',candidate,'',false,noLookup)).required,true)
 assert.equal(verifiedJobs('public',[...publicJobs,job('verify')]),false)
 assert.equal(verifiedJobs('public',publicJobs.map(j=>({...j,status:'in_progress'}))),false)
})
test('native selection needs all platform checks, durable release and successful audit; skips are not proof',()=>{
 assert.equal(verifiedJobs('recipient',nativeJobs),true)
 for(const name of nativeJobs.map(j=>j.name))assert.equal(verifiedJobs('recipient',nativeJobs.map(j=>j.name===name?{...j,conclusion:'skipped'}:j)),false,name)
 assert.equal(verifiedJobs('connect',[job('produce'),job('Workflow runtime budget')]),true)
 assert.equal(verifiedJobs('connect',[job('produce','skipped'),job('Workflow runtime budget')]),false)
})
test('real Git deltas include missed generated sources, shared/workflow changes and deletions',async()=>{
 const root=mkdtempSync(join(tmpdir(),'owner-ci-selection-'))
 const git=(...args:string[])=>execFileSync('git',args,{cwd:root,encoding:'utf8',stdio:['ignore','pipe','pipe']})
 const put=(path:string)=>{mkdirSync(join(root,path,'..'),{recursive:true});writeFileSync(join(root,path),'fixture')}
 const commit=()=>{git('add','.');git('-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','fixture')}
 try {
  put('shared/ui/theme.css');put('README.md');git('init');commit();const base=git('rev-parse','HEAD').trim()
  put('capture/engine/native.rs');commit();let current=git('rev-parse','HEAD').trim()
  const request=api([run(1,base)],{1:publicJobs})
  assert.equal((await selection('public',current,'synthetic',false,request,git)).required,false)
  put('capture/agent/skills/apparent/SKILL.md');commit();current=git('rev-parse','HEAD').trim()
  assert.equal((await selection('public',current,'synthetic',false,request,git)).required,true)
  rmSync(join(root,'shared/ui/theme.css'));commit();current=git('rev-parse','HEAD').trim()
  const result=await selection('public',current,'synthetic',false,request,git)
  assert.equal(result.required,true);assert.ok(result.paths.includes('shared/ui/theme.css'))
 }finally{rmSync(root,{recursive:true,force:true})}
})
