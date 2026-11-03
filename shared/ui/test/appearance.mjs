// Exact packaged surfaces, synthetic/disconnected data only. No external requests.
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile, mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, extname, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium, firefox } from '../../../connect/web/node_modules/playwright-core/index.mjs'
import { renderShare } from '../../../connect/service/.generated/recipient-render.js'
const root = fileURLToPath(new URL('../../../', import.meta.url))
const publicDir = process.env.PUBLIC_THEME_ARTIFACT || 'public/site/dist'
const evidence = process.env.THEME_EVIDENCE || await mkdtemp(join(tmpdir(), 'apparent-theme-'))
const recipient = renderShare({url:'',status:'pending',uploadedBytes:0,totalBytes:1,files:[]})
const mime={'.html':'text/html','.css':'text/css','.js':'text/javascript','.svg':'image/svg+xml','.png':'image/png','.ttf':'font/ttf'}
const server=createServer(async(req,res)=>{
 try {
  const url=new URL(req.url,'http://localhost')
  if(url.pathname==='/s/theme') {res.setHeader('Content-Type','text/html');res.end(recipient);return}
  if(url.pathname.startsWith('/share-')) {
   const assets=await import('../../../connect/service/.generated/recipient-assets.ts')
   const name=url.pathname.slice(1);const value={'share-view.css':assets.shareViewStyles,'share-view.js':assets.shareViewScript,'share-wordmark.svg':assets.shareWordmark,'share-font.ttf':Buffer.from(assets.shareFont,'base64')}[name]
   res.setHeader('Content-Type',mime[extname(name)]||'application/octet-stream');res.end(value);return
  }
  const dirs=['capture/desktop/ui/frontend/dist','connect/web/dist',publicDir]
  let relative=url.pathname.slice(1)
  if(url.pathname.endsWith('/'))relative+='index.html'
  let content,path
  for(const dir of dirs){
    const base=resolve(root,dir);path=resolve(base,relative==='foundation.html'?'index.html':relative)
    if(!path.startsWith(base+sep))continue
    if(relative==='index.html'&&dir!==publicDir)continue
    try{content=await readFile(path);break}catch{}
  }
  if(!content){res.writeHead(404).end();return}
  res.setHeader('Content-Type',mime[extname(path)]||'application/octet-stream');res.end(content)
 }catch {res.writeHead(404).end()}
})
await new Promise(r=>server.listen(0,'127.0.0.1',r))
const origin=`http://127.0.0.1:${server.address().port}`
const surfaces=[['desktop','settings.html','.settings-shell'],['desktop','foundation.html','body'],['connect','connect/','.hosted-page'],['public','','.pond-page'],['recipient','s/theme','html']].filter(([owner])=>!process.env.THEME_SURFACES||process.env.THEME_SURFACES.split(',').includes(owner))
try {
 for(const [name,engine,path] of [['chromium',chromium,process.env.CHROME_PATH||chromium.executablePath()],['firefox',firefox,process.env.FIREFOX_PATH||firefox.executablePath()]]){
  if(process.env.THEME_BROWSERS&&!process.env.THEME_BROWSERS.split(',').includes(name))continue
  const browser=await engine.launch({executablePath:path,headless:true,args:name==='chromium'?['--no-sandbox']:[]})
  try{
   const context=await browser.newContext({colorScheme:'light',viewport:{width:1100,height:800}})
   await context.route('**/*',route=>route.request().url().startsWith(origin+'/')?route.continue():route.abort())
   for(const [owner,path,selector] of surfaces){
    const page=await context.newPage();await page.goto(`${origin}/${path}`);await page.locator(selector).waitFor()
    if(owner==='public'&&process.env.PUBLIC_THEME_ARTIFACT)await page.locator('.preview-statement').waitFor()
    const states=[]
    for(const mode of ['light','dark','light']){
     await page.emulateMedia({colorScheme:mode})
     await page.waitForFunction(mode=>getComputedStyle(document.documentElement).colorScheme===mode,mode)
     const state=await page.locator(selector).evaluate(el=>({background:getComputedStyle(el).backgroundColor,foreground:getComputedStyle(el).color,token:getComputedStyle(el).getPropertyValue('--color-background').trim()}))
     const pairs=await page.evaluate(()=>{
      const names=[['foreground','background'],['muted-foreground','muted'],['primary-foreground','primary'],['secondary-foreground','secondary'],['accent-foreground','accent'],['destructive-foreground','destructive'],['selection-foreground','selection'],['warning-foreground','warning-surface']]
      const probe=document.createElement('span');document.body.append(probe)
      const result=names.map(([foreground,background])=>{probe.style.color=`var(--color-${foreground})`;probe.style.backgroundColor=`var(--color-${background})`;const css=getComputedStyle(probe);return [foreground,css.color,css.backgroundColor]})
      probe.remove();return result
     })
     const luminance=color=>color.match(/\d+/g).slice(0,3).map(Number).map(x=>x/255).map(x=>x<=.04045?x/12.92:((x+.055)/1.055)**2.4).reduce((sum,x,i)=>sum+x*[.2126,.7152,.0722][i],0)
     for(const [role,fg,bg] of pairs){const a=luminance(fg),b=luminance(bg);assert((Math.max(a,b)+.05)/(Math.min(a,b)+.05)>=4.5,`${name}/${owner} ${mode} ${role} contrast`)}
     if(owner==='connect'||owner==='public'){
      const selection=await page.locator(selector).evaluate(el=>{const css=getComputedStyle(el.querySelector('h1') || el,'::selection');return [css.color,css.backgroundColor]})
      const [a,b]=selection.map(luminance);assert((Math.max(a,b)+.05)/(Math.min(a,b)+.05)>=4.5,`${owner} ${mode} selection contrast`)
     }
     states.push(state)
     assert.equal(state.background,mode==='dark'?'rgb(32, 27, 36)':'rgb(245, 240, 232)',`${name}/${owner}/${path} ${mode} background`)
     assert.equal(state.foreground,mode==='dark'?'rgb(245, 240, 232)':owner==='public'&&process.env.PUBLIC_THEME_ARTIFACT?'rgb(2, 1, 2)':'rgb(25, 22, 29)',`${name}/${owner}/${path} ${mode} text`)
     if(owner==='public'&&process.env.PUBLIC_THEME_ARTIFACT){
      const patches=await page.evaluate(()=>{
       const statement=document.querySelector('.preview-statement'), hello=document.querySelector('.pond-hello'), bubble=document.querySelector('.rude-bubble')
       const pair=(el,bg)=>[getComputedStyle(el).color,bg||getComputedStyle(el).backgroundColor]
       return [pair(statement),...(hello?[pair(hello)]:[]),...(bubble?[pair(bubble,getComputedStyle(bubble.querySelector('svg')).fill)]:[])]
      })
      for(const [fg,bg] of patches){const a=luminance(fg),b=luminance(bg);assert((Math.max(a,b)+.05)/(Math.min(a,b)+.05)>=4.5,`published dynamic patch ${mode} contrast: ${fg} / ${bg}`)}
     }
     if(mode==='dark')await page.screenshot({path:join(evidence,`${name}-${owner}-${path.replaceAll('/','-')||'index'}-dark.png`),fullPage:true})
    }
    assert.deepEqual(states[0],states[2]);await page.close()
    console.log(`${name}: ${owner}/${path} live light → dark → light passed`)
   }
   await context.close()
  }finally{await browser.close()}
 }
}finally{await new Promise(r=>server.close(r))}
console.log(`Synthetic appearance screenshots: ${evidence}`)
