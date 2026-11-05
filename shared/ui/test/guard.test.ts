import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import test from 'node:test'
import { colorsIn, inventory, violations } from '../scripts/check-theme.ts'

test('owner checks retain shared surfaces and global coverage', () => {
  const root = mkdtempSync(join(tmpdir(), 'theme-'))
  try {
    const paths = ['public/site/src/home.css', 'connect/web/src/app.css', 'connect/service/src/recipient/view.css', 'shared/ui/src/extra.css', 'capture/desktop/ui/frontend/src/app.css', 'connect/delivery/recipient/tool.ts']
    for (const path of paths) { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), 'color: #fff;') }
    assert.deepEqual(new Set(Object.keys(inventory(root))), new Set(paths))
    assert.deepEqual(new Set(Object.keys(inventory(root, 'public'))), new Set([paths[0], paths[3]]))
    assert.deepEqual(new Set(Object.keys(inventory(root, 'connect'))), new Set([paths[1], paths[2], paths[3]]))
    assert.equal(violations(inventory(root, 'public'), {}).length, 2)
    assert.equal(violations(inventory(root, 'connect'), {}).length, 3)
    assert.throws(() => inventory(root, 'unknown'), /Unknown theme owner/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
test('reject new fixed colors and growth while preserving baseline allowance', () => {
  const baseline = { 'public/old.css': { '#fff': 1 } }
  assert.deepEqual(violations(baseline, baseline), [])
  assert.equal(violations({ 'public/new.css': { '#fff': 1 }, 'public/old.css': { '#fff': 2 } }, baseline).length, 2)
})
test('detect CSS and Tailwind bypasses while allowing semantic tokens', () => {
  for (const value of ['color:#fff;', 'background:rgb(0 0 0 / .5)', 'bg-white', 'text-zinc-950', 'dark:bg-black', 'color-scheme: light;', 'background:white;color:black', 'backgroundColor: "red"']) assert.ok(Object.keys(colorsIn(value)).length, value)
  for (const value of ['bg-background', 'text-foreground', 'var(--color-surface)', 'color:currentColor']) assert.equal(Object.keys(colorsIn(value)).length, 0, value)
})
