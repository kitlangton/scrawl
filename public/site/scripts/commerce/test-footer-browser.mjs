import assert from 'node:assert/strict'
import {createServer} from 'node:http'
import {readFile,writeFile,mkdir} from 'node:fs/promises'
import {resolve,extname} from 'node:path'
import {chromium} from 'playwright-core'
assert(process.env.SITE_ARTIFACT&&process.env.SITE_EVIDENCE,'SITE_ARTIFACT and SITE_EVIDENCE required')
const root=resolve(process.env.SITE_ARTIFACT),evidence=resolve(process.env.SITE_EVIDENCE)
await mkdir(evidence,{recursive:true})
const server=createServer(async(req,res)=>{
  const pathname=new URL(req.url,'http://localhost').pathname
  const file=resolve(root,'.'+(pathname==='/'?'/index.html':pathname))
  if(!file.startsWith(root+'/'))return res.writeHead(403).end()
  try{
    res.setHeader('Content-Type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.webp':'image/webp','.png':'image/png','.svg':'image/svg+xml','.jpg':'image/jpeg','.ttf':'font/ttf','.mp4':'video/mp4'})[extname(file)]||'text/plain')
    res.end(await readFile(file))
  }catch{res.writeHead(404).end()}
})
await new Promise(r=>server.listen(0,'127.0.0.1',r))
const browser=await chromium.launch({executablePath:process.env.CHROME_PATH||'/usr/bin/google-chrome',headless:true,args:['--no-sandbox']})
const results=[]
try{
  for(const width of [1280,768,390,320]){
    const page=await browser.newPage({viewport:{width,height:1000}})
    const errors=[]
    page.on('pageerror',e=>errors.push(e.message))
    page.on('console',m=>{if(m.type()==='error'&&/React|hydrat|match/i.test(m.text()))errors.push(m.text())})
    await page.goto(process.env.SITE_URL||`http://127.0.0.1:${server.address().port}/`)
    await page.evaluate(()=>document.fonts.ready)
    await page.locator('.frog-footer').scrollIntoViewIfNeeded()
    await page.waitForFunction(()=>customElements.get('apparent-footer'))
    const wheel=page.locator('.footer-wheel')
    assert.equal(await page.locator('.footer-greeting').textContent(),'Hey, fellow bottom-of-the-page person.')
    assert.equal(await page.locator('#footer-heading').textContent(),'We made this.You can make it better.')
    const geometry=await page.evaluate(()=>{
      const box=s=>document.querySelector(s).getBoundingClientRect()
      const brand=box('.footer-brand'),credit=box('.footer-credit'),base=box('.footer-base'),footer=box('.frog-footer'),scene=box('.footer-scene'),copy=box('.footer-invite')
      return {height:footer.height,scene:scene.height,base:base.height,beside:credit.left>brand.right&&Math.abs(credit.top-brand.top)<10,fits:copy.top>=scene.top&&copy.bottom<=scene.bottom,overflow:document.documentElement.scrollWidth>innerWidth}
    })
    assert.equal(geometry.overflow,false)
    assert(geometry.fits)
    assert(geometry.height <= (width>700?831:705),'no increase over accepted concept')
    if(width>700){assert(geometry.beside,'credit beside wordmark');assert(geometry.base<=88,'no separate credit row')}
    await page.waitForFunction(()=>[...document.querySelectorAll('.frog-footer img')].every(i=>i.complete&&i.naturalWidth))
    await wheel.focus()
    const held=await wheel.textContent()
    await page.waitForTimeout(3500)
    assert.equal(await wheel.textContent(),held,'keyboard focus pauses rotation')
    await wheel.press('Enter')
    assert.equal(await wheel.getAttribute('aria-pressed'),'true')
    await page.locator('.footer-github').focus()
    await page.mouse.move(1,1)
    await page.waitForTimeout(3500)
    assert.equal(await wheel.textContent(),held,'click pause persists after blur')
    await wheel.focus();await wheel.press('Enter');await page.locator('.footer-github').focus()
    if(width===1280){
      const samples=[]
      for(const phrase of ['with love','while running','with club mate','at C<>DE']){
        await page.waitForFunction(p=>document.querySelector('.footer-wheel span').textContent===p,phrase,{timeout:16000})
        await page.waitForTimeout(650)
        const m=await wheel.evaluate(e=>({text:e.textContent,width:e.getBoundingClientRect().width,textWidth:e.firstElementChild.getBoundingClientRect().width}))
        assert(Math.abs(m.width-m.textWidth-22)<2,'box follows text plus padding');samples.push(m)
      }
      assert(new Set(samples.map(s=>s.width)).size>=3)
      await writeFile(evidence+'/phrase-widths.json',JSON.stringify(samples,null,2))
    }
    await page.emulateMedia({reducedMotion:'reduce'})
    const reduced=await wheel.textContent()
    await page.waitForFunction(()=>document.querySelector('.footer-wheel').disabled)
    await page.waitForTimeout(3500)
    assert.equal(await wheel.textContent(),reduced)
    assert.equal(await wheel.evaluate(e=>getComputedStyle(e).transitionDuration),'0s')
    await page.screenshot({path:`${evidence}/footer-${width}.png`,fullPage:false})
    await page.locator('.footer-try').click()
    assert.match(await page.locator('.footer-try').textContent(),/Not installed yet\?.*click again to download/)
    assert.equal(await page.locator('dialog[open]').count(),0)
    await page.keyboard.press('Escape')
    await page.locator('.footer-hi').first().click()
    const social=page.locator('[data-dialog="social"]')
    assert.equal(await social.evaluate(e=>e.open),true)
    assert.deepEqual(await social.locator('a').evaluateAll(links=>links.map(a=>a.href)),['https://www.linkedin.com/in/olivia-allen-busin/','https://www.linkedin.com/in/simon-van-laak/'])
    await social.locator('.footer-close').focus();await page.keyboard.press('Enter')
    await page.waitForFunction(()=>document.querySelector('.footer-hi')===document.activeElement)
    assert.equal(await page.locator('.footer-hi').first().evaluate(e=>e===document.activeElement),true)
    await page.locator('.footer-hi').first().click();await page.keyboard.press('Escape')
    assert.equal(await page.locator('.footer-hi').first().evaluate(e=>e===document.activeElement),true)
    // A detached custom element must not receive later timer mutations.
    await page.emulateMedia({reducedMotion:'no-preference'})
    await page.locator('.footer-github').focus()
    await page.evaluate(()=>{window.detachedFooter=document.querySelector('apparent-footer');window.footerParent=window.detachedFooter.parentNode;window.detachedFooter.remove();window.footerAtDetach=window.detachedFooter.innerHTML})
    await page.waitForTimeout(3900)
    assert.equal(await page.evaluate(()=>window.detachedFooter.innerHTML===window.footerAtDetach),true)
    await page.evaluate(()=>window.footerParent.append(window.detachedFooter))
    await page.emulateMedia({reducedMotion:'reduce'})
    await page.waitForFunction(()=>document.querySelector('.footer-wheel').disabled)
    // Unrelated public content remains mounted.
    assert.equal(await page.locator('.pond-cover-frog').count(),1)
    assert.equal(await page.locator('.video-demo video').count(),1)
    assert.equal(await page.locator('#make-room').count(),1)
    assert.equal(await page.locator('.pond-team').count(),1)
    assert.deepEqual(errors,[])
    results.push({width,...geometry,status:'PASS'})
    await page.close()
  }
  await writeFile(evidence+'/results.json',JSON.stringify(results,null,2));console.log(JSON.stringify(results,null,2))
}finally{await browser.close();server.close()}
