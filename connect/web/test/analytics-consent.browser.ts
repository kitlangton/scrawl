// Real packaged React UI + isolated HTTP API. No provider or real accounts.
import assert from 'node:assert/strict'
import {createServer, type ServerResponse} from 'node:http'
import {readFile, mkdir} from 'node:fs/promises'
import {resolve, extname} from 'node:path'
import {chromium} from 'playwright-core'
const root=resolve(process.env.CONNECT_ARTIFACT || 'dist-test')
const evidence=resolve(process.env.CONNECT_EVIDENCE || `${process.env.TMPDIR}/analytics-consent-evidence`)
await mkdir(evidence,{recursive:true})
let origin='', consented=false, revision:string|null=null, available=true, unavailable=false, malformed=false, unconfirmed=false
let holdPositive=false, releasePositive:()=>void=()=>{}, signedOut=false
const mutations:Array<{body:Record<string,unknown>;csrf:string|undefined;cookie:string|undefined}>=[]
const reply=(res:ServerResponse,data:unknown,status=200)=>{res.writeHead(status,{'Content-Type':'application/json','Cache-Control':'no-store'});res.end(JSON.stringify(data))}
const handler=async(req:import('node:http').IncomingMessage,res:ServerResponse)=>{
 const url=new URL(req.url!,'http://fixture')
 if(url.pathname.startsWith('/v1/') || url.pathname==='/auth/logout'){
  res.setHeader('Access-Control-Allow-Origin',origin);res.setHeader('Access-Control-Allow-Credentials','true')
  res.setHeader('Access-Control-Allow-Headers','Content-Type,X-CSRF-Token');res.setHeader('Access-Control-Allow-Methods','GET,POST,OPTIONS')
  if(req.method==='OPTIONS'){res.writeHead(204);res.end();return}
  if(url.pathname==='/v1/session')return reply(res,signedOut?{authenticated:false}:{authenticated:true,csrfToken:'synthetic-csrf',account:{email:'synthetic@example.invalid',beta:'active',waitlist:{interestedAtFiveDollars:false}},slots:{limit:20,used:1}})
  if(req.method==='POST'){
   const chunks=[];for await(const chunk of req)chunks.push(chunk)
   const body=JSON.parse(Buffer.concat(chunks).toString())
   mutations.push({body,csrf:req.headers['x-csrf-token'] as string|undefined,cookie:req.headers.cookie})
   assert.equal(req.headers.origin,origin);assert.equal(req.headers['x-csrf-token'],'synthetic-csrf');assert.match(req.headers.cookie||'',/fixture=1/)
   if(url.pathname==='/auth/logout'){signedOut=true;return reply(res,{ok:true})}
   assert.equal(url.pathname,'/v1/analytics/consent');assert.deepEqual(Object.keys(body).sort(),['consented','noticeVersion','revision'])
   assert.equal(body.noticeVersion,'product-analytics-v1')
   if(body.consented && holdPositive)await new Promise<void>(r=>{releasePositive=r})
   if(body.consented && body.revision!==revision)return reply(res,{error:'stale'},409)
   if(!unconfirmed){consented=body.consented;revision=mutations.length.toString(16).padStart(64,'0')}
   return reply(res,{available,consented,revision,noticeVersion:'product-analytics-v1'})
  }
  if(url.pathname==='/v1/analytics/consent')return reply(res,malformed?{available:true,consented:'yes'}:{available,consented,revision,noticeVersion:'product-analytics-v1'},unavailable?503:200)
  return reply(res,{},404)
 }
 let path=url.pathname
 if(/^\/(connect|hosted)\/?$/.test(path))path=path.replace(/\/$/,'')+'/index.html'
 const file=resolve(root,'.'+path)
 if(!file.startsWith(root+'/')){res.writeHead(403);res.end();return}
 try{res.setHeader('Content-Type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.svg':'image/svg+xml','.ttf':'font/ttf','.png':'image/png'} as Record<string,string>)[extname(file)]||'application/octet-stream');res.end(await readFile(file))}catch{res.writeHead(404);res.end()}
}
const server=createServer((req,res)=>{void handler(req,res)})
await new Promise<void>(r=>server.listen(0,'127.0.0.1',r));origin=`http://127.0.0.1:${(server.address() as import('node:net').AddressInfo).port}`
// Existing dist-test may use either a production-shaped origin or a loopback build origin.
const browser=await chromium.launch({executablePath:process.env.CHROME_PATH||'/home/simon/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome',headless:true,args:['--no-sandbox']})
const context=await browser.newContext({viewport:{width:1100,height:1000}})
const page=await context.newPage()
const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message))
const apiOrigin=process.env.HOSTED_TEST_API_ORIGIN||'https://hosted.tryapparent.com'
const apiServer=createServer((req,res)=>{void handler(req,res)})
if(apiOrigin.startsWith('http://127.0.0.1:'))await new Promise<void>(r=>apiServer.listen(Number(new URL(apiOrigin).port),'127.0.0.1',r))
else await context.route(apiOrigin+'/**',async route=>{
 const request=route.request(),headers=await request.allHeaders()
 const response=await fetch(origin+new URL(request.url()).pathname,{method:request.method(),headers:{...headers,origin},body:request.postData()||undefined})
 await route.fulfill({status:response.status,headers:{...Object.fromEntries(response.headers),'access-control-allow-origin':origin},body:await response.text()})
})
await context.route('**/*',route=>{const url=new URL(route.request().url());return url.origin===origin||url.origin===apiOrigin?route.fallback():route.abort()})
await context.addCookies([{name:'fixture',value:'1',url:origin}])
await context.addCookies([{name:'fixture',value:'1',url:apiOrigin,...apiOrigin.startsWith('https:')?{secure:true,sameSite:'None' as const}:{}}])
try{
 await page.goto(origin+'/connect')
 await page.getByRole('button',{name:'Stop account analytics',exact:true}).waitFor({state:'visible'})
 assert.equal(await page.getByText('Daily service-activity totals.',{exact:true}).isVisible(),true,'aggregate disclosure is visible without opening individual consent')
 await page.getByText('Optional account analytics',{exact:true}).click({timeout:3000})
 const checkbox=page.getByRole('checkbox',{name:'Allow account analytics',exact:true})
 await checkbox.waitFor();assert.equal(await checkbox.isChecked(),false);assert.equal(mutations.length,0)
 await checkbox.check();await page.getByRole('status').filter({hasText:'Account analytics is on'}).waitFor()
 assert.equal(consented,true);assert.equal(await checkbox.isChecked(),true);assert.equal(mutations.length,1)
 await page.reload();await page.getByText('Optional account analytics',{exact:true}).click();await checkbox.waitFor();assert.equal(await checkbox.isChecked(),true)
 available=false;await page.reload();await page.getByText('Optional account analytics',{exact:true}).click()
 await page.getByRole('button',{name:'Stop account analytics',exact:true}).click();await page.getByRole('status').filter({hasText:'Account analytics is off'}).waitFor();assert.equal(consented,false)
 available=true;unconfirmed=true;await page.reload();await page.getByText('Optional account analytics',{exact:true}).click();await checkbox.check()
 await page.getByRole('alert').filter({hasText:'could not be confirmed'}).waitFor();assert.equal(consented,false)
 unconfirmed=false;malformed=true;await page.reload();await page.getByText('Optional account analytics',{exact:true}).click();await page.getByRole('alert').filter({hasText:'could not be loaded'}).waitFor()
 assert.equal(await checkbox.count(),0);await page.getByRole('button',{name:'Stop account analytics',exact:true}).click();assert.equal(consented,false)
 malformed=false;await page.reload();await page.getByText('Optional account analytics',{exact:true}).click();await checkbox.waitFor()
 holdPositive=true;await checkbox.check();await page.getByRole('button',{name:'Stop account analytics',exact:true}).click()
 await page.getByRole('status').filter({hasText:'Account analytics is off'}).waitFor()
 const rejected=page.waitForResponse(r=>r.url().endsWith('/v1/analytics/consent')&&r.request().method()==='POST',{timeout:5000})
 releasePositive();holdPositive=false;assert.equal((await rejected).status(),409)
 assert.equal(consented,false);assert.equal(await checkbox.isChecked(),false);assert.equal(await page.getByRole('alert').count(),0)
 await page.screenshot({path:resolve(evidence,'account-analytics-desktop.png'),fullPage:true})
 await page.setViewportSize({width:320,height:900});await page.screenshot({path:resolve(evidence,'account-analytics-320.png'),fullPage:true})
 assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true)
 for(const storage of await page.evaluate(()=>[JSON.stringify({...localStorage}),JSON.stringify({...sessionStorage})]))assert.doesNotMatch(storage,/csrf|synthetic@example|fixture/)
 assert.deepEqual(errors,[])
 console.log(JSON.stringify({passed:['initial-off/no-enrollment','explicit-opt-in-and-readback','reload-persistence','withdrawal-while-unavailable','unconfirmed-write-refusal','malformed-read-fail-closed','withdrawal-beats-late-opt-in','320px-layout','no-credential-persistence'],evidence,limits:'Synthetic packaged Chromium UI only. No provider writes or production acceptance.'},null,2))
}finally{releasePositive();await browser.close();await new Promise<void>(r=>server.close(()=>r()));if(apiServer.listening)await new Promise<void>(r=>apiServer.close(()=>r()))}
