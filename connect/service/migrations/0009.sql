/* Aggregate-only statistics. Apply after the short prospective notice and normal
   release admission; application time is the actual collection boundary.
   No historical activity is scanned; no individual consent/revision/budget changes. */
/* Preserve the aggregate objection independently of later individual opt-in.
   Existing revisions/times and individual consent are not rewritten. */
ALTER TABLE analytics_choices ADD COLUMN aggregate_objected INTEGER NOT NULL DEFAULT 0 CHECK(aggregate_objected IN (0,1));
UPDATE analytics_choices SET aggregate_objected=1 WHERE NOT EXISTS(SELECT 1 FROM analytics_consent WHERE account_id=analytics_choices.account_id);
CREATE TABLE aggregate_dau_state (
 id INTEGER PRIMARY KEY CHECK(id=1),
 collection_started_at_ms INTEGER NOT NULL,
 as_of_ms INTEGER NOT NULL
);
INSERT INTO aggregate_dau_state VALUES(1,CAST(unixepoch('subsec')*1000 AS INTEGER),CAST(unixepoch('subsec')*1000 AS INTEGER));
CREATE TABLE aggregate_dau_days (
 day_utc TEXT PRIMARY KEY,
 active_accounts INTEGER NOT NULL CHECK(typeof(active_accounts)='integer' AND active_accounts>=0)
);
/* Real account IDs are pseudonymous personal data, internal to D1. Logical
   expiry is UTC day end + one hour: at most 25h. Cron removes expired rows.
   No individual event IDs, paths, timestamps or provider identifiers are kept. */
CREATE TABLE aggregate_dau_dedup (
 day_utc TEXT NOT NULL, account_id TEXT NOT NULL,
 expires_at_ms INTEGER NOT NULL,
 PRIMARY KEY(day_utc,account_id)
);
CREATE INDEX aggregate_dau_expiry ON aggregate_dau_dedup(expires_at_ms);
CREATE TABLE aggregate_dau_exclusions (
 account_id TEXT PRIMARY KEY CHECK(length(account_id)=64 AND account_id NOT GLOB '*[^a-f0-9]*'),
 expires_at_ms INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec')*1000 AS INTEGER)+24*60*60*1000)
);
CREATE TRIGGER aggregate_dau_increment AFTER INSERT ON aggregate_dau_dedup BEGIN
 INSERT INTO aggregate_dau_days VALUES(NEW.day_utc,1)
 ON CONFLICT(day_utc) DO UPDATE SET active_accounts=active_accounts+1;
END;
/* Objection/account erasure within the attribution window removes its count.
   TTL expiry forgets attribution WITHOUT erasing the historical total. */
CREATE TRIGGER aggregate_dau_remove AFTER DELETE ON aggregate_dau_dedup
 WHEN OLD.expires_at_ms>CAST(unixepoch('subsec')*1000 AS INTEGER)
 OR NOT EXISTS(SELECT 1 FROM accounts WHERE id=OLD.account_id)
 OR EXISTS(SELECT 1 FROM aggregate_dau_exclusions WHERE account_id=OLD.account_id)
 OR EXISTS(SELECT 1 FROM analytics_choices WHERE account_id=OLD.account_id AND aggregate_objected=1)
 OR (EXISTS(SELECT 1 FROM analytics_choices WHERE account_id=OLD.account_id)
 AND NOT EXISTS(SELECT 1 FROM analytics_consent WHERE account_id=OLD.account_id)) BEGIN
 UPDATE aggregate_dau_days SET active_accounts=MAX(0,active_accounts-1) WHERE day_utc=OLD.day_utc;
END;

/* ONLY this aggregate view is eligible for connector allowlisting. Zero/missing
   admissions cannot establish a true zero under fail-open collection. */
CREATE VIEW reporting_daily_active_accounts AS
 SELECT d.day_utc,d.active_accounts,s.as_of_ms,s.collection_started_at_ms,
 'observed_lower_bound' AS coverage
 FROM aggregate_dau_days d CROSS JOIN aggregate_dau_state s
 WHERE s.id=1 AND d.active_accounts>0
 AND d.day_utc>=date(s.collection_started_at_ms/1000,'unixepoch')
 AND d.day_utc>=date('now','-29 days');
