-- Additive Better Auth 1.7.15 / OAuth Provider schema. No budget seeds or account rewrites.
PRAGMA foreign_keys = ON;
CREATE TABLE "auth_user" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "name" TEXT NOT NULL,
 "email" TEXT NOT NULL UNIQUE,
 "emailVerified" INTEGER NOT NULL,
 "image" TEXT,
 "createdAt" TEXT NOT NULL,
 "updatedAt" TEXT NOT NULL
);
CREATE TABLE "auth_session" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "expiresAt" TEXT NOT NULL,
 "token" TEXT NOT NULL UNIQUE,
 "createdAt" TEXT NOT NULL,
 "updatedAt" TEXT NOT NULL,
 "ipAddress" TEXT,
 "userAgent" TEXT,
 "userId" TEXT NOT NULL REFERENCES "auth_user"("id") ON DELETE CASCADE
);
CREATE INDEX "auth_session_userId" ON "auth_session"("userId");
CREATE TABLE "auth_account" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "accountId" TEXT NOT NULL,
 "providerId" TEXT NOT NULL,
 "userId" TEXT NOT NULL REFERENCES "auth_user"("id") ON DELETE CASCADE,
 "accessToken" TEXT,
 "refreshToken" TEXT,
 "idToken" TEXT,
 "accessTokenExpiresAt" TEXT,
 "refreshTokenExpiresAt" TEXT,
 "scope" TEXT,
 "password" TEXT,
 "createdAt" TEXT NOT NULL,
 "updatedAt" TEXT NOT NULL
);
CREATE INDEX "auth_account_userId" ON "auth_account"("userId");
CREATE TABLE "auth_verification" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "identifier" TEXT NOT NULL,
 "value" TEXT NOT NULL,
 "expiresAt" TEXT NOT NULL,
 "createdAt" TEXT NOT NULL,
 "updatedAt" TEXT NOT NULL
);
CREATE INDEX "auth_verification_identifier" ON "auth_verification"("identifier");
CREATE TABLE "oauthClient" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "clientId" TEXT NOT NULL UNIQUE,
 "clientSecret" TEXT,
 "clientDiscoveryId" TEXT,
 "disabled" INTEGER,
 "skipConsent" INTEGER,
 "enableEndSession" INTEGER,
 "subjectType" TEXT,
 "scopes" TEXT,
 "clientCredentialsScopes" TEXT,
 "userId" TEXT REFERENCES "auth_user"("id") ON DELETE CASCADE,
 "createdAt" TEXT,
 "updatedAt" TEXT,
 "name" TEXT,
 "uri" TEXT,
 "icon" TEXT,
 "contacts" TEXT,
 "tos" TEXT,
 "policy" TEXT,
 "softwareId" TEXT,
 "softwareVersion" TEXT,
 "softwareStatement" TEXT,
 "redirectUris" TEXT NOT NULL,
 "postLogoutRedirectUris" TEXT,
 "backchannelLogoutUri" TEXT,
 "backchannelLogoutSessionRequired" INTEGER,
 "tokenEndpointAuthMethod" TEXT,
 "applicationType" TEXT,
 "jwks" TEXT,
 "jwksUri" TEXT,
 "grantTypes" TEXT,
 "responseTypes" TEXT,
 "requirePKCE" INTEGER,
 "dpopBoundAccessTokens" INTEGER,
 "referenceId" TEXT,
 "metadata" TEXT
);
CREATE INDEX "oauthClient_userId" ON "oauthClient"("userId");
CREATE TABLE "oauthResource" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "identifier" TEXT NOT NULL UNIQUE,
 "name" TEXT NOT NULL,
 "accessTokenTtl" INTEGER,
 "refreshTokenTtl" INTEGER,
 "signingAlgorithm" TEXT,
 "signingKeyId" TEXT,
 "allowedScopes" TEXT,
 "customClaims" TEXT,
 "dpopBoundAccessTokensRequired" INTEGER,
 "disabled" INTEGER,
 "createdAt" TEXT,
 "updatedAt" TEXT,
 "policyVersion" INTEGER,
 "metadata" TEXT
);
CREATE TABLE "oauthClientResource" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "clientId" TEXT NOT NULL REFERENCES "oauthClient"("clientId") ON DELETE CASCADE,
 "resourceId" TEXT NOT NULL REFERENCES "oauthResource"("identifier") ON DELETE CASCADE,
 "metadata" TEXT,
 "createdAt" TEXT
);
CREATE INDEX "oauthClientResource_clientId" ON "oauthClientResource"("clientId");
CREATE INDEX "oauthClientResource_resourceId" ON "oauthClientResource"("resourceId");
CREATE TABLE "oauthRefreshToken" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "token" TEXT NOT NULL UNIQUE,
 "clientId" TEXT NOT NULL REFERENCES "oauthClient"("clientId") ON DELETE CASCADE,
 "sessionId" TEXT REFERENCES "auth_session"("id") ON DELETE SET NULL,
 "userId" TEXT NOT NULL REFERENCES "auth_user"("id") ON DELETE CASCADE,
 "referenceId" TEXT,
 "authorizationCodeId" TEXT,
 "resources" TEXT,
 "requestedUserInfoClaims" TEXT,
 "expiresAt" TEXT,
 "createdAt" TEXT,
 "revoked" TEXT,
 "rotatedAt" TEXT,
 "rotationReplayResponse" TEXT,
 "rotationReplayExpiresAt" TEXT,
 "authTime" TEXT,
 "confirmation" TEXT,
 "scopes" TEXT NOT NULL
);
CREATE INDEX "oauthRefreshToken_clientId" ON "oauthRefreshToken"("clientId");
CREATE INDEX "oauthRefreshToken_sessionId" ON "oauthRefreshToken"("sessionId");
CREATE INDEX "oauthRefreshToken_userId" ON "oauthRefreshToken"("userId");
CREATE INDEX "oauthRefreshToken_authorizationCodeId" ON "oauthRefreshToken"("authorizationCodeId");
CREATE TABLE "oauthAccessToken" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "token" TEXT UNIQUE,
 "clientId" TEXT NOT NULL REFERENCES "oauthClient"("clientId") ON DELETE CASCADE,
 "sessionId" TEXT REFERENCES "auth_session"("id") ON DELETE SET NULL,
 "userId" TEXT REFERENCES "auth_user"("id") ON DELETE CASCADE,
 "referenceId" TEXT,
 "authorizationCodeId" TEXT,
 "resources" TEXT,
 "requestedUserInfoClaims" TEXT,
 "refreshId" TEXT REFERENCES "oauthRefreshToken"("id") ON DELETE CASCADE,
 "expiresAt" TEXT,
 "createdAt" TEXT,
 "revoked" TEXT,
 "confirmation" TEXT,
 "scopes" TEXT NOT NULL
);
CREATE INDEX "oauthAccessToken_clientId" ON "oauthAccessToken"("clientId");
CREATE INDEX "oauthAccessToken_sessionId" ON "oauthAccessToken"("sessionId");
CREATE INDEX "oauthAccessToken_userId" ON "oauthAccessToken"("userId");
CREATE INDEX "oauthAccessToken_authorizationCodeId" ON "oauthAccessToken"("authorizationCodeId");
CREATE INDEX "oauthAccessToken_refreshId" ON "oauthAccessToken"("refreshId");
CREATE TABLE "oauthConsent" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "clientId" TEXT NOT NULL REFERENCES "oauthClient"("clientId") ON DELETE CASCADE,
 "userId" TEXT REFERENCES "auth_user"("id") ON DELETE CASCADE,
 "referenceId" TEXT,
 "resources" TEXT,
 "requestedUserInfoClaims" TEXT,
 "scopes" TEXT NOT NULL,
 "createdAt" TEXT,
 "updatedAt" TEXT
);
CREATE INDEX "oauthConsent_clientId" ON "oauthConsent"("clientId");
CREATE INDEX "oauthConsent_userId" ON "oauthConsent"("userId");
CREATE TABLE "oauthClientAssertion" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "expiresAt" TEXT NOT NULL
);
CREATE TABLE "auth_device" (
 "id" TEXT PRIMARY KEY NOT NULL,
 "deviceCode" TEXT NOT NULL,
 "userCode" TEXT NOT NULL,
 "userId" TEXT,
 "expiresAt" TEXT NOT NULL,
 "status" TEXT NOT NULL,
 "lastPolledAt" TEXT,
 "pollingInterval" INTEGER,
 "clientId" TEXT,
 "scope" TEXT,
 "resources" TEXT,
 "oauthClientId" TEXT
);
CREATE TABLE auth_grants (
 id TEXT PRIMARY KEY, client_id TEXT NOT NULL UNIQUE, device_hash TEXT NOT NULL UNIQUE,
 user_code TEXT NOT NULL UNIQUE, scope TEXT NOT NULL CHECK(scope IN ('read','upload')),
 account_id TEXT REFERENCES accounts(id), user_id TEXT REFERENCES auth_user(id),
 expires_at INTEGER NOT NULL, revoked_at INTEGER, response TEXT, last_poll INTEGER NOT NULL DEFAULT 0
);
-- Library family invalidation uses separate DELETEs on D1. Couple the FIRST
-- credential deletion to application denial in that same statement, even if
-- the Worker stops before the remaining rows are deleted. Ordinary expired
-- access cleanup must not revoke a still-refreshable device.
CREATE TRIGGER auth_access_deletion BEFORE DELETE ON oauthAccessToken
WHEN julianday(OLD.expiresAt)>julianday('now')
BEGIN
 UPDATE auth_grants SET revoked_at=COALESCE(revoked_at,unixepoch('subsec')*1000),response=NULL WHERE client_id=OLD.clientId;
END;
CREATE TRIGGER auth_refresh_deletion BEFORE DELETE ON oauthRefreshToken
BEGIN
 UPDATE auth_grants SET revoked_at=COALESCE(revoked_at,unixepoch('subsec')*1000),response=NULL WHERE client_id=OLD.clientId;
END;
-- Old credentials intentionally require one-time reapproval. Preserve identities/captures.
DROP TABLE sessions;
DROP TABLE oauth_states;
DROP TABLE devices;
DROP TABLE tokens;
