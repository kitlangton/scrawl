# TypeScript architecture guard

Run `npm ci`, then `npm run check`, `npm run typecheck`, and `npm test` from this directory. The checker scans tracked and non-ignored untracked `.ts`, `.tsx`, `.cts`, and `.mts` files and inspects product/shared/composition imports plus nested repo-owned package manifests. Product consumers use declared package exports; owner-local paths remain available to local tests and implementation code. Alias mappings come only from the nearest effective `tsconfig.json`, including inherited settings. Local package dependencies must target a matching package root in an allowed owner.

The guard checks statically visible TypeScript dependencies. It cannot prove runtime isolation for generated code, data-driven loaders, native code, or dependencies hidden behind arbitrary runtime behavior. Computed `import()` and `require()` in product owners are rejected because their destinations cannot be bounded from syntax. Composition loaders may remain dynamic because composition is the assembly boundary; their runtime targets are outside this static proof.

CI checks each stream's owner with `npm run check -- --owner record` (or `engine`, `agent`, `onboarding`, `connect`, `shared`, `composition`). Shared/composition checks and the checker regression tests run in `Delivery contracts`; each component job checks its own owner before artifact admission. An unrelated product owner's violation therefore fails that owner's lane. Omitting `--owner` scans all supported owners for local integration checks. The `connect` owner currently covers `connect/delivery/client`, `connect/tooling/hosting` and the consumed `connect/service` implementation; Connect web code and Public retain their own checks. This is a bounded client checkpoint, not whole-Connect certification.

Agent may consume declared public `@apparent/record` package exports for headless lifecycle work. This dependency does not allow private Record exports, cross-owner source paths, or a reverse Record-to-Agent edge.

Agent may also consume `@apparent/native-host` root and `./clipboard` exports and declare that canonical package dependency. This bounded exception does not permit other composition packages, NativeHost pen/private exports, relative source paths or aliases. NativeHost cannot depend back on Agent; ordinary shell assembly retains its public Agent dependency.

Owners can declare packed-package test loaders in `architecture.json` beside their package. No Platform edit or approval is required. Rules apply only to the named function in a `test/` file, using a constant environment-root plus `node_modules/<package>/<declared public export target>` and `pathToFileURL(...).href`. Private targets, other files/functions and other root expressions remain rejected. The declared package still follows normal dependency directions and export checks. For example:

```json
{"schema":1,"fixtureImports":[{"file":"test/consumer.test.ts","function":"packedClient","env":"APPARENT_CLIENT_CONSUMER","package":"@apparent/record","exports":[".","./fixtures"]}]}
```
