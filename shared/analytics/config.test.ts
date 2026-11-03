import test from 'node:test'
import assert from 'node:assert/strict'
import {browserAnalyticsDefinitions} from './config.ts'
const config=(env:Record<string,string|undefined>)=>JSON.parse(browserAnalyticsDefinitions(env).__APPARENT_ANALYTICS_CONFIG__)
test('browser capability defaults on for a public project key, EU and production; no choice is fabricated',()=>{
 assert.deepEqual(config({PUBLIC_POSTHOG_TOKEN:'phc_synthetic'}),{enabled:true,token:'phc_synthetic',host:'https://eu.i.posthog.com',environment:'production'})
 assert.equal(browserAnalyticsDefinitions({PUBLIC_POSTHOG_TOKEN:'phc_synthetic'}).__APPARENT_ANALYTICS_ENABLED__,'true')
})
test('missing/invalid keys, explicit stop and malformed configuration fail closed without exposing credentials',()=>{
 for(const env of [{},{PUBLIC_POSTHOG_TOKEN:'phx_private'},{PUBLIC_POSTHOG_TOKEN:'phc_synthetic',PUBLIC_POSTHOG_ENABLED:'false'},{PUBLIC_POSTHOG_TOKEN:'phc_synthetic',PUBLIC_POSTHOG_ENABLED:'bad'},{PUBLIC_POSTHOG_TOKEN:'phc_synthetic',PUBLIC_POSTHOG_HOST:'https://other.example/?private=secret'},{PUBLIC_POSTHOG_TOKEN:'phc_synthetic',PUBLIC_POSTHOG_ENVIRONMENT:'bad'}]){
  assert.equal(config(env).enabled,false)
  assert.doesNotMatch(JSON.stringify(browserAnalyticsDefinitions(env)),/phx_private|other\.example|private=secret/)
 }
})
