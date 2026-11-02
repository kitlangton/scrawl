// Shared palette + color-only compatibility for the accepted public snapshot.
import { readFileSync } from 'node:fs'
export function appearanceCss() {
  return readFileSync(new URL('../../../../shared/ui/src/tokens.css', import.meta.url), 'utf8') + '\n' +
    readFileSync(new URL('../../src/styles/system-appearance.css', import.meta.url), 'utf8')
}
export function withAppearance(html, css = appearanceCss()) {
  if (!html.includes('</head>') || html.includes('data-apparent-system-theme')) throw Error('Expected unmodified public document')
  return html.replace('</head>', `<style data-apparent-system-theme>${css.replaceAll('</style', '<\\/style')}</style></head>`)
}
