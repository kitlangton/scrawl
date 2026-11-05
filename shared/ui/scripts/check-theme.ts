// Ratchet legacy fixed colors; new frontend colors belong in shared tokens.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { resolve, relative, extname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { execFileSync } from 'node:child_process'

export const ROOT = fileURLToPath(new URL('../../../', import.meta.url))
const baselinePath = fileURLToPath(new URL('./legacy-colors.json', import.meta.url))
const COLOR = /#[\da-fA-F]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab|lab|lch)\([^)]*\)|\b(?:bg|text|border|ring|fill|stroke)-(?:white|black|(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3})\b|color-scheme\s*:\s*(?:light|dark)\s*[;}]/g
const NAMED = /\b(?:background(?:Color|-color)?|color|fill|stroke|borderColor|outlineColor|border-color|outline-color|accent-color)\s*:\s*['"]?(?!var\b|url\b|none\b|transparent\b|currentColor\b|inherit\b|initial\b|unset\b|revert\b)([a-zA-Z]+)\b(?=\s*(?:['";}]|!important|$))/g
export type Inventory = Record<string, Record<string, number>>
export function colorsIn(source: string): Record<string, number> {
  const counts: Record<string, number> = {}
  for (const color of [...source.matchAll(COLOR)].map(m => m[0]).concat([...source.matchAll(NAMED)].map(m => 'named:' + m[1]))) counts[color] = (counts[color] ?? 0) + 1
  return counts
}
export function inventory(root = ROOT, owner?: string): Inventory {
  if (owner !== undefined && owner !== 'public' && owner !== 'connect') throw new Error('Unknown theme owner')
  let paths: string[] = []
  let gitRoot = ''
  try { gitRoot = execFileSync('git', ['rev-parse', '--show-toplevel'], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim() } catch {}
  if (gitRoot && resolve(gitRoot) === resolve(root)) {
    paths = execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], { cwd: root, encoding: 'utf8' }).split('\0')
  } else {
    const walk = (directory: string) => {
      for (const entry of readdirSync(directory, { withFileTypes: true })) {
        if (entry.name.startsWith('dist') || entry.name.startsWith('.') || entry.name === 'node_modules') continue
        const file = join(directory, entry.name)
        if (entry.isDirectory()) walk(file)
        else paths.push(relative(root, file).split('\\').join('/'))
      }
    }
    for (const name of ['capture', 'connect', 'public', 'shared']) if (existsSync(join(root, name))) walk(join(root, name))
  }
  const result: Inventory = {}
  const surfaces = owner === 'public' ? ['public/site/', 'shared/'] : ['connect/web/', 'connect/service/', 'shared/']
  for (const path of [...new Set(paths)].sort()) {
    if (!['capture/', 'connect/', 'public/', 'shared/'].some(prefix => path.startsWith(prefix))) continue
    if (owner && !surfaces.some(prefix => path.startsWith(prefix))) continue
    const file = join(root, path)
    if (!['.css', '.tsx', '.ts', '.html', '.js', '.cjs'].includes(extname(file)) || !existsSync(file) || !statSync(file).isFile()) continue
    if (path.split('/').some(part => ['test', 'tests', 'scripts', 'node_modules'].includes(part)) || path === 'shared/ui/src/tokens.css') continue
    const colors = colorsIn(readFileSync(file, 'utf8'))
    if (Object.keys(colors).length) result[path] = colors
  }
  return result
}
export function violations(current: Inventory, baseline: Inventory): string[] {
  return Object.entries(current).flatMap(([path, colors]) => Object.entries(colors).filter(([color, count]) => count > (baseline[path]?.[color] ?? 0)).map(([color, count]) => `${path}: ${color} (${count} > ${baseline[path]?.[color] ?? 0})`))
}
export function main(args = process.argv.slice(2)) {
  if (args.length && (args.length !== 2 || args[0] !== '--owner' || !['public', 'connect'].includes(args[1]))) throw new Error('Usage: check-theme.ts [--owner public|connect]')
  const owner = args[1]
  const failures = violations(inventory(ROOT, owner), JSON.parse(readFileSync(baselinePath, 'utf8')))
  if (failures.length) { console.error('New fixed frontend colors: use shared/ui/src/tokens.css semantic tokens.\n' + failures.join('\n')); process.exitCode = 1 }
  else console.log(owner ? `Theme color contract passed for ${owner} + shared.` : 'Shared theme color contract passed.')
}
if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) main()
