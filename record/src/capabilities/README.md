# Record capabilities

Import `RecordClient`, `CapabilitySnapshot`, `RecordCommand` and `CommandResult` from `@apparent/record/capabilities`. Inject a host adapter into the consumer; subscribe with `observe` for a complete immediate snapshot and complete replacements, then call `execute` with a unique request ID and the observed provider epoch/revision. Unsubscribe and `detach()` release observation only and never stop capture. Refusals and `outcome-unknown` are not evidence of success; re-observe/reconcile authoritative facts.

`@apparent/record/fixtures` exports `createRecordFixture`, `fixtureScenarios` and their fixture types. They are explicitly synthetic test tools for injected-client consumer examples. They have no native, filesystem or Electron access and are not a production backend or application fallback.

The contract describes desired host capability only. Production readiness, permission/shortcut/pen writes, capture execution, local-only delivery enforcement, clipboard and open dispatch require verified native host adapters. Record stores factual capture identities and outcomes only. Associations, progress, milestones and paste confirmation belong to Onboarding.
