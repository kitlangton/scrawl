import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { candidate } from './candidate.ts'

const sha = 'a'.repeat(40)
const valid = { GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_REF: 'refs/heads/main',
  GITHUB_EVENT_NAME: 'push', GITHUB_SHA: sha,
  GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/web-delivery.yml@refs/heads/main' }

test('main web candidate admission needs no native workflow or network receipt', () => {
  for (const event of ['push', 'workflow_dispatch']) {
    const calls: string[][] = []
    assert.equal(candidate({ ...valid, GITHUB_EVENT_NAME: event }, (...args) => {
      calls.push(args)
      return args[0] === 'rev-parse' ? sha : ''
    }), sha)
    assert.deepEqual(calls, [['rev-parse', 'HEAD'],
      ['merge-base', '--is-ancestor', sha, 'refs/remotes/origin/main']])
  }
})

test('forks, feature branches, follower events and malformed identities cannot publish', () => {
  for (const [key, value] of Object.entries({ GITHUB_REPOSITORY: 'fork/apparent',
    GITHUB_REF: 'refs/heads/feature', GITHUB_EVENT_NAME: 'workflow_run', GITHUB_SHA: 'main',
    GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/other.yml@refs/heads/main' })) {
    assert.throws(() => candidate({ ...valid, [key]: value }, () => sha), key)
  }
  assert.throws(() => candidate(valid, () => 'b'.repeat(40)), /checkout/)
  assert.throws(() => candidate(valid, (...args) => {
    if (args[0] === 'merge-base') throw new Error('not an ancestor')
    return sha
  }), /not an ancestor/)
})

test('web publishers retain their own verification, artifact identity and activation gates', () => {
  const workflow = readFileSync(new URL('../../../.github/workflows/web-delivery.yml', import.meta.url), 'utf8')
  assert.match(workflow, /push:\n\s+branches: \[main\]/)
  assert.doesNotMatch(workflow, /workflow_run|Linux verification|--platform linux/)
  assert.match(workflow, /node .*enablement\/verification\/web\/candidate.ts/)
  for (const name of ['website', 'hosted']) {
    const section = workflow.split(`\n  ${name}:\n`)[1].split(/\n  [a-z-]+:\n/)[0]
    assert.match(section, /needs: \[admit, verify\]/)
    assert.match(section, /github.ref == 'refs\/heads\/main'/)
    assert.match(section, /ref: \$\{\{ needs.admit.outputs.sha \}\}/)
    assert.match(section, /vars\.[A-Z_]+_CD_ENABLED == 'true'/)
  }
  assert.match(workflow, /name: website-\$\{\{ needs.admit.outputs.sha \}\}/)
  assert.match(workflow, /cancel-in-progress: false/)
})
