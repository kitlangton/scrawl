import { execFileSync } from 'node:child_process'

export const COMPONENTS = ['platform-config', 'platform-resources', 'app-shell'] as const

const config = ['composition/config/', 'shared/config-native/', 'shared/contracts/', '.cargo/']
const resources = ['shared/resources/', '.cargo/']
const shell = ['composition/app/', 'composition/native-host/', 'shared/contracts/', 'shared/ui/']
const allPrefixes = ['composition/delivery/', 'composition/architecture/', '.github/actions/delivery-contracts/', '.github/actions/delivery-component/', '.github/actions/runtime-budget/']

function starts(path: string, prefixes: string[]) { return prefixes.some(prefix => path.startsWith(prefix)) }

export function selectComponents(paths: string[] | null): string[] {
  if (paths === null) return [...COMPONENTS]
  const selected = new Set<string>()
  for (const path of paths) {
    if (path === 'LICENSE' || path === 'enablement/verification/workflow-budget.ts' || ['.github/workflows/stream-platform.yml', '.github/workflows/stream-platform-config.yml', '.github/workflows/stream-platform-resources.yml', '.github/workflows/stream-app-shell.yml'].includes(path) || starts(path, allPrefixes)) return [...COMPONENTS]
    if (starts(path, config)) selected.add('platform-config')
    if (starts(path, resources)) selected.add('platform-resources')
    if (['enablement/native/owned-processes.ts', '.github/workflows/verification-platform.yml'].includes(path) || starts(path, shell)) selected.add('app-shell')
  }
  return COMPONENTS.filter(component => selected.has(component))
}

type Event = { pull_request?: { base?: { sha?: unknown }; head?: { sha?: unknown } }; before?: unknown; after?: unknown }
const validSha = (sha: unknown): sha is string => typeof sha === 'string' && /^[0-9a-f]{40,64}$/i.test(sha)

function git(root: string, args: string[]) {
  return execFileSync('git', args, { cwd: root, encoding: 'buffer', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 32 * 1024 * 1024 })
}

export function selectFromEvent(root: string, eventName: string, event: object): string[] {
  try {
    const payload = event as Event
    let range: string[]
    if (eventName === 'pull_request') {
      const base = payload.pull_request?.base?.sha, head = payload.pull_request?.head?.sha
      if (!validSha(base) || !validSha(head)) return selectComponents(null)
      const ancestor = git(root, ['merge-base', base, head]).toString('utf8').trim()
      range = [`${ancestor}...${head}`]
    } else if (eventName === 'push') {
      const before = payload.before, after = payload.after
      if (!validSha(before) || !validSha(after) || /^0+$/.test(before)) return selectComponents(null)
      range = [`${before}..${after}`]
    } else return selectComponents(null)
    const output = git(root, ['diff', '--name-only', '-z', '--no-renames', ...range, '--'])
    return selectComponents(output.toString('utf8').split('\0').filter(Boolean))
  } catch { return selectComponents(null) }
}

