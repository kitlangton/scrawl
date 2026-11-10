import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { registerRelease } from './posthog-release.ts'

const sha = 'a'.repeat(40)
const id = '019aaa00-1234-7000-8000-123456789abc'
const credential = 'private-test-canary'

test('registers the packaged version against the checked-out commit and intended EU project', () => {
  for (const [platform, env, version] of [
    ['macos', { APPARENT_PREVIEW_VERSION: '0.12.2' }, '0.12.2'],
    ['linux', { TAG: 'v1.2.3' }, 'v1.2.3'],
    ['linux', {}, `linux-main-v2-${sha}`],
  ] as const) {
    let calls = 0
    const result = registerRelease(platform, { ...env, POSTHOG_CLI_API_KEY: credential, GITHUB_SHA: 'b'.repeat(40), POSTHOG_CLI_DRY_RUN: 'true' }, sha, (args, passed) => {
      calls++
      assert.deepEqual(args, ['--yes', '@posthog/cli@0.19.0', '--host', 'https://eu.posthog.com', 'release', 'resolve', '--release-name', `apparent-desktop-${platform}`, '--release-version', version])
      assert.equal(passed.GITHUB_SHA, sha)
      assert.equal(passed.POSTHOG_CLI_PROJECT_ID, '329943')
      assert.equal(passed.POSTHOG_CLI_DRY_RUN, 'false')
      assert.equal(passed.POSTHOG_CLI_API_KEY, credential)
      assert.ok(!args.join(' ').includes(credential))
      return `${id}\n`
    })
    assert.equal(result, id)
    assert.equal(calls, 1)
  }
})

test('verification never registers a release, even without credentials', () => {
  assert.equal(registerRelease('linux', { VERIFY_ONLY: 'true' }, sha, () => { throw new Error('network called') }), '')
})

test('refuses missing credentials, invalid metadata and empty or malformed provider IDs', () => {
  const env = { POSTHOG_CLI_API_KEY: credential }
  const never = () => { throw new Error('must not call') }
  assert.throws(() => registerRelease('linux', {}, sha, never), /Missing POSTHOG/)
  assert.throws(() => registerRelease('linux', env, 'main', never), /full commit/)
  assert.throws(() => registerRelease('macos', env, sha, never), /Invalid release/)
  assert.throws(() => registerRelease('linux', { ...env, TAG: 'private/path' }, sha, never), /Invalid release/)
  for (const output of ['', 'unknown', `${id}\nextra`, 'private\nAPPARENT_POSTHOG_RELEASE_ID=bad']) {
    assert.throws(() => registerRelease('linux', env, sha, () => output), /no valid release ID/)
  }
})

test('both publication workflows register before compiling, passing only the registration secret to that step', () => {
  for (const [file, platform, build] of [
    ['release-linux.yml', 'linux', 'Build with PostHog error reporting'],
    ['release-mac-preview.yml', 'macos', 'Build and verify the preview app'],
  ]) {
    const text = readFileSync(new URL(`../../../../.github/workflows/${file}`, import.meta.url), 'utf8')
    const registration = text.indexOf('- name: Register PostHog desktop release')
    assert.ok(registration > 0 && registration < text.indexOf(`- name: ${build}`))
    const step = text.slice(registration, text.indexOf(`- name: ${build}`))
    assert.ok(step.includes('secrets.POSTHOG_CLI_API_KEY'))
    assert.ok(step.includes(`posthog-release.ts ${platform}`))
    assert.ok(!step.includes('APPARENT_POSTHOG_TOKEN'))
  }
})
