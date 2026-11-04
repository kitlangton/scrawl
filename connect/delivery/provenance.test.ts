import {test} from 'node:test';
import assert from 'node:assert/strict';
import {admitWorker,verifyArchive,validateArchivePaths} from './provenance.ts';
const sha='a'.repeat(40), path='.github/workflows/connect-delivery.yml';
const run={id:1,workflow_id:2,path,repository:{full_name:'tryapparent/apparent'},head_repository:{full_name:'tryapparent/apparent'},head_branch:'main',head_sha:sha,event:'push',status:'completed',conclusion:'success'};
const workflow={id:2,path,state:'active'};
const artifact={id:3,name:`connect-worker-${sha}`,workflow_run:{id:1,head_sha:sha,head_branch:'main'},expired:false,expires_at:'2099-01-01',digest:`sha256:${'b'.repeat(64)}`};
test('admits canonical successful producer only',()=>{assert.equal(admitWorker(run,workflow,artifact,sha).id,3);for(const change of [{conclusion:'failure'},{conclusion:'skipped'},{head_branch:'feature'},{head_repository:{full_name:'foreign/repo'}},{event:'pull_request'},{workflow_id:9}])assert.throws(()=>admitWorker({...run,...change},workflow,artifact,sha));});
test('rejects expired, foreign and malformed artifact metadata',()=>{for(const change of [{expired:true},{expires_at:'invalid'},{expires_at:'2000-01-01'},{digest:''},{workflow_run:{id:9}},{name:'connect-browser-'+sha}])assert.throws(()=>admitWorker(run,workflow,{...artifact,...change},sha));});
test('rejects archive tampering',()=>assert.throws(()=>verifyArchive(new Uint8Array([1]),artifact.digest)));

test('archive paths reject traversal, links targets and duplicates',()=>{validateArchivePaths(['manifest.json','dist/worker.js']);for(const names of [['../manifest.json','dist/worker.js'],['manifest.json','/dist/worker.js'],['manifest.json','manifest.json'],['manifest.json','dist/worker.js','extra']])assert.throws(()=>validateArchivePaths(names));});
