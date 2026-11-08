# Connect web ownership

This package owns browser account, auth entry, device consent and their presentation.
Only edit `connect/web/**` for the extraction slice. Do not change the running app,
legacy site, API service, deployment composition or native capture code.

- This is named **migration staging**: legacy `public/site/src/hosted` is still live.
  Keep copies behavior-identical until integration tests the composed artifact,
  switches routing, and removes legacy implementation in one reversible change.
- Browser/backend contract: `../../connect/docs/hosted-api.md`; selected visuals:
  `../../public/site/docs/hosted-design.md`. No redesign or shared marketing CSS.
- Run `bun install --frozen-lockfile`, then `npm test`. `bun run build` also works.
  Use maintained dependencies and the generated Bun lock, not a fabricated npm lock.
- `dist` is production route-only output. `dist-test` uses an explicitly synthetic
  loopback API. Never deploy it. Keep test receipts/screenshots outside git via
  `CONNECT_EVIDENCE=$TMPDIR/<unique-name>`.
- Never fetch account data during SSR, store credentials in browser storage,
  serialize CSRF into HTML/props, or log API bodies. Explicit separate consent,
  credentialed fetch, CSRF and mutation readback must remain intact.
- Build copies only five approved canonical assets from `../../public/site/public/assets`.
  Runtime/SSR must never import site implementation. Do not add a root page,
  sitemap or discovery link. Integration owns the origin router and 404 behavior.
- Run the artifact and browser checks after every behavior change. Preserve
  production `dist` when testing. No deployment or live OAuth claim from fixtures.