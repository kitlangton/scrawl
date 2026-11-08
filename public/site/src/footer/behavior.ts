// Lifecycle-scoped animation, launch attempt and dialogs. Never records or sends feedback.
export function mountFooter(root: HTMLElement): () => void {
  const wheel = root.querySelector<HTMLButtonElement>('.footer-wheel')!
  const word = wheel.querySelector<HTMLSpanElement>('span')!
  const phrases = ['with love', 'while running', 'with club mate', 'at C<>DE']
  const motion = matchMedia('(prefers-reduced-motion: reduce)')
  let active = true, paused = false, hover = false, focused = false, index = 0
  let timer = 0, frame = 0
  const cleanup: (() => void)[] = []
  function listen(target: EventTarget, name: string, listener: EventListener) {
    target.addEventListener(name, listener)
    cleanup.push(() => target.removeEventListener(name, listener))
  }
  const tryButton = root.querySelector<HTMLButtonElement>('.footer-try')!
  listen(tryButton, 'click', () => {
    if (tryButton.dataset.launchAttempted !== 'true') {
      // Change the action before leaving the page. Browsers cannot confirm
      // installation or launch; the next click always downloads by choice.
      tryButton.dataset.launchAttempted = 'true'
      tryButton.textContent = 'Not installed yet? No worries—click again to download ↗'
      try { window.location.assign('apparent://open') } catch { /* Keep the download action available. */ }
      return
    }
    // Reuse the actual hero link, including its platform/CPU selection, busy
    // guard and release-list fallback. Do not maintain a second download path.
    const download = document.querySelector<HTMLAnchorElement>('.pond-download a')
    if (download) download.click()
    else window.location.assign('https://github.com/tryapparent/apparent/releases')
  })
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
  const dialogs = [...root.querySelectorAll<HTMLDialogElement>('dialog')]
  let returnFocus: HTMLElement | null = null
  for (const button of root.querySelectorAll<HTMLButtonElement>('[data-open]')) {
    listen(button, 'click', () => {
      const dialog = dialogs.find(d => d.dataset.dialog === button.dataset.open)!
      const open = dialogs.find(d => d.open)
      if (!open) returnFocus = button
      if (open === dialog) return
      open?.close()
      dialog.showModal()
    })
  }
  for (const dialog of dialogs) {
    listen(dialog.querySelector('.footer-close')!, 'click', () => dialog.close())
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
    private cleanup?: () => void
    connectedCallback() { this.cleanup?.(); this.cleanup = mountFooter(this) }
    disconnectedCallback() { this.cleanup?.(); this.cleanup = undefined }
  }
  customElements.define('apparent-footer', ApparentFooter)
}
