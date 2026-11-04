import { execFileSync } from 'node:child_process'
import { pathToFileURL } from 'node:url'

const LIMIT = 2000
const excludedComponents = new Set(['.git', 'target', 'node_modules', 'vendor', 'third_party', 'third-party', '__pycache__'])
const generatedRoots = ['public/site/video-demo/output/', 'connect/web/dist/', 'connect/web/dist-site-check/']
const lockfiles = new Set(['package-lock.json', 'npm-shrinkwrap.json', 'pnpm-lock.yaml', 'bun.lockb'])

function excluded(path: string): boolean {
  const parts = path.split('/')
  const name = parts.at(-1)!
  return parts.some(part => excludedComponents.has(part)) ||
    generatedRoots.some(root => path.startsWith(root)) ||
    name.endsWith('.lock') || lockfiles.has(name)
}

export function check(root: string): string[] {
  const git = (...args: string[]): Buffer => execFileSync('git', args, { cwd: root, maxBuffer: 128 * 1024 * 1024 })
  if (git('ls-files', '--unmerged', '-z').length) throw new Error('Resolve unmerged index entries first')
  // Raw diff supplies staged blob IDs. Disable rename detection so destinations
  // are checked as additions; no worktree reads or shell path interpolation.
  const records = git('diff', '--cached', '--raw', '--no-abbrev', '--no-renames', '--diff-filter=ACMT', '-z', '--').toString('utf8').split('\0')
  const violations: string[] = []
  for (let i = 0; i < records.length - 1; i += 2) {
    const metadata = records[i].split(' ')
    const path = records[i + 1]
    const mode = metadata[1]
    if (!['100644', '100755'].includes(mode) || excluded(path)) continue
    const content = git('cat-file', 'blob', metadata[3])
    // Match Git's usual NUL-based binary heuristic.
    if (content.subarray(0, 8000).includes(0)) continue
    let lines = 0
    for (const byte of content) if (byte === 10) lines++
    if (content.length && content.at(-1) !== 10) lines++
    if (lines > LIMIT) violations.push(`${JSON.stringify(path)}: ${lines} lines (limit ${LIMIT})`)
  }
  return violations
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const violations = check(process.cwd())
    if (violations.length) {
      console.error(`pre-commit: staged files exceed the 2000-line limit:\n  ${violations.join('\n  ')}\nSplit files by responsibility and stage the result; this hook does not edit or stage files.`)
      process.exitCode = 1
    }
  } catch (error) {
    console.error('pre-commit: file-length check failed closed:', error)
    process.exitCode = 1
  }
}
