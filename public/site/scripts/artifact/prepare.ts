// Public-only Pages metadata; Connect keeps its independently served routes.
import { writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { inventory, validatePreservedFiles, validatePublic, type FileInventory } from './public.ts'

const site = resolve(import.meta.dirname, '../..')
export function preparePublic(output = join(site, 'dist-apparent'), canonical: FileInventory = inventory(join(site, 'public'))) {
  const raw = inventory(output)
  validatePublic(raw, canonical)
  writeFileSync(join(output, '404.html'), '<!doctype html><html lang="en"><meta name="robots" content="noindex"><title>Not found</title><body>Not found</body></html>\n')
  writeFileSync(join(output, '_redirects'), '')
  writeFileSync(join(output, '_headers'), '/*\n  Cache-Control: no-cache, no-transform\n  X-Apparent-Publisher: cloudflare-pages\n')
  const prepared = inventory(output)
  validatePublic(prepared, canonical, { prepared: true })
  validatePreservedFiles(prepared, raw)
  console.log(`Pages Public package: preserved ${raw.size} files; unknown paths denied`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) preparePublic()
