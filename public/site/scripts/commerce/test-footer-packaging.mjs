import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
import {test} from 'node:test'

test('footer overlay preserves production and is idempotent; conflicts fail closed',()=>{
  assert(process.env.SITE_BASELINE,'SITE_BASELINE required')
  const script=fileURLToPath(new URL('./package-footer-release.mjs',import.meta.url))
  const tmp=fs.mkdtempSync(path.join(os.tmpdir(),'footer-packaging-'))
  function pack(base,name){
    const candidate=path.join(tmp,name),manifest=candidate+'.json'
    const result=spawnSync(process.execPath,[script,base,candidate,manifest],{encoding:'utf8'})
    return {candidate,manifest,result}
  }
  try{
    const baseline=path.join(tmp,'baseline')
    fs.cpSync(fs.realpathSync(process.env.SITE_BASELINE),baseline,{recursive:true})
    const originalHtml=fs.readFileSync(path.join(baseline,'index.html'),'utf8')
    const alias=path.join(tmp,'baseline-alias')
    fs.symlinkSync(baseline,alias,'dir')
    const refused=pack(alias,'refused')
    assert.notEqual(refused.result.status,0)
    assert.match(refused.result.stderr,/Baseline must be a real directory, not a symlink/)
    assert.equal(fs.existsSync(refused.candidate),false)
    assert.equal(fs.readFileSync(path.join(baseline,'index.html'),'utf8'),originalHtml,'symlink preflight must not mutate its target')
    const first=pack(baseline,'first')
    assert.equal(first.result.status,0,first.result.stderr)
    const second=pack(first.candidate,'second')
    assert.equal(second.result.status,0,second.result.stderr)
    const html=fs.readFileSync(path.join(second.candidate,'index.html'),'utf8')
    assert.equal((html.match(/<apparent-footer>/g)||[]).length,1)
    assert.equal((html.match(/<link rel="stylesheet" href="\/assets\/footer-/g)||[]).length,1)
    assert.equal((html.match(/<script type="module" src="\/assets\/footer-/g)||[]).length,1)
    assert(html.indexOf('<script type="module" src="/assets/footer-')<html.indexOf('<script type="module" crossorigin src="/assets/index-'),'new behavior registers before the baseline bundle')
    assert.equal(html,fs.readFileSync(path.join(first.candidate,'index.html'),'utf8'))
    const manifest=JSON.parse(fs.readFileSync(second.manifest))
    assert.deepEqual(manifest.before,manifest.after,'unchanged repeat adds or changes nothing')
    assert.doesNotMatch(html,/Private prototype|footer-eight|\/home\/simon|data:video|contact-sheet/)
    fs.writeFileSync(path.join(first.candidate,'pond/footer-theatre.webp'),'conflicting fixture')
    const conflict=pack(first.candidate,'conflict')
    assert.notEqual(conflict.result.status,0)
    assert.match(conflict.result.stderr,/Immutable conflict/)
  }finally{fs.rmSync(tmp,{recursive:true,force:true})}
})
