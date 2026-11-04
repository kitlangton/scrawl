import assert from 'node:assert/strict'
import test from 'node:test'
import { createHash } from 'node:crypto'
import { trustedRun, trustedArtifact, verifyArchive, archivePaths, producerPath, compatibleBuild } from './artifact.ts'
const sha = 'a'.repeat(40)
const run = {id: 42, status:'completed',conclusion:'success',head_branch:'main',head_repository:{full_name:'tryapparent/apparent'},repository:{full_name:'tryapparent/apparent'},path:producerPath,event:'push',head_sha:sha}
test('only successful canonical main producer ancestors are trusted',()=>{
  assert(trustedRun(run,()=>true))
  assert(!trustedRun(run,()=>false))
  for(const change of [{status:'queued'},{conclusion:'skipped'},{conclusion:'failure'},{head_branch:'feature'},{path:'.github/workflows/other.yml'},{event:'pull_request'},{head_repository:{full_name:'fork/apparent'}},{repository:{full_name:'fork/apparent'}},{head_sha:'main'}]) assert(!trustedRun({...run,...change},()=>true))
})
test('artifact identity and server digest are bound to the producer',()=>{
 const artifact={id:1,name:`connect-browser-${sha}`,expired:false,digest:`sha256:${'b'.repeat(64)}`,workflow_run:{id:42,head_sha:sha}}
 assert.equal(trustedArtifact(artifact,run),'b'.repeat(64))
 for(const change of [{id:0},{expired:true},{digest:''},{name:'latest'},{workflow_run:{id:43,head_sha:sha}},{workflow_run:{id:42,head_sha:'c'.repeat(40)}}])assert.throws(()=>trustedArtifact({...artifact,...change},run))
 const bytes=Buffer.from('archive');verifyArchive(bytes,createHash('sha256').update(bytes).digest('hex'));assert.throws(()=>verifyArchive(bytes,'b'.repeat(64)))
})
test('archive paths cannot escape, collide or add served metadata',()=>{
 assert.deepEqual(archivePaths('manifest.json\ndist/connect/index.html\n'),['manifest.json','dist/connect/index.html'])
 for(const listing of ['../manifest.json','manifest.json\ndist/../../private','manifest.json\n/absolute','manifest.json\ndist/a\ndist/a','dist/a','manifest.json\nprivate','manifest.json\ndist/secret.map'])assert.throws(()=>archivePaths(listing))
})

test('consumer downloads immutable archive, checks identity and validates extracted bytes',async()=>{
 const { zipSync, strToU8 }=await import('fflate')
 const { mkdtempSync, readFileSync, rmSync, existsSync }=await import('node:fs')
 const { tmpdir }=await import('node:os'); const { join, resolve }=await import('node:path')
 const { execFileSync }=await import('node:child_process'); const { consume, decodeArchive }=await import('./artifact.ts')
 const root=resolve(import.meta.dirname,'../../../..'), sourceSha=execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8'}).trim()
 const temporary=mkdtempSync(join(tmpdir(),'public-artifact-consumer-')), out=join(temporary,'artifact')
 const producer={...run,head_sha:sourceSha}
 const bytes=Buffer.from(zipSync({'manifest.json':strToU8(JSON.stringify({sourceSha,build:{node:'22.23.0',bun:'1.3.10',environment:{VITE_HOSTED_API_ORIGIN:'https://hosted.tryapparent.com',PUBLIC_POSTHOG_ENABLED:process.env.PUBLIC_POSTHOG_ENABLED??'true',PUBLIC_POSTHOG_TOKEN:process.env.PUBLIC_POSTHOG_TOKEN??'',PUBLIC_POSTHOG_HOST:'https://eu.i.posthog.com',PUBLIC_POSTHOG_ENVIRONMENT:'production',NODE_ENV:'production'}}})),'dist/connect/index.html':strToU8('approved route')}))
 const artifact={id:1,name:`connect-browser-${sourceSha}`,expired:false,digest:`sha256:${createHash('sha256').update(bytes).digest('hex')}`,workflow_run:{id:42,head_sha:sourceSha}}
 const api=async(path:string)=>path.includes('/runs?')?{workflow_runs:[producer]}:{artifacts:[artifact]}
 try {
  let checked=false
  const receipt=await consume(root,out,api,async id=>{assert.equal(id,1);return bytes},dir=>{assert.equal(readFileSync(join(dir,'dist/connect/index.html'),'utf8'),'approved route');checked=true})
  assert(checked);assert.equal(receipt.sourceSha,sourceSha);assert.equal(receipt.artifactId,1)
  await assert.rejects(consume(root,out,api,async()=>Buffer.from('corrupt'),()=>{}),/digest/)
  await assert.rejects(consume(root,out,api,async()=>bytes,()=>{throw Error('changed product input')}),/No successful/)
  assert(!existsSync(out))
  for(const path of ['../outside','dist/../../outside','private.txt'])assert.throws(()=>decodeArchive(Buffer.from(zipSync({'manifest.json':strToU8('{}'),[path]:strToU8('invalid')}))))
 }finally{rmSync(temporary,{recursive:true,force:true})}
})

test('current approved build configuration must match reused browser bytes',()=>{
 const environment={VITE_HOSTED_API_ORIGIN:'https://hosted.tryapparent.com',PUBLIC_POSTHOG_ENABLED:'true',PUBLIC_POSTHOG_TOKEN:'phc_test',PUBLIC_POSTHOG_HOST:'https://eu.i.posthog.com',PUBLIC_POSTHOG_ENVIRONMENT:'production',NODE_ENV:'production'}
 const build={node:'22.23.0',bun:'1.3.10',environment}
 assert(compatibleBuild(build,{PUBLIC_POSTHOG_TOKEN:'phc_test'}))
 assert(!compatibleBuild(build,{PUBLIC_POSTHOG_TOKEN:'phc_changed'}))
 assert(!compatibleBuild(build,{PUBLIC_POSTHOG_TOKEN:'phc_test',PUBLIC_POSTHOG_ENABLED:'false'}))
 assert(!compatibleBuild({...build,node:'20.0.0'},{PUBLIC_POSTHOG_TOKEN:'phc_test'}))
})
