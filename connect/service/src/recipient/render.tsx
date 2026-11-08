import { renderToString } from 'react-dom/server.edge'
import type { ShareModel } from '../types.ts'
import { viewerModel } from './model.ts'
import { messages, ShareViewer } from './viewer.tsx'
export function renderShare(input: ShareModel) {
  const model = viewerModel(input)
  const title = model.ready ? model.title : (messages[model.status]?.[0] || 'Recording unavailable')
  // React escapes captured text and sanitized props; there is no inline script,
  // account credential or non-ready evidence in the hydration payload.
  const page = renderToString(<html lang="en"><head><meta charSet="utf-8" /><meta name="viewport" content="width=device-width,initial-scale=1" /><meta name="robots" content="noindex, nofollow, noarchive" /><meta name="referrer" content="no-referrer" /><meta httpEquiv="Cache-Control" content="no-store" /><link rel="stylesheet" href="/share-view.css" /><title>{`${title} — Apparent`}</title><meta property="og:title" content={title} /><meta property="og:description" content={model.ready ? 'Recording, transcript and original evidence. No account needed.' : (messages[model.status]?.[1] || 'This recording is unavailable.')} />{model.ready && model.thumbnailUrl && <meta property="og:image" content={model.thumbnailUrl} />}</head><body><div id="share-root" data-share-model={JSON.stringify(model)}><ShareViewer model={model} /></div><script src="/share-view.js" defer /></body></html>)
  return '<!doctype html>' + page
}
