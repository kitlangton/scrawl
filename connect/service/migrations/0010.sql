/* Native warehouse bootstrap needs an actual row even before any observed
   activity. Preserve 0009 and its original start, counters, choices and TTL.
   These fixed typed metadata fields are NOT activity and NOT feature flags. */
ALTER TABLE aggregate_dau_state ADD COLUMN reporting_unknown_accounts INTEGER
 CHECK(reporting_unknown_accounts IS NULL);
ALTER TABLE aggregate_dau_state ADD COLUMN reporting_observed_coverage TEXT NOT NULL
 DEFAULT 'observed_lower_bound' CHECK(reporting_observed_coverage='observed_lower_bound');
ALTER TABLE aggregate_dau_state ADD COLUMN reporting_unknown_coverage TEXT NOT NULL
 DEFAULT 'unknown' CHECK(reporting_unknown_coverage='unknown');

DROP VIEW reporting_daily_active_accounts;
CREATE VIEW reporting_daily_active_accounts AS
 SELECT d.day_utc,d.active_accounts,s.as_of_ms,s.collection_started_at_ms,
 s.reporting_observed_coverage AS coverage
 FROM aggregate_dau_days d CROSS JOIN aggregate_dau_state s
 WHERE s.id=1 AND d.active_accounts>0
 AND d.day_utc>=date(s.collection_started_at_ms/1000,'unixepoch')
 AND d.day_utc>=date('now','-29 days')
 UNION ALL
 SELECT CAST(date(unixepoch('subsec'),'unixepoch') AS TEXT),
 s.reporting_unknown_accounts,s.as_of_ms,s.collection_started_at_ms,
 s.reporting_unknown_coverage
 FROM aggregate_dau_state s
 WHERE s.id=1
 AND date(unixepoch('subsec'),'unixepoch')>=date(s.collection_started_at_ms/1000,'unixepoch')
 AND NOT EXISTS(SELECT 1 FROM aggregate_dau_days d
  WHERE d.day_utc=date(unixepoch('subsec'),'unixepoch') AND d.active_accounts>0);
