// Pending pages keep their bounded checks independent of React hydration.
export function pollReady(root: HTMLElement) {
  const panel = root.querySelector<HTMLElement>('[data-share-poll]')
  if (!panel) return () => {}
  const output = panel.querySelector<HTMLElement>('[data-poll-status]')
  const progress = panel.querySelector<HTMLElement>('[data-upload-progress]')
  const url = new URL(panel.dataset.manifest || '', location.href)
  if (url.origin !== location.origin) return () => {}
  const deadline = Date.now() + 120_000
  let attempts = 0, stopped = false
  let timer: ReturnType<typeof setTimeout>
  function stop() { if (output) output.textContent = 'Automatic checks stopped after two minutes. Reload later or ask the sender to check the upload.' }
  async function poll() {
    if (stopped) return
    if (Date.now() >= deadline) { stop(); return }
    const nextCheck = Date.now() + 5000
    attempts++
    try {
      const response = await fetch(url.href, { credentials: 'omit', cache: 'no-store', redirect: 'error', referrerPolicy: 'no-referrer', signal: AbortSignal.timeout(Math.max(1, Math.min(4500, deadline - Date.now()))) })
      if (stopped) return
      if ([404, 410].includes(response.status)) { location.reload(); return }
      if (response.ok) {
        const manifest = await response.json() as { status?: string; uploadedBytes?: number; totalBytes?: number }
        if (['ready', 'paused', 'failed', 'expired', 'revoked', 'deleted'].includes(manifest.status || '')) { location.reload(); return }
        if (manifest.status === 'pending' && progress && Number.isSafeInteger(manifest.uploadedBytes) && Number.isSafeInteger(manifest.totalBytes) && manifest.uploadedBytes! >= 0 && manifest.uploadedBytes! <= manifest.totalBytes!) progress.textContent = `${manifest.uploadedBytes} of ${manifest.totalBytes} bytes uploaded.`
      }
    } catch { /* Never log capability URLs or captured content. */ }
    if (!stopped && attempts < 24 && Date.now() < deadline) timer = setTimeout(poll, Math.max(0, Math.min(nextCheck, deadline) - Date.now()))
    else if (!stopped) stop()
  }
  timer = setTimeout(poll, 5000)
  return () => { stopped = true; clearTimeout(timer) }
}
