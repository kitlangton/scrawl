import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import test from 'node:test'

test('Public maintained sources and owner commands retain the TypeScript boundary', () => {
  const site = resolve(import.meta.dirname, '../..')
  const ignored = new Set(['node_modules', 'dist', 'dist-ssr', 'dist-apparent', '__pycache__', '.git', '.build'])
  const files = (root: string): string[] => readdirSync(root, { withFileTypes: true }).flatMap(entry => ignored.has(entry.name) ? [] : entry.isDirectory() ? files(join(root, entry.name)) : [join(root, entry.name)])
  const owned = files(site)
  assert.deepEqual(owned.filter(path => /\.(rs|py|js|mjs|cjs|sh)$/.test(path) || /(?:^|\/)Cargo\.(?:toml|lock)$/.test(path)), [], 'Public must not regain native builds or legacy authored tooling')
  const scripts = JSON.parse(readFileSync(join(site, 'package.json'), 'utf8')).scripts as Record<string, string>
  for (const [name, command] of Object.entries(scripts)) assert.doesNotMatch(command, /\b(?:cargo|rustc|rustup|python3?|uv)\b/, name)
})
