// Own the Easter egg per mounted frog; retain the existing React single-click greeting.
export function installFrogSplash(original: HTMLButtonElement) {
  let previousTap = -Infinity;
  let active = false;

  let layer: HTMLDialogElement;
  let flight: HTMLImageElement;
  let waterFrame: number;
  let waterSurface: HTMLElement | null = null;
  let savedFilter = "";
  let animations: Animation[] = [];
  let timers: number[] = [];
  let savedOverflow = '';
  const later = (fn: () => void, delay: number) => timers.push(window.setTimeout(fn, delay));
  function reset(restoreFocus = true) {
    if (!active) return;
    cancelAnimationFrame(waterFrame);
    if(waterSurface) waterSurface.style.filter = savedFilter;
    timers.forEach(clearTimeout); timers = [];
    animations.forEach(a => a.cancel()); animations = [];
    layer?.remove(); flight?.remove();
    if (original) { original.style.visibility = ''; if (restoreFocus) original.focus({preventScroll:true}); }
    document.documentElement.style.overflow = savedOverflow;
    active = false; previousTap = -Infinity;
  }
  function play(frog: HTMLButtonElement) {
    if (active) return;
    active = true;
    const rect = frog.getBoundingClientRect();
    const sourceX = rect.left + rect.width / 2;
    const x = innerWidth > 700 ? Math.max(100, sourceX - Math.min(330, innerWidth * .23)) : Math.min(innerWidth - 70, sourceX + innerWidth * .38);
    const dx = x - sourceX;
    const y = rect.bottom - 8;
    savedOverflow = document.documentElement.style.overflow;
    document.documentElement.style.overflow = 'hidden';
    layer = document.createElement('dialog');
    layer.className = 'frog-splash-dialog';
    layer.setAttribute('aria-label', 'A little surprise from the frog');
    layer.innerHTML = `<div class="splash-puddle" aria-hidden="true"></div><svg class="water-defs" aria-hidden="true"><defs><filter id="pond-water-refraction" x="-2%" y="-2%" width="104%" height="104%"><feTurbulence type="fractalNoise" baseFrequency=".012 .024" numOctaves="1" seed="7" result="water"/><feDisplacementMap in="SourceGraphic" in2="water" scale="0" xChannelSelector="R" yChannelSelector="G"/></filter></defs></svg><svg class="splash-ripples" aria-hidden="true" overflow="visible">${Array.from({length:5},(_,i)=>`<g style="--delay:${i*130}ms"><ellipse class="water-shade" rx="80" ry="38"/><ellipse class="water-light" rx="78" ry="36"/></g>`).join('')}</svg><div class="splash-flood" aria-hidden="true"></div>
      <svg class="splash-impact" viewBox="0 0 400 300" aria-hidden="true"><path d="M30 265 Q75 245 90 230 L65 144 Q63 128 78 139 L128 194 L143 65 Q148 43 159 67 L180 183 L213 18 Q222 0 229 22 L230 177 L295 75 Q312 57 304 85 L279 206 L351 161 Q369 153 357 171 L323 238 Q360 251 382 265Z" fill="currentColor"/></svg>
      <div class="splash-drops" aria-hidden="true">${Array.from({length:9},(_,i)=>`<i style="--i:${i};--dx:${Math.cos(i*2.4)*Math.min(innerWidth*.36,260)}px;--dy:${-90-(i%4)*38}px"></i>`).join('')}</div>
      <div class="splash-message"><a class="splash-destination" href="https://app.tryapparent.com/" aria-label="Open apparent. QR code links to app.tryapparent.com"><img src="/pond/apparent-qr.svg" alt="QR code for https://app.tryapparent.com/"/><span>app.tryapparent.com ↗</span></a><button class="splash-close" type="button">Back to the pond</button></div>`;
    layer.style.setProperty('--impact-x', `${x}px`);
    layer.style.setProperty('--impact-y', `${y}px`);
    document.body.append(layer);
    layer.addEventListener('cancel', event => {event.preventDefault();reset();});
    layer.querySelector<HTMLButtonElement>('.splash-close')!.onclick = () => reset();
    layer.showModal();
    layer.dataset.stage = 'jump';
    // Keep the native dialog from stealing attention until the reveal appears.
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
      layer.dataset.stage = 'reveal';
      layer.querySelector<HTMLAnchorElement>('a')!.focus();
      return;
    }
    flight = document.createElement('img');
    flight.src = '/pond/frog-air-cohesive.png';
    flight.alt = '';
    flight.className = 'splash-flying-frog';
    Object.assign(flight.style,{left:`${rect.left}px`,top:`${rect.top}px`,width:`${rect.width}px`,height:`${rect.height}px`});
    layer.append(flight);
    frog.style.visibility = 'hidden';
    const height = Math.max(40, Math.min(rect.top - Math.max(40,rect.width*.65), innerHeight*.55));
    const jump = flight.animate([
      {transform:'translate(0,0) rotate(0deg) scale(1)'},
      {transform:'translate(0,8px) rotate(0deg) scale(1.06,.88)',offset:.1},
      {transform:`translate(${dx*.28}px,${-height*.75}px) rotate(-110deg) scale(1)`,offset:.3},
      {transform:`translate(${dx*.56}px,${-height}px) rotate(-240deg) scale(1)`,offset:.5},
      {transform:`translate(${dx*.83}px,${-height*.6}px) rotate(-335deg) scale(1.05,.9)`,offset:.74},
      {transform:`translate(${dx}px,0) rotate(-360deg) scale(1.12,.72)`,offset:1}
    ],{duration:1150,easing:'linear',"fill":"forwards"});
    animations.push(jump);
    later(()=>{
      flight.src = '/pond/frog-lounge-cohesive.png';
      flight.animate([{opacity:1,translate:"0 0"},{opacity:0,translate:"0 14px"}],{duration:160,"fill":"forwards"});
      later(()=>flight.remove(),160);
      layer.dataset.stage = 'impact';
      later(()=>{
        layer.dataset.stage = 'ripple';
        waterSurface = document.querySelector<HTMLElement>('.pond-page');
        savedFilter = waterSurface?.style.filter ?? "";
        if (waterSurface) waterSurface.style.filter = 'url(#pond-water-refraction)';
        const displacement = layer.querySelector('feDisplacementMap')!;
        const started = performance.now();
        function refract(now: number){
          const t = Math.min(1,(now-started)/1600);
          displacement.setAttribute('scale',String(Math.sin(t*Math.PI)*4));
          if(t<1 && active) waterFrame=requestAnimationFrame(refract);
          else if (waterSurface) waterSurface.style.filter=savedFilter;
        }
        waterFrame=requestAnimationFrame(refract);
      },130);
      later(()=>layer.dataset.stage = 'flood',460);
      later(()=>{layer.dataset.stage='reveal';layer.querySelector<HTMLAnchorElement>('a')!.focus({preventScroll:true});},1280);
    },1150);
  }
  const onClick = (event: MouseEvent) => {
    if (active) return;
    const now = performance.now();
    if (now - previousTap < 420) {
      event.preventDefault(); event.stopImmediatePropagation();
      previousTap = -Infinity; play(original);
    } else previousTap = now;
  };
  const onPageHide = () => reset(false);
  original.addEventListener('click', onClick, true);
  window.addEventListener('pagehide', onPageHide);
  return () => {
    reset(false);
    original.removeEventListener('click', onClick, true);
    window.removeEventListener('pagehide', onPageHide);
  };
}
