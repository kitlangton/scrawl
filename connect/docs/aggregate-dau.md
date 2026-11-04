# Aggregate Connect daily activity

This is the bounded implementation of server-side daily totals, **not** default-on
account-linked PostHog events. Existing individual PostHog consent, choices, UUIDs,
outbox and delivery remain unchanged. Simon authorized this minimal feature for a
small project used by friends. This is not a declaration of GDPR exemption or
anonymity. The earlier blanket external-review/signature hold is withdrawn for
this aggregate feature, not for default-on individual PostHog tracking. Normal
source review, tests, release admission, notice and isolated acceptance still apply.
Applying 0009 establishes the prospective counting boundary; no hidden experiment
flag or historical backfill is introduced.

## Producer contract

Only `reporting_daily_active_accounts` may be synced by the existing D1 connector:

| Column | Value |
|---|---|
| `day_utc` | TEXT `YYYY-MM-DD`, original successful server-action UTC day, or current UTC day for status metadata |
| `active_accounts` | nullable INTEGER; positive **observed distinct-account lower bound**, or NULL for unknown status; never a fabricated zero |
| `as_of_ms` | INTEGER, coarse global successful maintenance watermark, not an individual's action time |
| `collection_started_at_ms` | INTEGER, immutable actual 0009 application boundary |
| `coverage` | TEXT, `observed_lower_bound` for activity or `unknown` for NULL status |

**This coverage qualification is necessary, not cosmetic.** Exact atomicity with
primary writes would let metric failures break uploads/shares. Instead the actual
successful transition schedules a separate caught D1 write with `waitUntil`.
Crashes, missing/failed schema, queue/cap/worker failure may lose admissions without
a reliable missed-event count. Even a healthy heartbeat cannot prove otherwise.
The implementation never certifies complete coverage and never exports a zero:
missing days, failed reads and zero observed admissions are **unknown**, not true
zero. Consumers must not zero-fill these gaps. First/current days are also partial;
identify the first from `collection_started_at_ms`, the current from UTC today.
The accepted metric is exact distinct counting of observed eligible admissions,
not a guaranteed census of all successful service actions.

Additive migration `0010.sql` resolves an empty native-source bootstrap: when the
current UTC day has no positive observed count, the same view includes one status
row with `active_accounts=NULL` and `coverage='unknown'`. It is metadata, not an
activity, zero, customer fixture or historical backfill. A positive current-day
count replaces that day's status row; historical missing days stay absent. All
five columns have stable SQLite declared types TEXT/INTEGER/INTEGER/INTEGER/TEXT,
including the nullable count and coverage in both compound-view branches. Fixed
typed fields live on the existing singleton state; no new writer, job or flag.
Original collection start, watermark, counters, dedup and choices are untouched.
The consumer must retain `coverage='observed_lower_bound' AND active_accounts>0`
for its sparse count chart. A real status row makes the source nonempty but only
actual native first-sync readback proves that PostHog materialized its table.

Successful `pending`→`ready` upload completion after object verification and first
durable share insertion qualify. SQL `RETURNING` identifies the actual winning
mutation; repeat requests do not re-admit an older action on a later day. The
completion time is produced inside that SQL statement; share creation uses its
original server-created `created_at`. A pending share counts creation, not watch
readiness. No signup, login, device approval, storage/background snapshot, local-only
recording, historical census or event backfill qualifies. One atomic metric-local
INSERT/trigger increments only for the first account/day; concurrent upload/share
admissions deduplicate together. None of these metric triggers attaches to product,
auth or existing consent tables, so a metric failure cannot roll back those writes.

No identity, pseudonym, event UUID, capture/share ID, individual event timestamp,
email, IP, path, recording or arbitrary properties appears in the exported view.
`as_of_ms` advances on global scheduled maintenance, not individual activity.
All four new internal tables and other raw tables must remain disabled for PostHog;
keep auto-sync-new false and direct query false. The reporting owner alone changes
the existing source/view allowlist, same board/top chart and its coverage labels.
The existing two aggregate views and storage/distribution reports are untouched.

