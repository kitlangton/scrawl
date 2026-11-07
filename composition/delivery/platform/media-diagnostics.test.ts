import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import test from 'node:test'
import { publicSummary, stagePublicSummary, writePublicSummary } from '../runtime/media-diagnostics.ts'
const root = fileURLToPath(new URL('../../../', import.meta.url))
const sha = 'a'.repeat(40), hash = (s: string) => createHash('sha256').update(s).digest('hex')
const env = { CI: 'true', GITHUB_SHA: sha, GITHUB_WORKFLOW_SHA: 'b'.repeat(40), GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1', GITHUB_JOB: 'verify-pr', OWNER_ID: 'engine-media-native', OWNER_TARGET: 'linux-x64' }
const entry = 'synthetic entry', manifest = JSON.stringify({ sourceSha: sha })
const identity = { generated: true, entrySha256: hash(entry), manifestSha256: hash(manifest), artifactSourceSha: sha, testCompleted: false, clientCloseReturned: true, request: { secret: 'never-export' }, path: '/private/never-export' }
function fixture(t: any) {
  const dir = mkdtempSync(join(tmpdir(), 'media-public-test-')); t.after(() => rmSync(dir, { recursive: true, force: true }))
  const input=join(dir,'media-failure-summary'), output=join(dir,'media-failure-public'), artifact=join(dir,'workspace/engine/delivery/media/dist'); mkdirSync(artifact,{recursive:true})
  writeFileSync(join(artifact,'apparent-engine-frames'),entry); writeFileSync(join(artifact,'manifest.json'),manifest)
  return {dir,input,output,artifact}
}
test('finite signatures retain useful abort observations without any raw text or private identity', () => {
  const cases = [['cannot allocate memory','allocation-failure'],['malloc(): corrupted size','allocator-corruption'],['pthread_create failed','thread-creation-failure'],['assertion x failed','assertion-failure'],['failed to load plugin','plugin-failure'],['error while loading shared libraries','loader-failure']]
  for(const [raw, category] of cases) {
    const summary=publicSummary(Buffer.from(raw+' /private/never-export token=never-export\napparent-media-diagnostic stage=worker-exit exit-code=none signal=6\n'),identity,env)
    assert.deepEqual(summary.categories,[category]); assert.equal(summary.workerSignal,6); assert.equal(summary.workerExitCode,null)
    assert(!JSON.stringify(summary).includes('never-export')); assert(!JSON.stringify(summary).includes(raw))
  }
  assert.deepEqual(publicSummary(Buffer.from('unknown private error'),identity,env).categories,['unclassified'])
  assert.deepEqual(publicSummary(Buffer.concat([Buffer.from('cannot allocate memory'),Buffer.alloc(8192,120)]),identity,env).categories,['unclassified'])
})
test('generated-only source/job bindings reject external fixtures, missing and arbitrary identities',()=>{
  for(const bad of [{APPARENT_MEDIA_FIXTURE:'/private/recording'}, {GITHUB_SHA:'secret'}, {GITHUB_JOB:'unknown'}, {OWNER_ID:'record'}, {GITHUB_RUN_ID:'secret'}]) assert.throws(()=>publicSummary(Buffer.alloc(0),identity,{...env,...bad}))
  for(const bad of [{generated:false},{entrySha256:'secret'},{artifactSourceSha:'b'.repeat(40)}]) assert.throws(()=>publicSummary(Buffer.alloc(0),{...identity,...bad},env))
})
test('actual staging CLI makes only safe JSON retrievable and binds current artifact bytes',t=>{
  const f=fixture(t); writePublicSummary(f.input,Buffer.from('cannot allocate memory token=never-export'),identity,env)
  writeFileSync(join(f.input,'media-stderr.raw'),'DO NOT EXPORT'); const output=join(f.dir,'github-output')
  const action=readFileSync(join(root,'.github/actions/delivery-component/action.yml'),'utf8'); const script=action.split('    - name: Stage allowlisted generated media failure summary')[1]?.match(/      run: (.+)/)?.[1]; assert(script)
  const run=spawnSync('bash',['-e','-c',script],{cwd:root,env:{...process.env,...env,GITHUB_OUTPUT:output,RUNNER_TEMP:f.dir,GITHUB_WORKSPACE:join(f.dir,'workspace')},encoding:'utf8'})
  assert.equal(run.status,0,run.stderr); assert.equal(readFileSync(output,'utf8'),'found=true\n')
  const publicBytes=readFileSync(join(f.output,'summary.json'),'utf8'); assert(!publicBytes.includes('never-export')); assert(!publicBytes.includes('DO NOT EXPORT'))
  assert.equal(JSON.parse(publicBytes).sourceSha,sha); assert.deepEqual(JSON.parse(publicBytes).categories,['allocation-failure'])
  writeFileSync(join(f.artifact,'apparent-engine-frames'),'changed'); assert.throws(()=>stagePublicSummary(f.input,join(f.dir,'other'),f.artifact,env),/changed/)
})
test('stager rejects stale, unknown, oversized and private-field receipts',t=>{
  for(const bad of [{sourceSha:'c'.repeat(40)},{runId:'124'},{raw:'SECRET'},{categories:['SECRET']},{requestLimits:{path:'SECRET'}},{stderrBytes:8193},{workerSignal:1000},{testCompleted:'SECRET'}]) {
    const f=fixture(t); writePublicSummary(f.input,Buffer.alloc(0),identity,env)
    const path=join(f.input,'summary.json'); writeFileSync(path,JSON.stringify({...JSON.parse(readFileSync(path,'utf8')),...bad})); assert.throws(()=>stagePublicSummary(f.input,f.output,f.artifact,env))
  }
  const f=fixture(t); writePublicSummary(f.input,Buffer.alloc(0),identity,env); writeFileSync(join(f.input,'summary.json'),'x'.repeat(8193)); assert.throws(()=>stagePublicSummary(f.input,f.output,f.artifact,env),/unsafe/)
})
test('stager refuses nonprivate/symlink roots/files and never overwrites output',t=>{
  const f=fixture(t); assert.equal(stagePublicSummary(f.input,f.output,f.artifact,env),false)
  writePublicSummary(f.input,Buffer.alloc(0),identity,env); assert(stagePublicSummary(f.input,f.output,f.artifact,env)); assert.throws(()=>stagePublicSummary(f.input,f.output,f.artifact,env),{code:'EEXIST'})
  const link=join(f.dir,'link'); symlinkSync(f.input,link); assert.throws(()=>stagePublicSummary(link,join(f.dir,'other'),f.artifact,env),/unsafe/)
  chmodSync(f.input,0o755); assert.throws(()=>stagePublicSummary(f.input,join(f.dir,'other'),f.artifact,env),/unsafe/); chmodSync(f.input,0o700)
  const s=fixture(t); mkdirSync(s.input,{mode:0o700}); symlinkSync(join(f.input,'summary.json'),join(s.input,'summary.json')); assert.throws(()=>stagePublicSummary(s.input,s.output,s.artifact,env))
})
test('actual fixture failure call site produces safe bound evidence without decoder or arbitrary fixture export', t=>{
  const f=fixture(t); const fake=join(f.artifact,'apparent-engine-frames'), ffmpeg=join(f.dir,'ffmpeg')
  writeFileSync(fake,"#!/bin/sh\nprintf '%s\\n' 'malloc(): corrupted size /private/never-export' 'apparent-media-diagnostic stage=worker-exit exit-code=none signal=6' >&2\nexit 6\n",{mode:0o700}); chmodSync(fake,0o700)
  writeFileSync(ffmpeg,'#!/bin/sh\nfor last do :; done\nprintf synthetic > "$last"\n',{mode:0o700})
  const childEnv={...process.env}; delete childEnv.NODE_TEST_CONTEXT
  const invoke=(more:Record<string,string>)=>spawnSync(process.execPath,['--experimental-strip-types','--test','--test-name-pattern=packaged entry decodes real H.264',join(root,'engine/delivery/media/test/packaged-h264.test.ts')],{env:{...childEnv,...env,APPARENT_ENGINE_FRAMES_BIN:fake,APPARENT_FFMPEG:ffmpeg,APPARENT_SYNTHETIC_DIAGNOSTICS_DIR:f.input,...more},encoding:'utf8',timeout:15000})
  const run=invoke({APPARENT_MEDIA_FIXTURE:''}); assert.notEqual(run.status,0,run.stdout+run.stderr); assert(!run.error, String(run.error)); assert.equal(JSON.parse(readFileSync(join(f.input,'summary.json'),'utf8')).workerSignal,6)
  assert(stagePublicSummary(f.input,f.output,f.artifact,env)); assert(!readFileSync(join(f.output,'summary.json'),'utf8').includes('never-export'))
  // Remove only clearly identified retained synthetic roots made by this test.
  const roots=[...run.stdout.matchAll(/packaged-h264 synthetic runtime retained: (.+)/g)].map(m=>m[1]); for(const p of roots) if(p.startsWith(resolve(tmpdir())+'/apparent-media-h264-')) rmSync(p,{recursive:true,force:true})
  const external=join(f.dir,'external'); writeFileSync(external,'synthetic external negative'); const other=join(f.dir,'external-public')
  const denied=invoke({APPARENT_MEDIA_FIXTURE:external,APPARENT_SYNTHETIC_DIAGNOSTICS_DIR:other}); assert.notEqual(denied.status,0); assert.equal(stagePublicSummary(other,join(f.dir,'denied'),f.artifact,env),false)
  for(const m of denied.stdout.matchAll(/packaged-h264 synthetic runtime retained: (.+)/g)) if(m[1].startsWith(resolve(tmpdir())+'/apparent-media-h264-')) rmSync(m[1],{recursive:true,force:true})
})
test('existing action exports only safe file on failed actual media build, retaining failure status',()=>{
  const action=readFileSync(join(root,'.github/actions/delivery-component/action.yml'),'utf8')
  assert.match(action,/id: owner-build\n/); assert.match(action,/APPARENT_SYNTHETIC_DIAGNOSTICS_DIR:.*media-failure-summary/)
  const stage=action.split('    - name: Stage allowlisted generated media failure summary\n')[1]?.split('    - name: Verify staged capture')[0]; assert(stage)
  assert.match(stage,/if: failure\(\) && steps.owner-build.outcome == 'failure' && inputs.id == 'engine-media-native' && inputs.target == 'linux-x64'/)
  assert.match(stage,/if: failure\(\) && steps.media-failure-summary.outputs.found == 'true'/); assert.match(stage,/path:.*media-failure-public\/summary.json/); assert.match(stage,/retention-days: 3/)
  assert(!stage.includes('continue-on-error')); assert(!stage.includes('media-stderr.raw')); assert(!stage.includes('media-failure-identity')); assert(!stage.includes('**'))
  const fixture=readFileSync(join(root,'engine/delivery/media/test/packaged-h264.test.ts'),'utf8'); assert.match(fixture,/!process.env.APPARENT_MEDIA_FIXTURE/); assert.match(fixture,/writePublicSummary\(process.env.APPARENT_SYNTHETIC_DIAGNOSTICS_DIR, diagnosticStderr, diagnosticIdentity, process.env\)/)
})
