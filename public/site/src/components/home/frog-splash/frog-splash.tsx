import { useLayoutEffect, useRef, useState, type CSSProperties } from 'react'
import { createPortal } from 'react-dom'

type Stage = 'jump' | 'impact' | 'ripple' | 'flood' | 'reveal'
type Props = { original: HTMLButtonElement; onDismiss: () => void }

// React owns the dialog and its stages; browser APIs execute the visual effects.
export function FrogSplash({ original, onDismiss }: Props) {
  const dialog = useRef<HTMLDialogElement>(null)
  const flying = useRef<HTMLImageElement>(null)
  const [rect] = useState(() => original.getBoundingClientRect())
  const [reduced] = useState(() => matchMedia('(prefers-reduced-motion: reduce)').matches)
  const [stage, setStage] = useState<Stage>(reduced ? 'reveal' : 'jump')
  const [landed, setLanded] = useState(false)
  const [flightVisible, setFlightVisible] = useState(!reduced)
  const [viewport] = useState(() => ({ width: innerWidth, height: innerHeight }))
  const sourceX = rect.left + rect.width / 2
  const x = viewport.width > 700 ? Math.max(100, sourceX - Math.min(330, viewport.width * .23)) : Math.min(viewport.width - 70, sourceX + viewport.width * .38)
  const y = rect.bottom - 8

  useLayoutEffect(() => {
    const layer = dialog.current!
    const flight = flying.current!
    const dx = x - sourceX
    let active = true
    let restoreFocus = true
    let waterFrame = 0
    let waterSurface: HTMLElement | null = null
    let savedFilter = ''
    const animations: Animation[] = []
    const timers: number[] = []
    const later = (fn: () => void, delay: number) => timers.push(window.setTimeout(fn, delay))
    const savedOverflow = document.documentElement.style.overflow
    const savedVisibility = original.style.visibility
    document.documentElement.style.overflow = 'hidden'
    layer.showModal()
    const onPageHide = () => { restoreFocus = false; onDismiss() }
    window.addEventListener('pagehide', onPageHide)
    if (!reduced) {
      original.style.visibility = 'hidden'
      const height = Math.max(40, Math.min(rect.top - Math.max(40,rect.width*.65), viewport.height*.55));
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
        setLanded(true);
        animations.push(flight.animate([{opacity:1,translate:"0 0"},{opacity:0,translate:"0 14px"}],{duration:160,"fill":"forwards"}));
        later(()=>setFlightVisible(false),160);
        setStage('impact');
        later(()=>{
          setStage('ripple');
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
        later(()=>setStage('flood'),460);
        later(()=>{setStage('reveal');},1280);
      },1150);
    }
    return () => {
      active = false
      cancelAnimationFrame(waterFrame)
      timers.forEach(clearTimeout)
      animations.forEach(animation => animation.cancel())
      if (waterSurface) waterSurface.style.filter = savedFilter
      original.style.visibility = savedVisibility
      layer.close()
      document.documentElement.style.overflow = savedOverflow
      window.removeEventListener('pagehide', onPageHide)
      if (restoreFocus && original.isConnected) original.focus({ preventScroll: true })
    }
  }, [original, onDismiss, reduced, rect, x, sourceX, viewport])

  useLayoutEffect(() => {
    if (stage === 'reveal') dialog.current?.querySelector<HTMLAnchorElement>('a')?.focus({ preventScroll: true })
  }, [stage])

  return createPortal(<dialog ref={dialog} className="frog-splash-dialog" aria-label="A little surprise from the frog" data-stage={stage} onCancel={event => { event.preventDefault(); onDismiss() }} style={{ '--impact-x': `${x}px`, '--impact-y': `${y}px` } as CSSProperties}>
    <div className="splash-puddle" aria-hidden="true" />
    <svg className="water-defs" aria-hidden="true"><defs><filter id="pond-water-refraction" x="-2%" y="-2%" width="104%" height="104%"><feTurbulence type="fractalNoise" baseFrequency=".012 .024" numOctaves="1" seed="7" result="water" /><feDisplacementMap in="SourceGraphic" in2="water" scale="0" xChannelSelector="R" yChannelSelector="G" /></filter></defs></svg>
    <svg className="splash-ripples" aria-hidden="true" overflow="visible">{Array.from({ length: 5 }, (_, i) => <g key={i} style={{ '--delay': `${i * 130}ms` } as CSSProperties}><ellipse className="water-shade" rx="80" ry="38" /><ellipse className="water-light" rx="78" ry="36" /></g>)}</svg>
    <div className="splash-flood" aria-hidden="true" />
    <svg className="splash-impact" viewBox="0 0 400 300" aria-hidden="true"><path d="M30 265 Q75 245 90 230 L65 144 Q63 128 78 139 L128 194 L143 65 Q148 43 159 67 L180 183 L213 18 Q222 0 229 22 L230 177 L295 75 Q312 57 304 85 L279 206 L351 161 Q369 153 357 171 L323 238 Q360 251 382 265Z" fill="currentColor" /></svg>
    <div className="splash-drops" aria-hidden="true">{Array.from({ length: 9 }, (_, i) => <i key={i} style={{ '--i': i, '--dx': `${Math.cos(i * 2.4) * Math.min(viewport.width * .36, 260)}px`, '--dy': `${-90 - (i % 4) * 38}px` } as CSSProperties} />)}</div>
    <div className="splash-message"><a className="splash-destination" href="https://app.tryapparent.com/" aria-label="Open apparent. QR code links to app.tryapparent.com"><img src="/pond/apparent-qr.svg" alt="QR code for https://app.tryapparent.com/" /><span>app.tryapparent.com ↗</span></a><button className="splash-close" type="button" onClick={onDismiss}>Back to the pond</button></div>
    {flightVisible && <img ref={flying} src={landed ? '/pond/frog-lounge-cohesive.png' : '/pond/frog-air-cohesive.png'} alt="" className="splash-flying-frog" style={{ left: rect.left, top: rect.top, width: rect.width, height: rect.height }} />}
  </dialog>, document.body)
}