## Internal data and retention

- `aggregate_dau_dedup`: original opaque **account ID** + UTC day + common expiry
  (`day end + 1 hour`). This is honest pseudonymous personal data, not anonymization;
  no extra hashing/rotation/identity service. Maximum logical lifetime is 25 hours.
- The separate objection bit remains with the existing account preference until
  account erasure; it is not event attribution and must not expire with dedup.
- Admission rejects expired original days, future timestamps, actions before actual
  start, absent accounts, reserved `.invalid` fixture addresses, pre-registered exact
  fixture exclusions, and an existing choice/tombstone without current affirmative
  consent. An additive `analytics_choices.aggregate_objected` bit preserves an
  aggregate objection even when individual PostHog consent is later granted.
  Existing refusal revisions/times and individual consent are not rewritten.
- Existing refusals/withdrawals block future admissions in the same D1 statement.
  `DNT:1`/`Sec-GPC:1` suppress the applicable request; an unflagged retry of that same
  already-completed action does not count it later. No inference is made that an
  unobserved device/browser sent a signal. Account-wide saved objections remain the
  existing preference; no new browser tracking or identity linkage is introduced.
- The metric table cap is 10,000 dedup rows. Overflow is an unmeasured loss, not zero
  and not a product error. No new R2 operations or budget writes occur.
- Normal 15-minute scheduled maintenance deletes expired dedup rows, reconciles
  saved objections/exclusions/account erasure, and removes aggregate days before
  the rolling 30-calendar-day window. It runs independently of legacy cleanup;
  the existing cleanup→individual-export order is preserved.
- Attribution expiry does **not** decrement historical totals. While an attribution
  row still exists, objection/account deletion removes its contribution when
  maintenance next succeeds, even if cleanup was late. Once attribution is forgotten,
  this implementation cannot locate or subtract one account from a retained total.
  Counter zero after a correction is not published as a verified zero.
- Physical removal is by the next **successful** maintenance run: ordinarily within
  15 minutes after logical expiry, not a guarantee during outages. Existing D1 backup/
  Time Travel copies retain data separately: Cloudflare documents seven days on
  Workers Free and 30 days on Paid. Verify the existing plan without changing it.
  No all-copy 25-hour
  deletion promise is made. PostHog and its backups retain synced totals under its
  existing settings; removing a D1 row is not proof of remote physical deletion.
- Fixture exclusions are exact account IDs, registered **before** any test action,
  plus the reserved `.invalid` address safeguard. Unexpired exclusions survive before
  the synthetic account exists; expired exclusions for absent accounts are removed.
  Never rely on provider-side identity exclusion to repair a polluted aggregate.

## Limited purpose, necessity and safeguards

Controller/contact: Simon van Laak, `simon@tryapparent.com`. The limited purpose is
to see whether friends use successful hosted handoff, using daily totals rather
than sending their individual actions to PostHog. The operational interest is
understanding use and reliability of this small service. Where GDPR applies, this
is a separate limited server-side statistics purpose, assessed through legitimate
purpose, necessity and balancing rather than borrowing consent for individual
events. Small-project context is not an exemption or a claim of legal certification.

Data flow: existing authenticated upload/share authorization resolves the account
inside the Worker. Only the winning service action passes its already-known opaque
ID and server time to the internal counter. D1 transiently reads the existing email
only for the `.invalid` fixture exclusion; neither that email nor the action time is
stored in dedup. D1 keeps account/day until the short attribution expiry. The
existing connector's D1 Read credential is account-scoped and can read operational
databases/tables. Only its **sync configuration** is restricted to reviewed views;
this is not credential-level or view-level access isolation. Simon chose that
existing native-access tradeoff after scope disclosure. Individual opt-in event export
remains separate. There is no new SDK, browser event, client cookie, local-storage key,
replay, cross-device link, export service, access grant or purchase.

