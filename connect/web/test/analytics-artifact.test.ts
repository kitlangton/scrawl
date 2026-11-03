import assert from 'node:assert/strict'
import {readFile,readdir} from 'node:fs/promises'
import {resolve} from 'node:path'
import test from 'node:test'
const root=resolve(process.env.CONNECT_ARTIFACT||'dist')
test('production JavaScript never embeds the synthetic analytics fixture key',async()=>{
 for(const name of await readdir(`${root}/assets`))if(name.endsWith('.js'))assert.doesNotMatch(await readFile(`${root}/assets/${name}`,'utf8'),/phc_synthetic/)
})
