/* Authorized default-on capability, additive after immutable analytics 0006.
   No account is enrolled, no consent/tombstone or spending state is changed.
   Applied once by the migration ledger, never at every Worker startup/deploy.
   An operator can still disable analytics_control.enabled afterward. */
UPDATE analytics_control SET enabled=1 WHERE id=1;
