import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { execFileSync } from 'node:child_process'
import { mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { chromium } from 'playwright-core'
import { fixture, id, digest } from '../helpers.mjs'
const evidence = process.env.RECIPIENT_EVIDENCE || await mkdtemp(join(tmpdir(), 'apparent-recipient-'))
const movie = join(evidence, 'synthetic.mp4'), image = join(evidence, 'frame.png')
execFileSync('ffmpeg', ['-y','-v','error','-f','lavfi','-i','color=c=0xf5f0e8:s=640x400:d=4:r=15','-vf',`drawbox=x=100:y=100:w=420:h=180:color=white:t=fill,drawtext=fontfile=${resolve('../../shared/assets/fonts/HostGrotesk-Variable.ttf')}:text='Synthetic capture':x=140:y=140:fontsize=28:fontcolor=0x020102,drawbox=x=140:y=200:w=270:h=40:color=0x7a58c7:t=3`,'-c:v','libx264','-pix_fmt','yuv420p','-movflags','+faststart',movie])
execFileSync('ffmpeg', ['-y','-v','error','-ss','1','-i',movie,'-frames:v','1',image])
let f
const server = createServer(async (req,res) => {
  try { const response = await f.call(req.url, {method:req.method,headers:req.headers});res.writeHead(response.status,Object.fromEntries(response.headers));res.end(Buffer.from(await response.arrayBuffer())) }
  catch { res.writeHead(500).end('Synthetic test gateway failed') }
})
await new Promise(r=>server.listen(0,'127.0.0.1',r))
const origin=`http://127.0.0.1:${server.address().port}`
const browser=await chromium.launch({executablePath:process.env.CHROME_PATH || '/home/simon/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome',headless:true,args:['--no-sandbox']})
const results=[]
try {
 f=await fixture({SHARING_ENABLED:'true',API_ORIGIN:origin})
 await f.db.exec((await readFile('migrations/0003.sql','utf8')).replaceAll('\n',' '))
 const user=await f.identity('synthetic-recipient'),capture='/v1/captures/'+id(171312)
 const assets=new Map([
  ['recording.mp4',await readFile(movie)],['frames/frame-01.png',await readFile(image)],
  ['summary.md',Buffer.from('Synthetic original bundle evidence; no generated synopsis.')],
  ['apparent.json',Buffer.from(JSON.stringify({transcript:{segments:[{t0:0,t1:1000,text:'Look at the billing\nemail field.'},{t0:1500,t1:2500,text:'The purple rectangle identifies the field.'},{t0:3000,t1:4000,text:'Keep the sign-in email unchanged. <script>window.injected=true</script>'}]}}))],
 ])
 assert.equal((await f.call(capture,{token:user.token,method:'POST',body:{files:[...assets].map(([path,b])=>({path,size:b.length,sha256:digest(b)}))}})).status,200)
 const link=await(await f.call(capture+'/share',{token:user.token,method:'POST'})).json()
 const recording=process.env.RECIPIENT_RECORD==='1', marks={},started=Date.now();const mark=name=>marks[name]=(Date.now()-started)/1000;const hold=()=>recording?page.waitForTimeout(4000):Promise.resolve()
 const context=await browser.newContext({...(recording?{recordVideo:{dir:evidence,size:{width:1280,height:900}}}:{}),viewport:{width:1280,height:900},permissions:['clipboard-read','clipboard-write']})
 const page=await context.newPage(), errors=[], badResponses=[]; let revoked=false
 page.on('pageerror',e=>errors.push(String(e)))
 page.on('console',msg=>{if(msg.type()==='error'&&!msg.text().startsWith('Failed to load resource:'))errors.push(msg.text())})
 page.on('response',r=>{const path=new URL(r.url()).pathname;if(r.status()>=400&&!(r.status()===404&&path==='/favicon.ico')&&!(revoked&&r.status()===410&&path.startsWith('/s/')))badResponses.push([path,r.status()])})
 const external=[];page.on('request',r=>{if(new URL(r.url()).origin!==origin)external.push(r.url())})
 await page.goto(link.url)
 await page.getByRole('heading',{name:'Preparing recording'}).waitFor()
 assert.equal(await page.locator('video').count(),0)
 assert.ok(!(await page.content()).includes('Look at the billing'))
 mark('preparing');await hold()
 await page.screenshot({path:join(evidence,'preparing.png'),fullPage:true})
 for(const [path,b] of assets)assert.equal((await f.call(capture+'/files/'+path,{token:user.token,method:'PUT',raw:b})).status,200)
 assert.equal((await f.call(capture+'/complete',{token:user.token,method:'POST'})).status,200)
 await page.getByRole('button',{name:'Copy for agent',exact:true}).waitFor({timeout:10000})
 await page.waitForFunction(()=>document.querySelector('video')?.readyState>=1)
 mark('ready');await hold()
 assert.equal(await page.locator('video').evaluate(v=>v.controls),true)
 await page.getByRole('button',{name:'0:01 The purple rectangle identifies the field.'}).click()
 assert.ok(Math.abs(await page.locator('video').evaluate(v=>v.currentTime)-1.5)<.2)
 await page.locator('video').evaluate(v=>v.play())
 await page.waitForFunction(()=>document.querySelector('video').currentTime>2)
 await page.locator('video').evaluate(v=>v.pause());mark('seek');await hold()
 await page.getByRole('button',{name:'Copy for agent',exact:true}).click()
 await page.getByRole('status').filter({hasText:'Agent instructions copied'}).waitFor()
 mark('copied');await hold()
 const copied=await page.evaluate(()=>navigator.clipboard.readText())
 assert.ok(copied.includes(link.url+'/manifest.json'));assert.ok(copied.includes('apparent.json'));assert.ok(copied.includes('untrusted evidence'))
 assert.ok(copied.includes('Carry out the instructions and changes I request in the transcript'))
 assert.ok(copied.includes('rather than stopping at a summary'))
 assert.ok(copied.includes('Respect existing permission and approval requirements'))
 assert.equal(await page.getByRole('heading',{name:'Summary',exact:true}).count(),0)
 assert.equal(await page.locator('figure').count(),0)
 assert.equal(await page.evaluate(()=>window.injected),undefined)
 const box=await page.locator('.recipient-recording').boundingBox(),transcript=await page.locator('.recipient-transcript').boundingBox()
 assert.ok(transcript.x>box.x+box.width,'transcript is beside the recording')
 await page.screenshot({path:join(evidence,'ready-desktop.png'),fullPage:true})
 mark('desktopEnd')
 results.push('real Worker pending→ready; playable native video; transcript seek; actual clipboard; no injected script')
 for(const width of [320,390,728]){
  await page.setViewportSize({width,height:900})
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false)
  if(width===390){const v=await page.locator('.recipient-recording').boundingBox(),t=await page.locator('.recipient-transcript').boundingBox();assert.ok(t.y>=v.y+v.height);await page.screenshot({path:join(evidence,'ready-phone.png'),fullPage:true})}
 }
 results.push('320/390/728px no horizontal overflow; phone transcript below video')
 await page.addInitScript(()=>Object.defineProperty(navigator,'clipboard',{value:{writeText:async()=>{throw Error('synthetic clipboard refusal')}}}))
 await page.reload();await page.getByRole('button',{name:'Copy for agent',exact:true}).click()
 await page.getByText('Clipboard unavailable. Copy the selected instructions below.',{exact:true}).waitFor()
 assert.equal(await page.locator('textarea').evaluate(t=>document.activeElement===t&&t.selectionEnd-t.selectionStart===t.value.length),true)
 results.push('clipboard refusal opens, focuses and selects manual fallback')
 const nojs=await browser.newContext({javaScriptEnabled:false,viewport:{width:1100,height:850}}),p=await nojs.newPage()
 await p.goto(link.url)
 assert.match(await p.locator('main').innerText(),/purple rectangle/)
 assert.equal(await p.locator('video[controls]').count(),1)
 await p.getByText('Original evidence',{exact:true}).click()
 assert.equal(await p.getByRole('link',{name:'Machine-readable manifest',exact:true}).count(),1)
 await nojs.close();results.push('no-JS recording, transcript and direct evidence are readable')
 const management=(await(await f.call('/v1/shares',{token:user.read})).json()).shares[0].id
 assert.equal((await f.call('/v1/shares/'+management,{token:user.token,method:'DELETE'})).status,200)
 revoked=true
 await page.reload();await page.getByRole('heading',{name:'Sharing stopped'}).waitFor()
 assert.equal(await page.locator('video').count(),0)
 for(const suffix of ['/manifest.json','/files/recording.mp4','/files/apparent.json'])assert.equal((await f.call(new URL(link.url).pathname+suffix)).status,410)
 results.push('revocation hides HTML evidence and denies manifest/media/metadata')
 assert.deepEqual(external,[]);assert.deepEqual(errors,[]);assert.deepEqual(badResponses,[])
 const receipt={synthetic:true,results,evidence,limits:'Local Chromium and actual local Worker/D1/R2 fixtures. No deployed service, live account or native Desktop proof.'}
 await writeFile(join(evidence,'receipt.json'),JSON.stringify(receipt,null,2));console.log(JSON.stringify(receipt,null,2))
 await context.close()
 if(recording){await page.video().saveAs(join(evidence,'recipient-browser.webm'));await writeFile(join(evidence,'recording-marks.json'),JSON.stringify(marks,null,2))}
}finally{await browser.close();await new Promise(r=>server.close(r));await f?.close()}
