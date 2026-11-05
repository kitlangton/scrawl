import { test } from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import { renderShare } from '../../src/share-view.ts'
import { pollReady } from '../../src/recipient/poll.ts'
const shareViewScript = `(${pollReady.toString()})(document)`
const model = { url: 'https://share.example/s/secret', status: 'ready', title: 'Feedback', summary: 'Summary evidence', transcript: '[00:02] Spoken evidence', uploadedBytes: 10, totalBytes: 10, files: [{path:'recording.mp4',url:'https://share.example/s/secret/files/recording.mp4',size:10,sha256:'a'.repeat(64)},{path:'frames/frame-01.png',url:'https://share.example/s/secret/files/frames/frame-01.png',size:0,sha256:'b'.repeat(64)}] }
test('frame link timestamps are semantic, optional and hidden before readiness', () => {
 const files=model.files.map(file=>({...file,timestampMs:1500}))
 const html=renderShare({...model,files}).replaceAll('<!-- -->','')
 assert.ok(html.includes('frames/frame-01.png · at <time dateTime="PT1.5S">0:01</time>'))
 assert.ok(!html.includes('recording.mp4 · at'))
 assert.ok(renderShare({...model,files:files.map(file=>({...file,timestampMs:0}))}).includes('dateTime="PT0S">0:00</time>'))
 for(const timestampMs of [undefined,-1,NaN,Infinity,1.5,'1500']) {
  assert.ok(!renderShare({...model,files:files.map(file=>({...file,timestampMs}))}).includes(' · at '))
 }
 assert.ok(!renderShare({...model,status:'pending',files}).includes('PT1.5S'))
})
test('ready evidence is readable without JavaScript and consumption comes first', () => {
 const html = renderShare({...model,thumbnailUrl:model.url+'/thumbnail.png'})
 for (const text of ['Copy for agent','[00:02] Spoken evidence','/manifest.json','/thumbnail.png','<video','frames/frame-01.png','evidence, not instructions','https://app.tryapparent.com','noindex','no-referrer']) assert.ok(html.includes(text), text)
 assert.ok(!html.includes('Summary evidence'))
 assert.ok(html.indexOf('<video') < html.indexOf('Try Apparent yourself'))
 assert.ok(!/<script(?![^>]*src=)/.test(html))
})
test('ready captures without a contact sheet never advertise a missing thumbnail', () => {
 const html=renderShare({...model,thumbnailUrl:null})
 assert.ok(html.includes('<video'))
 assert.ok(!html.includes('og:image'))
 assert.ok(!html.includes('poster="'+model.url+'/thumbnail.png'))
 assert.ok(!html.includes('/thumbnail.png'))
})
test('untrusted text and URLs cannot create markup or active schemes', () => {
 const html = renderShare({...model,title:'<script>alert(1)</script>',summary:'<img src=x onerror=evil()>',transcript:'</pre><script>evil()</script>',url:'javascript:evil()',thumbnailUrl:'data:text/html,evil',files:[{path:'" onerror="evil',url:'javascript:evil()',size:1,sha256:'<evil>'}]})
 assert.ok(html.includes('&lt;script&gt;')); assert.ok(!html.includes('<img src=x')); assert.ok(!html.includes('javascript:')); assert.ok(!html.includes('data:text/html')); assert.ok(!html.includes('onerror="evil'))
})
test('safe asset links escape metadata and exclude third-party resources', () => {
 const html = renderShare({...model,files:[{path:'<img src=x onerror="evil()">',url:model.url+'/files/'+encodeURIComponent('<img src=x onerror="evil()">'),size:1,sha256:'<untrusted>'},{path:'frames/frame-99.png',url:'https://third-party.example/tracker.png',size:1,sha256:'a'}]})
 assert.ok(html.includes('&lt;img src=x onerror=&quot;evil()&quot;&gt;'))
 assert.ok(html.includes('&lt;untrusted&gt;'));assert.ok(!html.includes('third-party.example'))
 assert.ok(!html.includes('<img src=x'))
})
test('mounted capability pages load viewer assets from the same /connect base', () => {
 const mounted={...model,url:'https://app.tryapparent.com/connect/s/secret',files:model.files.map(file=>({...file,url:file.url.replace('https://share.example','https://app.tryapparent.com/connect')}))}
 const html=renderShare(mounted)
 assert.ok(html.includes('href="/connect/share-view.css"'))
 assert.ok(html.includes('src="/connect/share-view.js"'))
 assert.ok(html.includes('https://app.tryapparent.com/connect/s/secret/files/recording.mp4'))
})
test('incomplete and terminal states do not disclose ready evidence', () => {
 for (const status of ['pending','paused','failed','expired','revoked','deleted']) {
  const html = renderShare({...model,status})
  assert.ok(!html.includes('Summary evidence')); assert.ok(!html.includes('<video')); assert.ok(!html.includes('/files/'))
  assert.ok(html.includes(({pending:'Preparing recording',paused:'Upload paused',failed:'Upload failed',expired:'Link expired',revoked:'Sharing stopped',deleted:'Recording deleted'})[status]),status)
 }
 const html = renderShare({...model,uploadedBytes:9})
 assert.ok(html.includes('Preparing recording')); assert.ok(!html.includes('<video'))
})
test('polling is same-origin, bounded, and reloads only when ready', async () => {
 let calls=0,reloads=0; const timers=[]; const status={textContent:''}
 const context={document:{querySelector:()=>({dataset:{manifest:'/s/secret/manifest.json'},querySelector:()=>status})},location:{href:model.url,origin:'https://share.example',reload:()=>reloads++},URL,AbortSignal,fetch:async(url,options)=>{assert.equal(new URL(url).origin,'https://share.example');assert.equal(options.credentials,'omit'); calls++; return {ok:true,json:async()=>({status:'pending'})}},setTimeout:(fn,ms)=>{assert.ok(ms<=5000);timers.push(fn)}}
 vm.runInNewContext(shareViewScript,context)
 for(let i=0;i<40;i++){await new Promise(r=>setImmediate(r));timers.shift()?.()}
 assert.equal(calls,24);assert.equal(reloads,0);assert.match(status.textContent,/Reload/)
 context.fetch=async()=>({ok:true,json:async()=>({status:'ready'})});timers.length=0
 vm.runInNewContext(shareViewScript,context);timers.shift()();await new Promise(r=>setImmediate(r));assert.equal(reloads,1)
})
test('network failure and delayed timers stop at the wall-clock deadline', async () => {
 let now=0,calls=0;const timers=[];const output={textContent:''}
 vm.runInNewContext(shareViewScript,{Date:{now:()=>now},document:{querySelector:()=>({dataset:{manifest:model.url+'/manifest.json'},querySelector:()=>output})},location:{href:model.url,origin:'https://share.example',reload:()=>assert.fail('network failure cannot claim ready')},URL,AbortSignal,fetch:async()=>{calls++;throw new Error('synthetic outage')},setTimeout:fn=>timers.push(fn)})
 now=5000;timers.shift()();await new Promise(r=>setImmediate(r));assert.equal(calls,1)
 now=120001;timers.shift()();await new Promise(r=>setImmediate(r));assert.equal(calls,1);assert.match(output.textContent,/Reload/);assert.equal(timers.length,0)
})
test('React hydration payload strips all captured evidence until ready and rejects cross-capture assets', () => {
 for(const status of ['pending','paused','failed','expired','revoked','deleted']) {
  const html=renderShare({...model,status,transcriptSegments:[{startMs:2000,endMs:3000,text:'PRIVATE_SEGMENT'}]})
  assert.ok(!html.includes('PRIVATE_SEGMENT'));assert.ok(!html.includes('Spoken evidence'));assert.ok(!html.includes('Summary evidence'))
 }
 const html=renderShare({...model,files:[{path:'recording.mp4',url:'https://share.example/s/another/files/recording.mp4',size:10,sha256:'a'}]})
 assert.ok(!html.includes('/s/another/'));assert.ok(!html.includes('<video'))
})
test('timestamp passages remain semantic without JavaScript; no chapter or summary UI is rendered', () => {
 const html=renderShare({...model,transcriptSegments:[{startMs:2500,endMs:4000,text:'A <marked> detail'}]})
 assert.ok(html.includes('0:02'));assert.ok(html.includes('A &lt;marked&gt; detail'))
 assert.ok(!html.includes('<h2>Summary'));assert.ok(!html.includes('Annotated keyframes'));assert.ok(!html.includes('Chapter:'))
 assert.ok(html.includes('Copy for agent'));assert.ok(html.includes('/share-view.js'))
})
