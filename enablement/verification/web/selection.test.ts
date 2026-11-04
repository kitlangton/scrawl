import assert from 'node:assert/strict'
import test from 'node:test'
import { select, gate } from './selection.ts'
test('website and shared reverse consumers select web verification',()=>{
 for(const path of ['public/site/src/components/home/Copy.tsx','public/site/src/features.json','public/site/package.json','connect/web/src/api.ts','connect/delivery/artifacts.ts','shared/ui/theme.css','shared/analytics/index.ts','.github/workflows/web-delivery.yml'])assert(select([path],true).required,path)
 assert(select(['capture/engine/runtime/recording.rs','shared/ui/deleted.css'],true).required)
})
test('unrelated native changes skip web; missing history never narrows',()=>{
 assert(!select(['capture/engine/runtime/recording.rs','Cargo.lock'],true).required)
 assert(select([],false).required)
 assert(select(['capture/engine/runtime/recording.rs'],false).required)
})
test('only intentional skipped checks are not applicable',()=>{
 assert.equal(gate('false','skipped'),'not-applicable');assert.equal(gate('true','success'),'verified')
 for(const result of ['skipped','failure','cancelled',''])assert.throws(()=>gate('true',result))
 for(const result of ['success','failure','cancelled',''])assert.throws(()=>gate('false',result))
 assert.throws(()=>gate('','skipped'))
})

test('manual refresh reruns admission even when source inputs are unchanged',()=>{assert(select([],true,true).required)})
