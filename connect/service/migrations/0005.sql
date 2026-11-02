-- Additive E2EE transport metadata. Apply once via the migration ledger;
-- replay errors (as in 0004), never resets budgets or incumbent records.
ALTER TABLE captures ADD COLUMN format TEXT NOT NULL DEFAULT 'plaintext-v1' CHECK(format IN ('plaintext-v1','apparent-encrypted-v1'));
ALTER TABLE auth_grants ADD COLUMN approved_at INTEGER;
CREATE TABLE encryption_keys (
 grant_id TEXT PRIMARY KEY REFERENCES auth_grants(id),
 version INTEGER NOT NULL CHECK(version=1),
 recipient TEXT NOT NULL,
 signing TEXT NOT NULL
);
CREATE INDEX encryption_keys_recipient ON encryption_keys(recipient);
CREATE INDEX encryption_keys_signing ON encryption_keys(signing);
-- Pending unauthenticated starts must not squat public keys. First explicit
-- account approval binds them; reapproval of the same account/scope/descriptor
-- is permitted, but cross-account/scope or half-descriptor rebinding is not.
CREATE TRIGGER encryption_key_binding BEFORE UPDATE OF account_id,user_id,approved_at ON auth_grants
WHEN NEW.approved_at IS NOT NULL AND EXISTS(
 SELECT 1 FROM encryption_keys k JOIN encryption_keys other ON other.grant_id!=k.grant_id
 JOIN auth_grants g ON g.id=other.grant_id
 WHERE k.grant_id=NEW.id AND (k.recipient=other.recipient OR k.signing=other.signing)
 AND g.approved_at IS NOT NULL AND (g.account_id!=NEW.account_id OR g.scope!=NEW.scope OR k.recipient!=other.recipient OR k.signing!=other.signing)
)
BEGIN SELECT RAISE(ABORT,'encryption key conflict'); END;
CREATE TRIGGER auth_grant_approval_immutable BEFORE UPDATE OF account_id,user_id,scope,approved_at ON auth_grants
WHEN OLD.approved_at IS NOT NULL AND (NEW.account_id IS NOT OLD.account_id OR NEW.user_id IS NOT OLD.user_id OR NEW.scope IS NOT OLD.scope OR NEW.approved_at IS NOT OLD.approved_at)
BEGIN SELECT RAISE(ABORT,'immutable approved grant'); END;
-- Keys are immutable even after revocation.
CREATE TRIGGER encryption_keys_immutable BEFORE UPDATE ON encryption_keys
BEGIN SELECT RAISE(ABORT,'immutable encryption key'); END;
CREATE TRIGGER encryption_keys_retained BEFORE DELETE ON encryption_keys
BEGIN SELECT RAISE(ABORT,'retained encryption key'); END;
