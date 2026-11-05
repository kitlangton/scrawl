// Verify an extracted raw Public component after Pages metadata preparation.
import { resolve, join } from 'node:path'
import { createHash } from 'node:crypto'
import { inventory, validatePreservedFiles, validatePublic } from './public.ts'

const site = resolve(import.meta.dirname, '../..')
const prepared = resolve(process.env.SITE_ARTIFACT || join(site, 'dist-apparent'))
const rawInput = resolve(process.env.PUBLIC_ARTIFACT_INPUT || join(site, 'dist-apparent'))
const raw = inventory(rawInput), files = inventory(prepared), canonical = inventory(join(site, 'public'))
validatePublic(raw, canonical)
validatePublic(files, canonical, { prepared: true })
validatePreservedFiles(files, raw)
const landing = createHash('sha256').update(raw.get('index.html') || Buffer.alloc(0)).digest('hex')
console.log(JSON.stringify({ artifact: prepared, rawInput, files: files.size, preservedPublicFiles: raw.size, homepageSha256: landing, limits: 'Public artifact graph and prepared metadata only; no live Pages or native proof.' }))
