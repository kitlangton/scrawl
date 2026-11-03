/* Additive DAU activity only: no census, backfill, control reset or consent change.
   A durable first share-row insert qualifies; retries reuse that row. A pending
   share link is created activity, not proof that its recording is ready to watch. */
CREATE TRIGGER analytics_share_created AFTER INSERT ON shares
 WHEN (SELECT enabled FROM analytics_control WHERE id=1)=1 AND EXISTS (
  SELECT 1 FROM analytics_consent WHERE account_id=NEW.account_id
   AND consented_at<=NEW.created_at AND consented_at<=unixepoch('subsec')*1000
 ) BEGIN
 UPDATE analytics_control SET dropped=dropped+((SELECT count(*) FROM analytics_outbox)>=10000) WHERE id=1;
 INSERT INTO analytics_outbox(id,account_id,event,occurred_at,consent_id)
 SELECT 'share:'||NEW.id,NEW.account_id,'connect_share_created',NEW.created_at,consent_id
 FROM analytics_consent WHERE account_id=NEW.account_id AND (SELECT count(*) FROM analytics_outbox)<10000;
END;