Necessity: distinct account/day counting requires temporary equality testing; pure
request totals cannot deduplicate retries or multiple actions. Reusing the existing
opaque account ID avoids additional fingerprinting/identifiers. The 25-hour logical
window permits UTC rollover and one-hour late scheduling without counting an old
retry twice after forgetting. Longer account-level histories, action IDs, per-action
journals, content and individual provider events are unnecessary for this purpose.
Alternatives considered: existing affirmative event DAU has consent-limited coverage;
request totals measure volume, not distinct accounts; primary-transaction counting
would violate the required fail-open service behavior. None permits overriding a
refusal. The consumer labels the accepted sparse lower-bound metric accordingly.

Balancing concerns: accounts may not reasonably expect new nonconsented processing,
even on our own server; stable IDs remain personal data. A tiny beta cohort means a
daily total of one may reveal a known person's use. Exact totals and consecutive-day
patterns are **not certified anonymous** just because identity columns are absent.
No k-anonymity guarantee is made. Simon chose exact daily observed totals for this
small project, without a new cohort threshold, suppression or bucketing feature.
Mitigations implemented: bounded attribution, prospective-only start, no event history,
refusal/signal exclusions, exact fixture isolation, minimal provider view, coarse
watermark, no true-zero/full-coverage claim, retained provider protections, and an
always-available objection control. The acknowledged residual risks are small-cohort
inference, broader connector credential access and retention in provider backups.

Terminal/credential purpose audit: upload/read tokens in the existing client's private
`hosted/*.credential` files, Google/browser session cookies and device grants support
requested authentication/upload/read access. This counter neither reads those files
nor creates or refreshes a credential; it receives the account already established
for the requested service. It introduces no extra terminal storage/read request,
credential access or device fingerprint. It does not reclassify analytics as
necessary credential storage. Server-side reuse for statistics is documented above;
individual browser/account tracking retains its existing separate consent rules.

Relevant rules: [TDDDG25](https://www.gesetze-im-internet.de/ttdsg/__25.html),
[GDPR6/13/21](https://eur-lex.europa.eu/eli/reg/2016/679/ojv),
[Baden-Württemberg FAQ3.1](https://www.baden-wuerttemberg.datenschutz.de/faq-zu-cookies-und-tracking-2/),
[EDPB consent05/2020 ¶121–123](https://www.edpb.europa.eu/system/files/documents/files/file1/edpb_guidelines_202005_consent_en.pdf).
Provider backup reference: [Cloudflare Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/).

## Prospective notice and release

The candidate account notice visibly separates these service totals from individual
consent, explains the attribution window, lower-bound/inference limitations and
objection. The existing Stop action remains available even with no saved choice;
it records the existing false preference and stops individual export as before.
A no-choice account is not marked consented. Existing true/false records are not
rewritten. Individual re-consent does not cancel the separate aggregate objection;
the candidate has no automatic objection-reset control. The expanded notice is a
part of the reviewed implementation, not a new consent flow.

Publish and read back the short account notice **before** applying 0009. Record its
actual publication/readback and the migration's actual SQL start separately. Do not
claim that every prior user opened the notice or infer a historical counting window.
Normal verification, release admission and bounded backup/runtime checks apply;
there is no undefined external-lawyer or controller-signature release stage.

Release: independent candidate review and focused/full affected checks; exact
current-main Linux release admission; backup and checked additive schema application;
verified source deployment; pre-register exact synthetic exclusions before any trial;
read back aggregation/TTL/refusal/failure behavior; only the consumer owner enables
this exact view and verifies first sync. No dashboard/source credential changes by
this producer. Keep the existing opt-in stream until the consumer accepts replacement
coverage. Existing provider-erasure work is separate and must not be resubmitted.

Rollback: leave the original individual collector/choices and existing views intact;
stop the new queue calls by deploying the prior compatible Worker, then expire and
remove only the new metric state under an explicit reviewed cleanup. Never restore
old usage/budget counters or pretend an already-synced total was erased remotely.
