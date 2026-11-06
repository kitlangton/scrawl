/** Bind conservative change selection to the existing component registrations. */
import { appendFileSync, readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import { makePlan } from '../core.ts'
import { selectComponents, selectFromEvent } from './selector.ts'

export function platformPlan(root: string, selected: string[]) {
  const plan = makePlan(root)
  for (const id of selected) {
    if (!plan.matrix.include.some(row => row.id === id)) throw new Error(`Selected platform component is unavailable: ${id}`)
  }
  const include = plan.matrix.include.filter(row => selected.includes(row.id))
  return { matrix: { include }, has_ready: include.length > 0 }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  let selected: string[]
  try {
    const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH ?? '', 'utf8'))
    selected = selectFromEvent(process.cwd(), process.env.GITHUB_EVENT_NAME ?? '', event)
  } catch {
    selected = selectComponents(null)
  }
  const plan = platformPlan(process.cwd(), selected)
  if (!process.env.GITHUB_OUTPUT) throw new Error('GITHUB_OUTPUT is required')
  appendFileSync(process.env.GITHUB_OUTPUT, `matrix=${JSON.stringify(plan.matrix)}\nhas_ready=${plan.has_ready}\n`)
  console.log(JSON.stringify({ selected, ...plan }))
}
