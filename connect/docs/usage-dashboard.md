# Visitors and Connect actions

Issue #356: signing into Connect does not populate the starter dashboard's
pageview-based “Active users”. Main has no product-usage collector; the optional,
consent-gated implementation remains in draft PR #269. Neither a login nor a
zero-valued query establishes collection coverage.

From `connect/service`, run `node scripts/usage-dashboard.mjs` to print an offline
PostHog plan. It contains a new private dashboard, a mandatory coverage text tile
and four separate insight payloads. It reads no credentials, changes no provider
settings and sends no events. Validate with
`node --test test/analytics/dashboard.test.mjs`.

An operator with approved project write access can create the dashboard and
insights, then attach the notice and insight tiles using the provider's supported
API. Discover current tool schemas first; the JSON is a plan, not a bulk API
request. Do not modify the existing starter dashboard or its shared insights.
Keep the coverage notice visible. Read back private visibility, queries and all
rendered tiles before claiming the dashboard is installed. Rollback deletes only
the newly created dashboard/insights identified in that operator's receipt.

The queries require `environment = production`, deliberately excluding synthetic,
development and unclassified legacy events. The browser number counts distinct
pageview identities. The account numbers count distinct pseudonymous account
identities across each entire rolling UTC window (24 hours, seven days, 30 days),
with account creation, first explicit device connection or completed upload.
They do not sum daily counts. Duplicate delivery does not increase distinct
identity counts. Snapshots do not establish human activity. Login-only activity
is not measured by #269 and must not be claimed here.

This plan is independently usable on main, but useful production values depend
on #269's consent, migration, deployment and activation gates. Implementing this
plan does not authorize those operations or provider settings changes. Before
removing the unverified-coverage notice, verify the exact project, deployed event
sources, production marker, collection start date and opted-in coverage. Confirm
eligible actions arrive and opted-out actions do not; browser pageviews and
account events remain separate. Retain limitations for incomplete history and
consent coverage. No GUI, live ingestion or complete-user census is proved by
local plan tests.
