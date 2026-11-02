import { hydrateRoot } from 'react-dom/client'
import { ShareViewer } from './viewer.tsx'
import type { ViewerModel } from './model.ts'
import { pollReady } from './poll.ts'
const root = document.getElementById('share-root')
if (root) {
  try {
    const model: ViewerModel = JSON.parse(root.dataset.shareModel || '{}')
    hydrateRoot(root, <ShareViewer model={model} />)
    if (model.status === 'pending') pollReady(root)
  } catch { /* Server-rendered recording and evidence remain usable. */ }
}
