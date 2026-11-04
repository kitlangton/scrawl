/** Maintained consumer closures. Unknown inputs broaden selection, never narrow it. */
export type Owner = 'public' | 'connect' | 'recipient'
const browserAssets = new Set([
  'hosted/source-pond-v4.png','hosted/agent-pond-v4.png','hosted/agent-reading-v4.png',
  'fonts/HostGrotesk-Variable.ttf','brand/apparent-wordmark.svg',
].map(path => `public/site/public/assets/${path}`))
const publicExternal = new Set(['README.md','capture/agent/skills/apparent/SKILL.md','enablement/release_gate.py'])
const nativeWorkflows = new Set([
  'verification.yml','verification-mac.yml','verification-platform.yml','release-linux.yml',
  'release-mac-preview.yml','release-windows.yml','native-proof-mac.yml',
].map(name => `.github/workflows/${name}`))
export function relevant(owner: Owner, path: string): boolean {
  if (!path || path.startsWith('/') || path.includes('\\') || path.split('/').some(part => part==='..'||part==='.'||part==='')) return true
  if (path==='enablement/verification/workflow-budget.ts' || path.startsWith('.github/actions/runtime-budget/') ||
      path==='enablement/verification/web/inputs.ts' || path==='enablement/verification/web/baseline.ts' ||
      path==='.github/workflows/connect-delivery.yml') return true
  if (publicExternal.has(path)) return owner==='public' || (owner==='connect' && path==='enablement/release_gate.py')
  if (path.startsWith('public/site/')) return owner==='public' || (owner==='connect' && browserAssets.has(path))
  if (path.startsWith('connect/web/') || path.startsWith('connect/service/')) return owner==='connect' || (owner==='public' && path.startsWith('connect/web/'))
  if (path.startsWith('connect/client/')) return owner==='recipient'
  if (path.startsWith('connect/delivery/recipient/')) return owner==='recipient'
  if (path.startsWith('connect/delivery/')) return owner!=='recipient'
  if (path.startsWith('shared/ui/') || path.startsWith('shared/analytics/') || path.startsWith('shared/assets/')) return owner!=='recipient'
  if (path==='.github/workflows/web-delivery.yml' || path.startsWith('enablement/verification/web/')) return owner==='public'
  // These current owners are not imported by the independent delivery entrypoints.
  // The generated Public skill exception is handled before the native exclusion.
  if (path.startsWith('capture/') || path.startsWith('composition/') || path.startsWith('connect/docs/') ||
      path.startsWith('connect/hosted/') || path==='connect/hosted.rs' || path.startsWith('.cargo/') ||
      path.startsWith('.githooks/') || nativeWorkflows.has(path) ||
      ['Cargo.toml','Cargo.lock','build.rs','main.rs','AGENTS.md','CONTRIBUTING.md','LICENSE','.gitignore','enablement/verification.py'].includes(path) ||
      path.startsWith('enablement/docs/') || path.startsWith('enablement/tests/') || path.startsWith('enablement/native/') ||
      (path.startsWith('shared/') && path.endsWith('.rs'))) return false
  return true
}
export function selectInputs(paths:string[],known:boolean,force=false,owner:Owner='public') {
  const changed=paths.some(path=>relevant(owner,path))
  return {required:force||!known||changed,
    reason:force?'Explicit owner refresh':!known?'No actual successful owner baseline':changed?'Owner, shared or uncertain inputs changed':'No owner consumer inputs changed'}
}
