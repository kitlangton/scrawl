import assert from 'node:assert/strict'
import test from 'node:test'
import { candidate } from './candidate.ts'

const sha = 'a'.repeat(40)
const valid = { GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_REF: 'refs/heads/main',
  GITHUB_EVENT_NAME: 'push', GITHUB_SHA: sha,
  GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-public.yml@refs/heads/main' }

test('main Public candidate admission needs no native workflow or network receipt', () => {
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

test('forks, feature branches, wrong workflows, and malformed identities cannot publish', () => {
  for (const [key, value] of Object.entries({ GITHUB_REPOSITORY: 'fork/apparent',
    GITHUB_REF: 'refs/heads/feature', GITHUB_EVENT_NAME: 'schedule', GITHUB_SHA: 'main',
    GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/web-delivery.yml@refs/heads/main' })) {
    assert.throws(() => candidate({ ...valid, [key]: value }, () => sha), key)
  }
  assert.throws(() => candidate(valid, () => 'b'.repeat(40)), /checkout/)
  assert.throws(() => candidate(valid, (...args) => {
    if (args[0] === 'merge-base') throw new Error('not an ancestor')
    return sha
  }), /not an ancestor/)
})

test('other stream completion events cannot trigger Public delivery', () => {
  assert.throws(() => candidate({ ...valid, GITHUB_EVENT_NAME: 'workflow_run' }, () => sha), /canonical main workflow/)
})
