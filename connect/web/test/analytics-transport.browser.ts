// Built Public + Connect clients, real Chromium, intercepted synthetic-only transport.
import assert from 'node:assert/strict'
import {createServer} from 'node:http'
import {readFile,mkdir} from 'node:fs/promises'
import {resolve,extname} from 'node:path'
import {chromium,type BrowserContext,type Page} from 'playwright-core'
const publicRoot=resolve('../../public/site/dist-analytics-test'),connectRoot=resolve('dist-analytics-test')
const evidence=resolve(process.env.CONNECT_EVIDENCE||`${process.env.TMPDIR}/analytics-transport-evidence`)
await mkdir(evidence,{recursive:true})
const key='apparent.product-analytics.v1',accountId='account:'+'a'.repeat(64)
let accountAllowed=false,revision:string|null=null,signedOut=false,origin=''
let holdNextSession=false,enteredSession:()=>void=()=>{},releaseSession:()=>void=()=>{}
const holdSession=()=>new Promise<void>((resolve,reject)=>{holdNextSession=true;const timer=setTimeout(()=>reject(new Error('Expected held account session was not requested')),5000);enteredSession=()=>{clearTimeout(timer);resolve()}})
const server=createServer(async(req,res)=>{
 const url=new URL(req.url!,'http://synthetic')
 const json=(body:unknown,status=200)=>{res.writeHead(status,{'Content-Type':'application/json','Cache-Control':'no-store'});res.end(JSON.stringify(body))}
 if(url.pathname.startsWith('/v1/')||url.pathname==='/auth/logout'){
  if(req.method==='POST'){
   assert.equal(req.headers['x-csrf-token'],'synthetic-csrf');assert.match(req.headers.cookie||'',/fixture=1/);assert.equal(req.headers.origin,origin)
   let raw='';for await(const part of req)raw+=part;const body=JSON.parse(raw)
   if(url.pathname==='/auth/logout'){signedOut=true;return json({ok:true})}
   assert.equal(url.pathname,'/v1/analytics/consent');assert.equal(body.noticeVersion,'product-analytics-v1')
   if(body.consented&&body.revision!==revision)return json({error:'stale'},409)
   accountAllowed=body.consented;revision=accountAllowed?'1'.repeat(64):'2'.repeat(64)
  }
  if(url.pathname==='/v1/session'){
   const snapshot=signedOut?{authenticated:false}:{authenticated:true,csrfToken:'synthetic-csrf',account:{email:'never-export@example.invalid',beta:'active',analyticsId:accountAllowed?accountId:null,waitlist:{interestedAtFiveDollars:false}},slots:{limit:20,used:1}}
   if(holdNextSession&&accountAllowed&&!signedOut){holdNextSession=false;enteredSession();await new Promise<void>(r=>{releaseSession=r})}
   return json(snapshot)
  }
  if(url.pathname==='/v1/analytics/consent')return json({available:true,consented:accountAllowed,revision,noticeVersion:'product-analytics-v1'})
  if(url.pathname==='/v1/device')return json({scope:'upload',status:'pending',expiresAt:new Date(Date.now()+600000).toISOString()})
  return json({},404)
 }
 let path=url.pathname
 if(/^\/(?:connect|hosted)(?:\/(?:device|shares))?\/?$/.test(path))path=path.replace(/\/$/,'')+'/index.html'
 else if(path==='/'||!extname(path))path='/index.html'
 for(const root of path.startsWith('/connect')||path.startsWith('/hosted')?[connectRoot,publicRoot]:[publicRoot,connectRoot]){
  const file=resolve(root,'.'+path);if(!file.startsWith(root+'/'))continue
  try{const bytes=await readFile(file);res.setHeader('Content-Type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.svg':'image/svg+xml','.png':'image/png','.webp':'image/webp','.ttf':'font/ttf','.mp4':'video/mp4'} as Record<string,string>)[extname(file)]||'application/octet-stream');res.end(bytes);return}catch{}
 }
 res.writeHead(404);res.end()
})
await new Promise<void>(r=>server.listen(0,'127.0.0.1',r));origin=`http://127.0.0.1:${(server.address() as import('node:net').AddressInfo).port}`
const browser=await chromium.launch({executablePath:process.env.CHROME_PATH||'/home/simon/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome',headless:true,args:['--no-sandbox','--mute-audio']})
const passed:string[]=[]
type Event={event:string;uuid:string;timestamp:string;properties:Record<string,unknown>}
async function scenario(name:string,body:(page:Page,sent:Event[],context:BrowserContext)=>Promise<void>,init?:()=>void){
 accountAllowed=false;revision=null;signedOut=false;holdNextSession=false;releaseSession=()=>{}
 const context=await browser.newContext({viewport:{width:1100,height:1000}}),sent:Event[]=[]
 await context.addCookies([{name:'fixture',value:'1',url:origin}])
 if(init)await context.addInitScript(init)
 await context.route('https://eu.i.posthog.com/**',async route=>{
  assert.equal(route.request().url(),'https://eu.i.posthog.com/capture/')
  const headers=await route.request().allHeaders();assert.equal(headers.cookie,undefined);assert.equal(headers.referer,undefined)
  const event=route.request().postDataJSON();assert.equal(event.api_key,'phc_synthetic');assert.doesNotMatch(JSON.stringify(event),/never-export|example\.invalid|NEVER-SEND|csrf|filename|capture-secret/)
  assert.equal(event.properties.environment,'synthetic');assert.equal(event.properties.$geoip_disable,true);assert.equal(event.properties.$ip,null)
  if(event.event==='$identify')assert.match(event.properties.$anon_distinct_id,/^anon:[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/)
  sent.push(event);await route.fulfill({status:200,headers:{'Access-Control-Allow-Origin':'*','Content-Type':'application/json'},body:'{"status":1}'})
 })
 await context.route('**/*',route=>{const url=new URL(route.request().url());return url.origin===origin?route.continue():url.origin==='https://eu.i.posthog.com'?route.fallback():route.abort()})
 const page=await context.newPage()
 try{await body(page,sent,context);passed.push(name)}finally{releaseSession();await context.close()}
}
const settled=async(page:Page)=>{await page.evaluate(()=>new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r()))));await page.waitForTimeout(100)}
const download=async(page:Page)=>{await page.evaluate(()=>{const a=document.createElement('a');a.href='https://github.com/tryapparent/apparent/releases/download/synthetic/Apparent.dmg?token=NEVER-SEND#capture-secret';a.textContent='Synthetic download';a.onclick=e=>e.preventDefault();document.body.append(a)});await page.getByRole('link',{name:'Synthetic download',exact:true}).last().click();await settled(page)}
const allow=async(page:Page)=>{await page.getByRole('button',{name:'Allow analytics',exact:true}).click();await settled(page)}
try{
 await scenario('default-available-but-no-preconsent-send-and-denial-persists',async(page,sent)=>{
  await page.goto(origin+'/?utm_source=NEVER-SEND#capture-secret');await page.getByRole('button',{name:'No thanks',exact:true}).waitFor();await settled(page);assert.equal(sent.length,0)
  await download(page);assert.equal(sent.length,0);await page.getByRole('button',{name:'No thanks',exact:true}).click();await page.reload();await settled(page)
  assert.equal(sent.length,0);assert.equal(await page.getByRole('button',{name:'Allow analytics',exact:true}).count(),0)
  await page.getByRole('button',{name:'Analytics settings',exact:true}).click();await allow(page)
  assert.equal(sent.filter(e=>e.event==='$pageview').length,1)
  assert.equal(sent[0].properties.acquisition_source,'other');assert.equal(sent[0].properties.page,'/')
  await download(page);assert.equal(sent.filter(e=>e.event==='app_download_clicked').length,1)
  assert.equal(sent.at(-1)!.properties.platform,'mac')
  await page.getByRole('button',{name:'Analytics settings',exact:true}).click();await page.getByRole('button',{name:'No thanks',exact:true}).click();const count=sent.length;await download(page);assert.equal(sent.length,count)
  const stored=await page.evaluate(k=>JSON.parse(localStorage.getItem(k)!),key);assert.equal(stored.choice,'deny');assert.equal(stored.id,undefined);assert.equal(stored.account,undefined)
 })
 for(const signal of ['gpc','dnt'])await scenario(signal+'-suppresses-even-saved-consent',async(page,sent)=>{
  await page.goto(origin);await settled(page);assert.equal(await page.getByRole('button',{name:'Allow analytics',exact:true}).count(),0);await download(page);assert.equal(sent.length,0)
 },signal==='gpc'?()=>{Object.defineProperty(navigator,'globalPrivacyControl',{get:()=>true});localStorage.setItem('apparent.product-analytics.v1',JSON.stringify({choice:'allow',noticeVersion:'product-analytics-v1',at:Date.now(),id:'anon:'+crypto.randomUUID()}))}:()=>{Object.defineProperty(navigator,'doNotTrack',{get:()=>'1'});localStorage.setItem('apparent.product-analytics.v1',JSON.stringify({choice:'allow',noticeVersion:'product-analytics-v1',at:Date.now(),id:'anon:'+crypto.randomUUID()}))})
 await scenario('blocked-storage-keeps-disclosure-and-refusal-but-never-sends-or-persists',async(page,sent)=>{
  await page.goto(origin);await page.getByRole('button',{name:'No thanks',exact:true}).waitFor({timeout:2000})
  assert.equal(await page.getByRole('button',{name:'Allow analytics',exact:true}).isDisabled(),true)
  await page.getByText('Details, retention and your rights',{exact:true}).click();await page.getByRole('link',{name:'simon@tryapparent.com',exact:true}).waitFor()
  await page.screenshot({path:resolve(evidence,'browser-blocked-storage.png')})
  await page.getByRole('button',{name:'No thanks',exact:true}).click();await download(page);assert.equal(sent.length,0)
  assert.equal(await page.evaluate(()=>(window as Window&{syntheticStorageWrites:number}).syntheticStorageWrites),0)
  await page.getByRole('button',{name:'Analytics settings',exact:true}).click();assert.equal(await page.getByRole('button',{name:'Allow analytics',exact:true}).isDisabled(),true)
 },()=>{
  (window as Window&{syntheticStorageWrites:number}).syntheticStorageWrites=0
  Object.defineProperty(window,'localStorage',{value:{getItem:()=>{throw new DOMException('blocked','SecurityError')},setItem:()=>{(window as Window&{syntheticStorageWrites:number}).syntheticStorageWrites++}}})
 })
 await scenario('write-blocked-storage-cannot-grant-and-keeps-refusal-visible',async(page,sent)=>{
  await page.goto(origin);await page.getByRole('button',{name:'Allow analytics',exact:true}).click()
  await page.getByRole('status').filter({hasText:'Analytics stays off'}).waitFor()
  assert.equal(await page.getByRole('button',{name:'Allow analytics',exact:true}).isDisabled(),true)
  await download(page);assert.equal(sent.length,0);await page.getByRole('button',{name:'No thanks',exact:true}).click()
 },()=>{Object.defineProperty(window,'localStorage',{value:{getItem:()=>null,setItem:()=>{throw new DOMException('full','QuotaExceededError')}}})})
 for(const removalBlocked of [false,true])await scenario('failed-withdrawal-does-not-replay-saved-allow-after-reload:remove-blocked='+removalBlocked,async(page,sent,context)=>{
  await page.goto(origin);await allow(page);const count=sent.length;assert.ok(count>0)
  const blockStorage=(removeBlocked:boolean)=>{
   Storage.prototype.setItem=()=>{throw new DOMException('read-only','SecurityError')}
   if(removeBlocked)Storage.prototype.removeItem=()=>{throw new DOMException('read-only','SecurityError')}
  }
  await page.evaluate(blockStorage,removalBlocked);await context.addInitScript(blockStorage,removalBlocked)
  await page.getByRole('button',{name:'Analytics settings',exact:true}).click();await page.getByRole('button',{name:'No thanks',exact:true}).click()
  const warned=await page.getByRole('status').filter({hasText:'Analytics stays off'}).isVisible()
  await download(page);assert.equal(sent.length,count)
  await page.reload();await settled(page);assert.equal(sent.length,count,'a failed withdrawal must not replay the previous allow on reload')
  await page.getByRole('button',{name:'No thanks',exact:true}).waitFor({timeout:2000});assert.equal(warned,true)
  if(removalBlocked)assert.equal(await page.getByRole('button',{name:'Allow analytics',exact:true}).isDisabled(),true)
  else assert.equal(await page.evaluate(k=>localStorage.getItem(k),key),null)
 })
 await scenario('failed-opt-in-preserves-an-existing-persisted-refusal',async(page,sent)=>{
  await page.goto(origin);await page.getByRole('button',{name:'No thanks',exact:true}).click()
  const refusal=await page.evaluate(k=>localStorage.getItem(k),key)
  await page.evaluate(()=>{Storage.prototype.setItem=()=>{throw new DOMException('read-only','SecurityError')}})
  await page.getByRole('button',{name:'Analytics settings',exact:true}).click();await page.getByRole('button',{name:'Allow analytics',exact:true}).click()
  await page.getByRole('status').filter({hasText:'Analytics stays off'}).waitFor()
  assert.equal(await page.evaluate(k=>localStorage.getItem(k),key),refusal)
  await page.reload();await download(page);assert.equal(sent.length,0)
 })
 await scenario('malformed-preference-requires-a-fresh-choice-not-permanent-storage-lockout',async(page,sent)=>{
  await page.goto(origin);await page.getByRole('button',{name:'Allow analytics',exact:true}).waitFor();assert.equal(sent.length,0)
  await allow(page);assert.equal(sent.filter(e=>e.event==='$pageview').length,1)
 },()=>{localStorage.setItem('apparent.product-analytics.v1','{corrupt')})
 for(const path of ['/r/capture-secret','/connect/device?code=ABCD-EFGH','/connect/shares','/private/capture-secret'])await scenario('no-instrumentation:'+path,async(page,sent)=>{
  await page.goto(origin+path);await settled(page);assert.equal(await page.getByRole('button',{name:'Allow analytics',exact:true}).count(),0);await download(page);assert.equal(sent.length,0)
 },()=>{localStorage.setItem('apparent.product-analytics.v1',JSON.stringify({choice:'allow',noticeVersion:'product-analytics-v1',at:Date.now(),id:'anon:'+crypto.randomUUID()}))})
 await scenario('browser-and-account-consent-independent-then-both-link-and-withdraw',async(page,sent)=>{
  await page.goto(origin+'/connect?ignored=NEVER-SEND#capture-secret');await page.getByRole('button',{name:'Allow analytics',exact:true}).waitFor();await settled(page);assert.equal(sent.length,0)
  await allow(page);assert.equal(sent.filter(e=>e.event==='$identify').length,0);assert.equal(sent.filter(e=>e.event==='$pageview').length,1);assert.match(sent[0].properties.distinct_id as string,/^anon:/)
  await page.getByText('Optional account analytics',{exact:true}).click();const box=page.getByRole('checkbox',{name:'Allow account analytics',exact:true});await box.check()
  await page.getByRole('status').filter({hasText:'Account analytics is on'}).waitFor();await settled(page)
  const identified=sent.filter(e=>e.event==='$identify');assert.equal(identified.length,1);assert.equal(identified[0].properties.distinct_id,accountId)
  await box.uncheck();await page.getByRole('status').filter({hasText:'Account analytics is off'}).waitFor();await settled(page);const count=sent.length
  await download(page);assert.equal(sent.length,count+1);assert.match(sent.at(-1)!.properties.distinct_id as string,/^anon:/);assert.notEqual(sent.at(-1)!.properties.distinct_id,identified[0].properties.$anon_distinct_id)
 })
 await scenario('account-consent-alone-never-sends-browser-events',async(page,sent)=>{
  accountAllowed=true;revision='1'.repeat(64);await page.goto(origin+'/connect');await settled(page);await download(page);assert.equal(sent.length,0)
 })
 await scenario('malformed-stored-identity-is-replaced-not-forwarded',async(page,sent)=>{
  accountAllowed=true;revision='1'.repeat(64);await page.goto(origin+'/connect');await settled(page)
  assert.equal(sent.filter(e=>e.event==='$identify').length,1)
 },()=>{localStorage.setItem('apparent.product-analytics.v1',JSON.stringify({choice:'allow',noticeVersion:'product-analytics-v1',at:Date.now(),id:'never-export@example.invalid'}))})
 await scenario('out-of-band-account-withdrawal-is-rechecked-before-linked-send',async(page,sent)=>{
  accountAllowed=true;revision='1'.repeat(64);await page.goto(origin+'/connect');await allow(page)
  assert.equal(sent.filter(e=>e.event==='$identify').length,1);const count=sent.length
  accountAllowed=false;revision='2'.repeat(64);await download(page);assert.equal(sent.length,count)
  await download(page);assert.equal(sent.length,count+1);assert.match(sent.at(-1)!.properties.distinct_id as string,/^anon:/)
 })
 for(const action of ['withdraw','logout'])await scenario('late-identity-cannot-restore-link-after-'+action,async(page,sent)=>{
  await page.goto(origin+'/connect');await allow(page);await page.getByText('Optional account analytics',{exact:true}).click()
  const entered=holdSession();await page.getByRole('checkbox',{name:'Allow account analytics',exact:true}).check();await entered
  if(action==='withdraw'){
   await page.getByRole('button',{name:'Stop account analytics',exact:true}).click();await page.getByRole('status').filter({hasText:'Account analytics is off'}).waitFor()
  }else{
   await page.getByRole('button',{name:'Log out',exact:true}).click()
   await page.getByRole('button',{name:'Log out',exact:true}).waitFor({state:'hidden'})
   assert.equal(await page.getByText('Optional account analytics',{exact:true}).count(),0)
   assert.equal(await page.getByText('never-export@example.invalid',{exact:true}).count(),0)
  }
  releaseSession();await settled(page);assert.equal(sent.filter(e=>e.event==='$identify').length,0)
  await download(page);assert.match(sent.at(-1)!.properties.distinct_id as string,/^anon:/)
 })
 await scenario('cross-tab-withdrawal-stops-an-already-consented-page',async(page,sent,context)=>{
  await page.goto(origin);await allow(page);const second=await context.newPage();await second.goto(origin);await second.getByRole('button',{name:'Analytics settings',exact:true}).click();await second.getByRole('button',{name:'No thanks',exact:true}).click();await settled(page)
  const count=sent.length;await download(page);assert.equal(sent.length,count)
 })
 await scenario('runtime-gpc-and-expiry-are-rechecked-at-send',async(page,sent)=>{
  await page.goto(origin);await allow(page);const count=sent.length
  await page.evaluate(()=>Object.defineProperty(navigator,'globalPrivacyControl',{configurable:true,get:()=>true}));await download(page);assert.equal(sent.length,count)
 })
 await scenario('long-lived-tab-consent-expiry-stops-sends',async(page,sent)=>{
  await page.goto(origin);await allow(page);const count=sent.length
  await page.evaluate(()=>{const now=Date.now();Date.now=()=>now+181*86400000});await download(page);assert.equal(sent.length,count)
 })
 await scenario('disclosure-and-choices-fit-320px-with-keyboard-focus',async(page,sent)=>{
  await page.setViewportSize({width:320,height:900});await page.goto(origin);await page.getByRole('button',{name:'No thanks',exact:true}).waitFor();await page.getByText('Details, retention and your rights',{exact:true}).click()
  assert.ok(await page.getByRole('link',{name:'simon@tryapparent.com',exact:true}).count());assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true)
  await page.screenshot({path:resolve(evidence,'browser-consent-320.png'),fullPage:true});assert.equal(sent.length,0)
  await page.locator('aside[aria-label="Analytics preferences"] section').evaluate(el=>{el.scrollTop=el.scrollHeight})
  await page.screenshot({path:resolve(evidence,'browser-consent-320-bottom.png')})
  await page.getByRole('button',{name:'No thanks',exact:true}).focus();await page.keyboard.press('Enter');await page.getByRole('button',{name:'Analytics settings',exact:true}).waitFor()
  await page.getByRole('button',{name:'Analytics settings',exact:true}).focus();await page.keyboard.press('Enter')
  await page.getByRole('button',{name:'No thanks',exact:true}).focus();await page.keyboard.press('Enter')
  assert.equal(await page.evaluate(()=>document.querySelector('aside[aria-label="Analytics preferences"]')?.shadowRoot?.activeElement?.textContent),'Analytics settings')
 })
 console.log(JSON.stringify({passed,evidence,limits:'Built synthetic Chromium clients and intercepted transport only; no actual PostHog, provider writes, real identity or deployed-acceptance claim.'},null,2))
}finally{await browser.close();await new Promise<void>(r=>server.close(()=>r()))}
