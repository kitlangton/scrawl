import assert from 'node:assert/strict'
import test from 'node:test'
import {relevant, type Owner} from './inputs.ts'
import {readFileSync} from 'node:fs'
const owners:Owner[]=['public','connect','recipient']
test('build entrypoints scope the color guard to their declared consumers; repository guard remains global',()=>{
 for(const [path,owner] of [['public/site','public'],['connect/web','connect'],['connect/service','connect']]) {
  const manifest=JSON.parse(readFileSync(new URL(`../../../${path}/package.json`,import.meta.url),'utf8'))
  assert.ok(manifest.scripts.build.includes(`check-theme.${path === 'connect/service' ? 'py' : 'ts'} --owner ${owner} &&`),path)
 }
 const verifier=readFileSync(new URL('../../verification.py',import.meta.url),'utf8')
 assert.match(verifier,/\["python3", "shared\/ui\/scripts\/check-theme.py"\]/)
})
test('actual owner, generated Public content and reverse shared consumers select their lanes',()=>{
  for(const path of ['public/site/src/Home.tsx','public/delivery.json','composition/delivery/core.ts','.github/actions/delivery-component/action.yml','README.md','capture/agent/skills/apparent/SKILL.md'])assert(relevant('public',path),path)
  assert(relevant('public','enablement/verification/web/baseline.ts'))
  assert(relevant('public','public/site/scripts/cd/stream-artifact.ts'))
  assert(!relevant('connect','README.md'));assert(!relevant('recipient','public/site/src/Home.tsx'))
  assert(relevant('connect','connect/service/src/worker.ts'));assert(!relevant('public','connect/service/src/worker.ts'))
  for(const owner of ['public','connect'] as Owner[])for(const path of ['connect/web/package.json','shared/ui/token.css','shared/analytics/config.ts','shared/assets/brand/logo.svg','enablement/release_gate.py'])assert(relevant(owner,path),`${owner}: ${path}`)
  assert(relevant('connect','public/site/public/assets/fonts/HostGrotesk-Variable.ttf'))
  assert(!relevant('connect','public/site/src/Home.tsx'))
  for(const path of ['connect/client/Cargo.lock','connect/delivery/recipient/seal.ts','shared/config-native/src/lib.rs','shared/config-native/Cargo.lock']) {
    assert(relevant('recipient',path));assert(!relevant('public',path));assert(!relevant('connect',path))
  }
})
test('workflow, toolchain, provenance and uncertain additions cannot silently narrow checks',()=>{
  for(const owner of owners)for(const path of ['.github/workflows/connect-delivery.yml','enablement/verification/workflow-budget.ts','enablement/verification/web/inputs.ts','enablement/verification/web/baseline.ts','.github/actions/runtime-budget/action.yml','.npmrc','new-build-tool.ts','.github/workflows/new-reusable.yml','shared/new-input.ts','../escape'])assert(relevant(owner,path),`${owner}: ${path}`)
  for(const owner of owners)for(const path of ['capture/engine/runtime/recording.rs','composition/cli.rs','Cargo.lock','AGENTS.md'])assert(!relevant(owner,path),`${owner}: ${path}`)
  assert(relevant('public','.github/workflows/web-delivery.yml'))
  assert(!relevant('recipient','.github/workflows/web-delivery.yml'))
})
