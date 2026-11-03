/* Optional analytics, AFTER main's immutable auth 0004 and E2EE 0005.
   No account history is backfilled and no storage budget is changed.
   Both global control and per-account permission start disabled/empty. */
CREATE TABLE analytics_control (
 id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
 dropped INTEGER NOT NULL DEFAULT 0, snapshot_day INTEGER NOT NULL DEFAULT -1
);
INSERT INTO analytics_control(id) VALUES(1);
CREATE TABLE analytics_outbox (
 id TEXT PRIMARY KEY, account_id TEXT NOT NULL, event TEXT NOT NULL,
 occurred_at INTEGER NOT NULL, bytes INTEGER NOT NULL DEFAULT 0,
 captures INTEGER NOT NULL DEFAULT 0, pending_bytes INTEGER NOT NULL DEFAULT 0,
 physical_bytes INTEGER NOT NULL DEFAULT 0, consent_id TEXT
);
CREATE INDEX analytics_delivery ON analytics_outbox(occurred_at,id);
/* An operator may record an explicit choice before account creation, using the
   exact opaque account ID. No foreign key: signup is eligible only if that
   reviewed pre-enrolment exists. Browser consent never populates this ledger. */
CREATE TABLE analytics_consent (
 account_id TEXT PRIMARY KEY CHECK(length(account_id)=64 AND account_id NOT GLOB '*[^a-f0-9]*'),
 consent_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(16)))) UNIQUE,
 notice_version TEXT NOT NULL DEFAULT 'operator-recorded',
 consented_at INTEGER NOT NULL CHECK(typeof(consented_at)='integer' AND consented_at>0 AND consented_at<=9007199254740991)
);
/* A withdrawal tombstone prevents a delayed browser opt-in from restoring a
   deleted consent row. Reads never create choices; revisions are server-owned. */
CREATE TABLE analytics_choices (
 account_id TEXT PRIMARY KEY,
 revision TEXT NOT NULL CHECK(length(revision)=64 AND revision NOT GLOB '*[^a-f0-9]*'),
 decided_at INTEGER NOT NULL
);

CREATE TRIGGER analytics_consent_insert BEFORE INSERT ON analytics_consent BEGIN
 DELETE FROM analytics_outbox WHERE account_id=NEW.account_id;
END;
CREATE TRIGGER analytics_consent_update BEFORE UPDATE ON analytics_consent BEGIN
 SELECT RAISE(ABORT,'revoke and re-enrol with a new consent identity');
END;
CREATE TRIGGER analytics_consent_delete AFTER DELETE ON analytics_consent BEGIN
 DELETE FROM analytics_outbox WHERE account_id=OLD.account_id;
 INSERT INTO analytics_choices(account_id,revision,decided_at)
 VALUES(OLD.account_id,lower(hex(randomblob(32))),CAST(unixepoch('subsec')*1000 AS INTEGER))
 ON CONFLICT(account_id) DO UPDATE SET revision=excluded.revision,decided_at=excluded.decided_at;
END;
CREATE TRIGGER analytics_account_delete AFTER DELETE ON accounts BEGIN
 DELETE FROM analytics_consent WHERE account_id=OLD.id;
 DELETE FROM analytics_outbox WHERE account_id=OLD.id;
 DELETE FROM analytics_choices WHERE account_id=OLD.id;
END;

CREATE TRIGGER analytics_signup AFTER INSERT ON accounts
 WHEN (SELECT enabled FROM analytics_control WHERE id=1)=1 AND EXISTS (
  SELECT 1 FROM analytics_consent WHERE account_id=NEW.id AND consented_at<=NEW.created_at AND consented_at<=unixepoch('subsec')*1000
 ) BEGIN
 UPDATE analytics_control SET dropped=dropped+((SELECT count(*) FROM analytics_outbox)>=10000) WHERE id=1;
 INSERT INTO analytics_outbox(id,account_id,event,occurred_at,consent_id)
 SELECT 'signup:'||NEW.id,NEW.id,'connect_account_created',NEW.created_at,consent_id
 FROM analytics_consent WHERE account_id=NEW.id AND (SELECT count(*) FROM analytics_outbox)<10000;
END;
/* The adapter binds a grant only after the library's explicit device approval.
   Renewal/replay/revocation do not create another connection. */
CREATE TRIGGER analytics_device AFTER UPDATE OF account_id,user_id ON auth_grants
 WHEN OLD.user_id IS NULL AND NEW.user_id IS NOT NULL AND NEW.account_id IS NOT NULL AND NEW.revoked_at IS NULL
 AND (SELECT enabled FROM analytics_control WHERE id=1)=1 AND EXISTS (
  SELECT 1 FROM analytics_consent WHERE account_id=NEW.account_id AND consented_at<=unixepoch('subsec')*1000
 ) BEGIN
 UPDATE analytics_control SET dropped=dropped+((SELECT count(*) FROM analytics_outbox)>=10000) WHERE id=1;
 INSERT INTO analytics_outbox(id,account_id,event,occurred_at,consent_id)
 SELECT 'device:'||NEW.id,NEW.account_id,'connect_device_connected',CAST(unixepoch('subsec')*1000 AS INTEGER),consent_id
 FROM analytics_consent WHERE account_id=NEW.account_id AND (SELECT count(*) FROM analytics_outbox)<10000;
END;
CREATE TRIGGER analytics_upload AFTER UPDATE OF status ON captures
 WHEN NEW.status='ready' AND OLD.status='pending' AND (SELECT enabled FROM analytics_control WHERE id=1)=1 AND EXISTS (
  SELECT 1 FROM analytics_consent WHERE account_id=NEW.account_id AND consented_at<=unixepoch('subsec')*1000
 ) BEGIN
 UPDATE analytics_control SET dropped=dropped+((SELECT count(*) FROM analytics_outbox)>=10000) WHERE id=1;
 INSERT INTO analytics_outbox(id,account_id,event,occurred_at,bytes,captures,consent_id)
 SELECT 'upload:'||NEW.id,NEW.account_id,'connect_upload_completed',CAST(unixepoch('subsec')*1000 AS INTEGER),NEW.bytes,1,consent_id
 FROM analytics_consent WHERE account_id=NEW.account_id AND (SELECT count(*) FROM analytics_outbox)<10000;
END;
