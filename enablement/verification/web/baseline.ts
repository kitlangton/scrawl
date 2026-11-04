/** Owner verification selects work; a skip never creates new artifact/commit evidence. */
import {execFileSync} from 'node:child_process'
import {appendFileSync} from 'node:fs'
import {pathToFileURL} from 'node:url'
import {selectInputs, type Owner} from './inputs.ts'
const repository='tryapparent/apparent'
const nativeTargets=['x86_64-unknown-linux-gnu','aarch64-unknown-linux-gnu','x86_64-apple-darwin','aarch64-apple-darwin']
export function verifiedJobs(owner: Owner, jobs: any[]): boolean {
  if(jobs.some(job=>job.status!=='completed'))return false
  const one=(name:string)=>jobs.filter(job=>job.name===name).length===1 && jobs.find(job=>job.name===name)?.conclusion==='success'
  if (!one('Workflow runtime budget')) return false
  if (owner==='public') return one('verify') && one('release')
  if (owner==='connect') return one('produce')
  return one('recipient-release') && nativeTargets.every(target=>{
    const matches=jobs.filter(job=>job.name?.startsWith('recipient (') && job.name.endsWith(`, ${target})`))
    return matches.length===1 && matches[0].conclusion==='success'
  })
}
export async function selection(owner: Owner, sha:string, token:string, force=false,
  request:typeof fetch=fetch,
  git=(...args:string[])=>execFileSync('git',args,{encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim()) {
  const full=(reason:string)=>({required:true,reason,baselineSha:'',paths:[] as string[]})
  if (!/^[a-f0-9]{40}$/.test(sha)) throw Error('Missing exact selection candidate')
  if (force) return full('Explicit owner refresh')
  if (!token) return full('Missing authenticated baseline access')
  const workflow=owner==='public'?'web-delivery.yml':'connect-delivery.yml'
  const api=async(path:string)=>{
    const response=await request(`https://api.github.com/repos/${repository}/${path}`,{
      headers:{Authorization:`Bearer ${token}`,Accept:'application/vnd.github+json'},redirect:'error',signal:AbortSignal.timeout(30_000)})
    if(!response.ok)throw Error(`Baseline API ${response.status}`)
    return response.json()
  }
  try {
    for(let page=1;page<=100;page++) {
      const listing=await api(`actions/workflows/${workflow}/runs?branch=main&status=success&per_page=100&page=${page}`)
      if(!Array.isArray(listing.workflow_runs))throw Error('Invalid baseline listing')
      if(!listing.workflow_runs.length)break
      for(const run of listing.workflow_runs) {
        if(run.head_branch!=='main'||run.repository?.full_name!==repository||run.head_repository?.full_name!==repository||
          run.path!==`.github/workflows/${workflow}`||!['push','workflow_dispatch','workflow_run'].includes(run.event)||
          run.status!=='completed'||run.conclusion!=='success'||!/^[a-f0-9]{40}$/.test(run.head_sha)||
          !Number.isSafeInteger(run.id)||run.id<=0||!Number.isSafeInteger(run.run_attempt)||run.run_attempt<=0)continue
        try {git('merge-base','--is-ancestor',run.head_sha,sha)}catch{continue}
        const jobs:any[]=[]
        for(let jobPage=1;jobPage<=100;jobPage++) {
          const batch=await api(`actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100&page=${jobPage}`)
          if(!Number.isInteger(batch.total_count)||batch.total_count<1||!Array.isArray(batch.jobs)||jobs.length+batch.jobs.length>batch.total_count)throw Error('Incomplete baseline jobs')
          jobs.push(...batch.jobs)
          if(jobs.length===batch.total_count)break
          if(!batch.jobs.length||jobPage===100)throw Error('Incomplete baseline jobs')
        }
        if(!verifiedJobs(owner,jobs))continue
        const paths=git('diff','--no-renames','--name-only','-z',run.head_sha,sha).split('\0').filter(Boolean)
        return {...selectInputs(paths,true,false,owner),baselineSha:run.head_sha,paths}
      }
    }
    return full('No actual successful owner verification baseline')
  }catch(error){return full(`Uncertain baseline; full checks required: ${String(error)}`)}
}
if(process.argv[1]&&import.meta.url===pathToFileURL(process.argv[1]).href){
  const owner=process.argv[2] as Owner
  if(!['public','connect','recipient'].includes(owner))throw Error('Unknown delivery owner')
  const result=await selection(owner,process.env.GITHUB_SHA??'',process.env.GH_TOKEN??'',process.env.GITHUB_EVENT_NAME==='workflow_dispatch')
  if(!process.env.GITHUB_OUTPUT)throw Error('Missing selection output')
  appendFileSync(process.env.GITHUB_OUTPUT,`required=${result.required}\n`)
  console.log(JSON.stringify(result))
}
