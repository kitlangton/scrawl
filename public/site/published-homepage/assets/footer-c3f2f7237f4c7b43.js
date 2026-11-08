// Lifecycle-scoped animation and dialogs. No recording, upload or outbound messages.
export function mountFooter(root             )             {
  const wheel = root.querySelector                   ('.footer-wheel') 
  const word = wheel.querySelector                 ('span') 
  const phrases = ['with love', 'while running', 'with club mate', 'at C<>DE']
  const motion = matchMedia('(prefers-reduced-motion: reduce)')
  let active = true, paused = false, hover = false, focused = false, index = 0
  let timer = 0, frame = 0
  const cleanup                 = []
  function listen(target             , name        , listener               ) {
    target.addEventListener(name, listener)
    cleanup.push(() => target.removeEventListener(name, listener))
  }
  function fit() {
    if (!active) return
    // Layout width is stable while the rolling word is transformed in 3D.
    wheel.style.width = `${word.offsetWidth + 22}px`
  }
  function label() {
    wheel.disabled = motion.matches
    wheel.setAttribute('aria-label', `Made ${phrases[index]} in Berlin. ${motion.matches ? 'Rotation disabled by reduced-motion preference' : paused ? 'Resume rotation' : 'Pause rotation'}`)
    wheel.setAttribute('aria-pressed', String(paused || motion.matches))
  }
  function stop() {
    clearTimeout(timer)
    cancelAnimationFrame(frame)
    wheel.removeAttribute('data-step')
  }
  function schedule() {
    if (!active || paused || hover || focused || motion.matches || document.hidden) return
    timer = window.setTimeout(next, 3200)
  }
  function next() {
    if (!active) return
    wheel.dataset.step = 'out'
    timer = window.setTimeout(() => {
      if (!active) return
      index = (index + 1) % phrases.length
      word.textContent = phrases[index]
      wheel.dataset.step = 'in'
      fit()
      label()
      // Two frames give the incoming transform a rendered start state.
      frame = requestAnimationFrame(() => {
        frame = requestAnimationFrame(() => {
          if (!active) return
          wheel.removeAttribute('data-step')
          timer = window.setTimeout(schedule, 240)
        })
      })
    }, 220)
  }
  function restart() { stop(); label(); schedule() }
  listen(wheel, 'click', () => { paused = !paused; restart() })
  listen(wheel, 'mouseenter', () => { hover = true; restart() })
  listen(wheel, 'mouseleave', () => { hover = false; restart() })
  listen(wheel, 'focus', () => { focused = true; restart() })
  listen(wheel, 'blur', () => { focused = false; restart() })
  listen(motion, 'change', restart)
  listen(document, 'visibilitychange', restart)
  listen(window, 'resize', fit)
  document.fonts.ready.then(() => { if (active) fit() })
  const dialogs = [...root.querySelectorAll                   ('dialog')]
  let returnFocus                     = null
  for (const button of root.querySelectorAll                   ('[data-open]')) {
    listen(button, 'click', () => {
      const dialog = dialogs.find(d => d.dataset.dialog === button.dataset.open) 
      const open = dialogs.find(d => d.open)
      if (!open) returnFocus = button
      if (open === dialog) return
      open?.close()
      dialog.showModal()
    })
  }
  for (const dialog of dialogs) {
    listen(dialog.querySelector('.footer-close') , 'click', () => dialog.close())
    listen(dialog, 'close', () => {
      if (active && !dialogs.some(d => d.open)) returnFocus?.focus()
    })
  }
  fit(); label(); schedule()
  return () => {
    active = false
    stop()
    cleanup.forEach(remove => remove())
    dialogs.forEach(dialog => { if (dialog.open) dialog.close() })
  }
}

if (typeof customElements !== 'undefined' && !customElements.get('apparent-footer')) {
  class ApparentFooter extends HTMLElement {
            cleanup             
    connectedCallback() { this.cleanup?.(); this.cleanup = mountFooter(this) }
    disconnectedCallback() { this.cleanup?.(); this.cleanup = undefined }
  }
  customElements.define('apparent-footer', ApparentFooter)
}
