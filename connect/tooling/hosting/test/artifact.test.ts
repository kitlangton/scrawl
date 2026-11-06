import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, rm, symlink } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { validateOutput } from '../scripts/validate-output.ts'

const names = ['worker.js', 'wrangler.json', 'dist/connect/index.html', 'dist/connect/device/index.html', 'dist/connect/shares/index.html', ...['HostGrotesk-Variable.ttf','apparent-wordmark.svg','source-pond-v4.png','agent-pond-v4.png','agent-reading-v4.png','index-abc.js','index-abc.css'].map(n=>`dist/connect/assets/${n}`)]
test('deployment inventory rejects runtime SQLite/WAL residue, unknown files, links and missing bytes', async()=>{
 const root=await mkdtemp(join(tmpdir(),'connect-output-check-'))
 try {
  for(const name of names){await mkdir(dirname(join(root,name)),{recursive:true});await writeFile(join(root,name),'synthetic')}
  await validateOutput(root)
  await mkdir(join(root,'.wrangler/state/v3'),{recursive:true});await writeFile(join(root,'.wrangler/state/v3/trace.sqlite-wal'),'synthetic test residue')
  await assert.rejects(validateOutput(root),/Unexpected artifact directory: .wrangler/)
  await rm(join(root,'.wrangler'),{recursive:true})
  await writeFile(join(root,'debug.log'),'synthetic');await assert.rejects(validateOutput(root),/Unexpected artifact entry: debug.log/);await rm(join(root,'debug.log'))
  await rm(join(root,'worker.js'));await symlink('wrangler.json',join(root,'worker.js'));await assert.rejects(validateOutput(root),/Unexpected artifact link/);await rm(join(root,'worker.js'))
  await assert.rejects(validateOutput(root),/Missing artifact entry: worker.js/)
 } finally {await rm(root,{recursive:true,force:true})}
})
