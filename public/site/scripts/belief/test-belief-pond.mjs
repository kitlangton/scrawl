import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile} from 'node:fs/promises';
import {resolve, extname} from 'node:path';
import {pathToFileURL} from 'node:url';
const root=resolve(process.env.SITE_ARTIFACT);
const evidence=resolve(process.env.SITE_EVIDENCE);
await (await import('node:fs/promises')).mkdir(evidence,{recursive:true});
const {chromium}=await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const server=createServer(async(req,res)=>{ const pathname=new URL(req.url,'http://localhost').pathname; const path=resolve(root,'.'+(pathname==='/'?'/index.html':pathname)); if(!path.startsWith(root+'/'))return res.writeHead(403).end(); try{res.setHeader('Content-Type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.png':'image/png','.svg':'image/svg+xml','.jpg':'image/jpeg','.mp4':'video/mp4','.ttf':'font/ttf'})[extname(path)]||'text/plain');res.end(await readFile(path));}catch{res.writeHead(404).end();}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await chromium.launch({executablePath:'/usr/bin/google-chrome',headless:true,args:['--no-sandbox']});
const results=[];
try{
 for(const [name,width,height] of [['desktop',1280,900],['mobile',375,812],['small-mobile',320,740],['wide-mobile',430,932]]){
  const page=await browser.newPage({viewport:{width,height}});const errors=[];let starsRateLimited=false;page.on('console',m=>{if(m.type()==='error'&&/hydrat|React error|didn't match/i.test(m.text()))errors.push(m.text());});page.on('pageerror',e=>errors.push(e.message));page.on('response',r=>{
    // The star badge deliberately falls back to GitHub when its public API is rate-limited.
    if(r.url()==='https://api.github.com/repos/tryapparent/apparent' && [403,429].includes(r.status())){starsRateLimited=true;return;}
    if(r.status()>=400)errors.push(`${r.status()} ${r.url()}`);
  });

  await page.goto(process.env.SITE_URL || `http://127.0.0.1:${server.address().port}/`);await page.evaluate(()=>document.fonts.ready);
  await page.evaluate(()=>scrollTo(0,scrollY+document.querySelector('#belief').getBoundingClientRect().top-innerHeight*.12));
  await page.waitForTimeout(60);
  assert.equal(await page.locator('#make-room').textContent(),'Me too','agreement action');
  assert.equal(await page.locator('#belief').textContent(),'We believe work should be enjoyable and leave us free for late nights with friends.');
  const unloaded=await page.evaluate(()=>[...document.images].filter(i=>i.getAttribute('src')&&i.loading!=='lazy'&&(!i.complete||!i.naturalWidth)).map(i=>i.getAttribute('src')));
  assert.deepEqual(unloaded,[],'eager image assets load');
  const box=selector=>page.locator(selector).boundingBox();
  const initial={left:await box('.pair-left'),right:await box('.pair-right'),frog:await box('.newcomer')};
  assert(initial.frog.x+initial.frog.width*.8<initial.left.x,'newcomer starts all the way left');
  assert(initial.left.x+initial.left.width<initial.right.x,'pair leaves a visible central gap');
  const scene=await box('.scene');const button=await box('#make-room');
  const frogCenter=initial.frog.x+initial.frog.width*.5;
  assert(Math.abs(button.x+button.width/2-frogCenter)<3,'button centered under single frog');
  assert(button.y>initial.frog.y+initial.frog.height,'button below single frog');
  assert(initial.left.x-(initial.frog.x+initial.frog.width*.87)>scene.width*.30,'newcomer substantially farther from pair');
  const small=await box('.pad-small'),big=await box('.pad-large');
  if(width<=600){
    for(const selector of ['#belief','#make-room','.newcomer','.pad-small','.pad-large','.pair-left','.pair-right','.hug']){
      const bounds=await box(selector);
      assert(bounds.x>=24 && bounds.x+bounds.width<=width-24,`${selector} has mobile edge clearance: ${JSON.stringify(bounds)}`);
    }
    const heading=await box('#belief');
    assert(initial.frog.y-heading.y-heading.height<=90,'mobile frogs stay close to the belief');
  }
  assert(big.width>small.width*1.8,'one big group pad and one small single pad');
  assert(Math.abs(small.x+small.width/2-frogCenter)<3,'small pad centered under newcomer');
  assert(big.x<initial.left.x&&big.x+big.width>=initial.right.x+initial.right.width-2,'large pad supports both waiting frogs');
  await page.screenshot({path:`${evidence}/${name}-idle.png`});
  await page.locator('#make-room').focus();assert.equal(await page.evaluate(()=>document.activeElement.id),'make-room');await page.keyboard.press('Enter');
  assert.equal(await page.locator('#study').getAttribute('data-state'),'moving');
  const animationCount=await page.evaluate(()=>document.querySelector('apparent-belief').getAnimations({subtree:true}).length);await page.keyboard.press('Enter');assert.equal(await page.evaluate(()=>document.querySelector('apparent-belief').getAnimations({subtree:true}).length),animationCount,'reentry guarded');
  // Sample the actual apex, not a wall-clock delay plus several protocol trips.
  // Pause every scene animation so screenshots and assertions see the same frame.
  await page.waitForFunction(()=>document.querySelector('.newcomer').dataset.pose==='air');
  const flight=await page.evaluate(async()=>{
    const animations=document.querySelector('apparent-belief').getAnimations({subtree:true});
    const jump=animations.find(a=>a.effect.target.matches('.newcomer')&&a.effect.getKeyframes().some(f=>f.transform));
    if(!jump)throw new Error('Expected an active newcomer jump');
    animations.forEach(a=>a.pause());
    await Promise.all(animations.map(a=>a.ready));
    const apex=jump.effect.getTiming().duration*.54;
    animations.forEach(a=>{a.currentTime=apex;});
    const bounds=selector=>document.querySelector(selector).getBoundingClientRect().toJSON();
    return {left:bounds('.pair-left'),right:bounds('.pair-right'),frog:bounds('.newcomer'),heading:bounds('#belief')};
  });
  assert(Math.abs(flight.left.x-initial.left.x)<1&&Math.abs(flight.right.x-initial.right.x)<1,'waiting frogs do not shuffle');
  assert(flight.frog.y<initial.frog.y-width*.055,'one high airborne arc, not a ground slide');
  assert(flight.frog.x>initial.frog.x+width*.04,'jump travels right');
  assert.equal(await page.locator('.newcomer').getAttribute('data-pose'),'air');
  assert(flight.frog.y>flight.heading.bottom,`jump stays clear of belief at apex: ${JSON.stringify(flight)}`);
  await page.screenshot({path:`${evidence}/${name}-airborne.png`});
  await page.evaluate(()=>document.querySelector('apparent-belief').getAnimations({subtree:true}).forEach(a=>a.play()));
  await page.waitForFunction(()=>document.querySelector('#study').dataset.state==='hugged');
  assert.equal(await page.locator('.hug').evaluate(e=>getComputedStyle(e).opacity),'1');
  assert.equal(await page.locator('#make-room').textContent(),'Again');
  assert.equal(await page.evaluate(()=>document.activeElement.id),'make-room');
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth&&document.querySelector('#make-room').getBoundingClientRect().bottom<=innerHeight));
  await page.screenshot({path:`${evidence}/${name}-hugged.png`});
  await page.keyboard.press('Space');assert.equal(await page.locator('#study').getAttribute('data-state'),'idle');
  await page.emulateMedia({reducedMotion:'reduce'});assert.deepEqual(await page.evaluate(()=>{document.querySelector('#make-room').click();return {state:document.querySelector('#study').dataset.state,animations:document.querySelector('apparent-belief').getAnimations({subtree:true}).length};}),{state:'hugged',animations:0});
  await page.locator('#make-room').click();await page.emulateMedia({reducedMotion:'no-preference'});await page.locator('#make-room').click();await page.emulateMedia({reducedMotion:'reduce'});await page.waitForFunction(()=>document.querySelector('#study').dataset.state==='hugged');assert.equal(await page.evaluate(()=>document.querySelector('apparent-belief').getAnimations({subtree:true}).length),0);
  await page.emulateMedia({reducedMotion:'no-preference'});
  await page.locator('#make-room').click();await page.locator('#make-room').click();
  await page.waitForTimeout(200);
  await page.evaluate(()=>{window.detachedBelief=document.querySelector('apparent-belief');window.beliefParent=window.detachedBelief.parentNode;window.detachedBelief.remove();});
  await page.waitForTimeout(1400);
  assert.deepEqual(await page.evaluate(()=>({state:window.detachedBelief.querySelector('#study').dataset.state,animations:window.detachedBelief.getAnimations({subtree:true}).length})),{state:'moving',animations:0},'cancelled completion cannot mutate detached scene');
  await page.evaluate(()=>window.beliefParent.appendChild(window.detachedBelief));
  assert.equal(await page.locator('#study').getAttribute('data-state'),'idle','reattach clears interrupted state');
  assert.equal(await page.locator('#make-room').getAttribute('aria-disabled'),null);
  await page.locator('#make-room').click();await page.waitForFunction(()=>document.querySelector('#study').dataset.state==='hugged');
  await page.locator('#make-room').click();await page.locator('#make-room').click();
  await page.setViewportSize({width:width-10,height});
  await page.waitForFunction(()=>document.querySelector('#study').dataset.state==='hugged');
  await page.setViewportSize({width,height});
  const samples=[];
  for(const progress of [0,.2,.48,.52,.7,1,.7,.52,.48,.2,0]){
    const marks=await page.evaluate(async p=>{
      const heading=document.querySelector('#belief');const origin=scrollY+heading.getBoundingClientRect().top-innerHeight*.7;
      scrollTo(0,origin+innerHeight*.58*p);await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
      return {circle:+document.querySelector('#study').dataset.circle,highlight:+document.querySelector('#study').dataset.highlight,offset:getComputedStyle(document.querySelector('.work-circle')).strokeDashoffset,transition:getComputedStyle(document.querySelector('.work-circle')).transitionDuration};
    },progress);
    const clamp=v=>Math.max(0,Math.min(1,v));
    assert(Math.abs(marks.circle-clamp(progress/.48))<.008,'circle follows exact scroll');
    assert(Math.abs(marks.highlight-clamp((progress-.55)/.45))<.008,'highlight follows exact scroll after circle');
    assert.equal(marks.transition,'0s');samples.push({progress,...marks});
    await page.waitForTimeout(80);assert.equal(await page.locator('.work-circle').evaluate(e=>getComputedStyle(e).strokeDashoffset),marks.offset,'no autoplay or drift while scroll stops');
  }
  await writeFile(`${evidence}/${name}-scroll-samples.json`,JSON.stringify(samples,null,2));
  if(starsRateLimited){
    assert.equal(await page.locator('.github-stars').getAttribute('aria-label'),'Star Apparent on GitHub','rate-limited optional counter keeps its honest fallback');
    assert.equal(await page.locator('.github-stars span').textContent(),'GitHub');
    console.log(`${name}: live GitHub API returned 403; verified optional counter fallback`);
  }
  await page.locator('#team').scrollIntoViewIfNeeded();
  await page.waitForFunction(()=>[...document.querySelectorAll('#team img')].every(i=>i.complete&&i.naturalWidth>0));
  assert.deepEqual(errors,[]);results.push({viewport:`${width}x${height}`,status:'PASS',checks:['unchanged belief','distant newcomer','Me too under single frog','large and small lily pads','central gap','static pair during flight','high single rightward jump','existing air pose','heading clearance','hug','keyboard','reentry guard','reset','reduced motion before and during flight','no overflow','exact sequential scroll mapping','reversible marks','no drift when stopped','all assets and no page errors']});await page.close();
 }
 await writeFile(`${evidence}/jump-results.json`,JSON.stringify(results,null,2));console.log(JSON.stringify(results,null,2));
}finally{await browser.close();server.close();}
