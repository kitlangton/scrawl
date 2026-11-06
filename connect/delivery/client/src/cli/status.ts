/** Secret-free input facts admitted by the local caller. This module performs no I/O. */
export type StatusGrantFacts = Readonly<{
  scope: "read" | "upload";
  server: string;
  authRequired: boolean;
  hasRefresh: boolean;
  expiresAt: number | null;
  refreshExpiresAt: number | null;
}>;

export type StatusJob = Readonly<{
  id: string;
  bundle: string;
  status: string;
  encrypted: boolean;
  attempts: number;
  nextAttempt: number;
  error?: string | null;
  sizeFailure?: unknown;
}>;

type ProjectedJob = Readonly<{
  id: string; bundle: string; status: string; encrypted: boolean; attempts: number;
  nextAttempt: number; error: string | null; sizeFailure: unknown | null;
}>;

export type LocalStatusFacts = Readonly<{
  server: string;
  serverValid: boolean;
  hostedSync: boolean;
  cloudEnabled: boolean;
  uploadConsent: boolean;
  revocationPending: boolean;
  encryption: boolean;
  queuePath: string;
  nowSeconds: number;
  readGrant: StatusGrantFacts | null;
  uploadGrant: StatusGrantFacts | null;
  jobs: readonly StatusJob[];
}>;

type AuthStatus = Readonly<{
  status: "signed_out" | "auth_required" | "legacy_reapproval_on_expiry" | "renewal_due" | "signed_in";
  expiresAt?: number | null;
  refreshExpiresAt?: number | null;
  action?: string | null;
}>;

export type LocalStatus = Readonly<{
  server: string;
  enabled: boolean;
  uploadConsent: boolean;
  readSignedIn: boolean;
  uploadSignedIn: boolean;
  encryption: boolean;
  queuePath: string;
  queue: readonly ProjectedJob[];
  readAuth: AuthStatus;
  uploadAuth: AuthStatus;
}>;

const badFacts = (): never => { throw new Error("invalid_local_status_facts"); };
const record = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const uint = (v: unknown): v is number => Number.isSafeInteger(v) && typeof v === "number" && v >= 0;
const nullableExpiry = (v: unknown): v is number | null => v === null || uint(v);

function validateGrant(value: unknown, scope: "read" | "upload", server: string): StatusGrantFacts | null {
  if (value === null) return null;
  if (!record(value) || value.scope !== scope || typeof value.server !== "string" ||
    typeof value.authRequired !== "boolean" || typeof value.hasRefresh !== "boolean" ||
    !nullableExpiry(value.expiresAt) || !nullableExpiry(value.refreshExpiresAt)) return badFacts();
  if (value.hasRefresh ? !(typeof value.expiresAt === "number" && value.expiresAt > 0 &&
      typeof value.refreshExpiresAt === "number" && value.refreshExpiresAt > 0) : value.refreshExpiresAt !== null) return badFacts();
  // Caller supplies context-matched facts. A mismatch is absent for status purposes.
  return value.server === server ? {
    scope, server, authRequired: value.authRequired, hasRefresh: value.hasRefresh,
    expiresAt: value.expiresAt, refreshExpiresAt: value.refreshExpiresAt,
  } : null;
}

function validateJob(value: unknown): ProjectedJob {
  if (!record(value) || typeof value.id !== "string" || typeof value.bundle !== "string" ||
    typeof value.status !== "string" || typeof value.encrypted !== "boolean" ||
    !uint(value.attempts) || !uint(value.nextAttempt) ||
    !(value.error === undefined || value.error === null || typeof value.error === "string")) return badFacts();
  let sizeFailure: unknown | null = null;
  if (value.sizeFailure !== undefined && value.sizeFailure !== null) {
    const failure = value.sizeFailure;
    if (!record(failure) || typeof failure.path !== "string" || !uint(failure.size) || !uint(failure.limit)) return badFacts();
    sizeFailure = { path: failure.path, size: failure.size, limit: failure.limit };
  }
  return {
    id: value.id, bundle: value.bundle, status: value.status, encrypted: value.encrypted,
    attempts: value.attempts, nextAttempt: value.nextAttempt, error: value.error ?? null,
    sizeFailure,
  };
}

function usable(grant: StatusGrantFacts, now: number): boolean {
  return !grant.authRequired && (grant.refreshExpiresAt === null || grant.refreshExpiresAt > now) &&
    (grant.hasRefresh || grant.expiresAt === null || grant.expiresAt > now);
}

function auth(grant: StatusGrantFacts | null, scope: "read" | "upload", now: number): AuthStatus {
  if (!grant) return { status: "signed_out" };
  const status: AuthStatus["status"] = !usable(grant, now) ? "auth_required" :
    !grant.hasRefresh ? "legacy_reapproval_on_expiry" :
    grant.expiresAt !== null && grant.expiresAt <= now + 30 ? "renewal_due" : "signed_in";
  return {
    status, expiresAt: grant.expiresAt, refreshExpiresAt: grant.refreshExpiresAt,
    action: status === "auth_required"
      ? scope === "upload" ? "Run apparent connect login --scope upload --consent-upload for this server" : "Run apparent connect login --scope read for this server"
      : null,
  };
}

function freeze<T>(value: T): T {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value as Record<string, unknown>)) freeze(child);
  }
  return value;
}

/** Pure projection of already admitted local facts into the Rust status(false) JSON shape. */
export function projectLocalStatus(value: unknown): LocalStatus {
  if (!record(value) || typeof value.server !== "string" || typeof value.serverValid !== "boolean" ||
    typeof value.hostedSync !== "boolean" || typeof value.cloudEnabled !== "boolean" ||
    typeof value.uploadConsent !== "boolean" || typeof value.revocationPending !== "boolean" ||
    typeof value.encryption !== "boolean" || typeof value.queuePath !== "string" ||
    !uint(value.nowSeconds) || !Array.isArray(value.jobs)) return badFacts();
  const facts = value as unknown as LocalStatusFacts;
  const read = validateGrant(value.readGrant, "read", facts.server);
  const upload = validateGrant(value.uploadGrant, "upload", facts.server);
  const queue = facts.jobs.map(validateJob);
  const now = facts.nowSeconds;
  const result: LocalStatus = {
    server: facts.server,
    enabled: !facts.revocationPending && facts.hostedSync && facts.cloudEnabled && facts.uploadConsent && facts.serverValid,
    uploadConsent: facts.uploadConsent,
    readSignedIn: read !== null && usable(read, now),
    uploadSignedIn: upload !== null && usable(upload, now),
    encryption: facts.encryption,
    queuePath: facts.queuePath,
    queue,
    readAuth: auth(read, "read", now),
    uploadAuth: auth(upload, "upload", now),
  };
  return freeze(result);
}
