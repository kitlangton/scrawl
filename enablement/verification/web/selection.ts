/** Select the entire delta since successful web verification, including deletions. */
import { appendFileSync, readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import {relevant as ownerRelevant} from './inputs.ts'
import {selection} from './baseline.ts'
export function relevant(path:string){return ownerRelevant('public',path)}
export {selectInputs as select} from './inputs.ts'
export function gate(required: string, result: string) {
  if (!['true', 'false'].includes(required)) throw Error('Missing web selection')
  if (required === 'true' ? result !== 'success' : result !== 'skipped') throw Error(`Unexpected web verification result: ${required}/${result}`)
  return required === 'true' ? 'verified' : 'not-applicable'
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv[2] === 'gate') console.log(gate(process.env.WEB_REQUIRED ?? '', process.env.WEB_RESULT ?? ''))
  else {
    const force = process.env.GITHUB_EVENT_NAME === 'workflow_dispatch' || (process.env.GITHUB_EVENT_NAME === 'workflow_run' && JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH!, 'utf8')).workflow_run?.event === 'workflow_dispatch')
    const result = await selection('public', process.env.GITHUB_SHA ?? '', process.env.GH_TOKEN ?? '', force)
    if (!process.env.GITHUB_OUTPUT) throw Error('Missing selection output')
    appendFileSync(process.env.GITHUB_OUTPUT, `required=${result.required}\n`)
    console.log(JSON.stringify(result))
  }
}
