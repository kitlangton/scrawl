// Run in the page's main world: isolated preload listeners miss page rejections.
declare global {
  interface Window {
    apparentExceptions?: { exception(): void; rejection(): void };
  }
}
function report(kind: 'exception' | 'rejection') {
  try { window.apparentExceptions?.[kind](); } catch { /* Never report the reporter. */ }
}
window.addEventListener('error', () => report('exception'));
window.addEventListener('unhandledrejection', () => report('rejection'));
export {};
