import { DeviceFlowError, type DeviceTransport } from "../device-flow.cjs";
import { requestDevicePoll, requestDeviceStart, type DeviceScope } from "./device.ts";
import { normalizeRecipientServer, RecipientTransportError } from "../recipient/transport.ts";
import { validatePeerAuthorityKey, type PeerAuthorityKey } from "../recipient/peer-authority.ts";
import { prepareLoginCredential } from "../credentials/login-grant.ts";
import type { StoredCredential } from "../credentials/wire.ts";
import type { SafeError } from "../settings-presentation.js";
import { assertLoginStateCurrent, type LoginReceipt } from "./login-state.ts";
import type { LoginIdentityResult } from "../credentials/identity-store.ts";
import type { NamespaceResources } from "../state/queue-transaction.ts";

export type DeviceSessionContext = Readonly<{
  scope: DeviceScope; approvedServer: string; generation: string;
  encryptionIdentity: string | null; encryptionKey?: PeerAuthorityKey;
}>;
export type DeviceSessionPorts = Readonly<{
  signal: AbortSignal;
  assertCurrent(signal: AbortSignal): void | Promise<void>;
  commit(credential: StoredCredential, signal: AbortSignal): Promise<SafeError | null>;
  now?: () => number;
  issuedNow?: () => number;
}>;
export type DeviceLoginTransport = DeviceTransport & { dispose(): void };
export type StateDeviceSessionPorts = Omit<DeviceSessionPorts, "assertCurrent"> & Readonly<{
  generation: string;
  /** Host-owned fresh consent/settings check, outside queue.lock. */
  assertAllowed(signal: AbortSignal): void | Promise<void>;
  lockTimeoutMs?: number;
}>;
type SessionState = "new" | "starting" | "pending" | "polling" | "approved" | "committing" | "settled" | "failed" | "disposed";
type Approved = Readonly<{ status: "approved"; scope: DeviceScope; accessToken: string; refreshToken: string;
  grantId: string; expiresIn: number; refreshExpiresIn: number; accountInitial: string | null }>;
const SAFE_ERRORS = new Set<SafeError>(["cancelled", "offline", "auth-required", "rejected", "expired", "local-state", "invalid-server", "unverified-commit", "unavailable"]);
const validIdentity = (value: unknown): value is string => typeof value === "string" && /^[0-9a-f]{32}$/.test(value);
const fail = (code: SafeError = "unavailable"): never => { throw new DeviceFlowError(code); };
const safeError = (error: unknown): SafeError => {
  if (error instanceof DeviceFlowError && SAFE_ERRORS.has(error.code)) return error.code;
  if (error instanceof RecipientTransportError) {
    if (error.category === "cancelled") return "cancelled";
    if (error.category === "invalid_server") return "invalid-server";
    if (["request_failed", "offline", "timeout"].includes(error.category)) return "offline";
  }
  return "unavailable";
};

/** Private inactive adapter. Consent, durable identity and current-login authority are host-owned. */
export function createDeviceLoginTransport(context: DeviceSessionContext, ports: DeviceSessionPorts): DeviceLoginTransport {
  let scope: DeviceScope, approvedServer: string, generation: string, encryptionIdentity: string | null;
  let encryptionKey: PeerAuthorityKey | undefined;
  let originalSignal: AbortSignal, assertCurrent: DeviceSessionPorts["assertCurrent"], commit: DeviceSessionPorts["commit"];
  let monotonic: () => number, issueClock: () => number, createdAt: number;
  try {
    ({ scope, approvedServer, generation, encryptionIdentity } = context);
    const key = context.encryptionKey;
    originalSignal = ports.signal;
    assertCurrent = ports.assertCurrent.bind(ports); commit = ports.commit.bind(ports);
    monotonic = ports.now ?? (() => performance.now());
    issueClock = ports.issuedNow ?? (() => Math.floor(Date.now() / 1000));
    if (scope !== "read" && scope !== "upload" || !validIdentity(generation)
      || !(encryptionIdentity === null || validIdentity(encryptionIdentity))
      || !(originalSignal instanceof AbortSignal) || typeof monotonic !== "function" || typeof issueClock !== "function"
      || normalizeRecipientServer(approvedServer) !== approvedServer
      || scope === "read" && encryptionIdentity === null
      || (encryptionIdentity === null) !== (key === undefined)) return fail();
    if (key !== undefined) encryptionKey = validatePeerAuthorityKey(key);
    createdAt = monotonic();
    if (!Number.isFinite(createdAt) || createdAt < 0) return fail();
  } catch { return fail(); }
  const lifetime = 15 * 60_000, deadline = createdAt + lifetime;
  if (!Number.isFinite(deadline)) return fail();
  const disposal = new AbortController();
  const signal = AbortSignal.any([originalSignal, disposal.signal, AbortSignal.timeout(lifetime)]);
  let state: SessionState = "new", deviceCode: string | null = null;
  let approved: Readonly<{ reply: Approved; issuedAt: number }> | null = null;
  const clear = () => { approved = null; deviceCode = null; };
  const failed = () => { if (state !== "disposed") state = "failed"; clear(); };
  const active = () => {
    if (state === "disposed" || originalSignal.aborted) return fail("cancelled");
    const current = monotonic();
    if (!Number.isFinite(current) || current < createdAt) return fail();
    if (current >= deadline) return fail("expired");
    if (signal.aborted) return fail("cancelled");
    return true;
  };
  const check = async () => {
    active();
    try { await assertCurrent(signal); } catch { return fail("cancelled"); }
    active();
  };
  const timeout = (requested = 30_000) => Math.max(1, Math.min(requested, 30_000, Math.floor(deadline - monotonic())));
  const transport: DeviceLoginTransport = {
    async start() {
      if (state !== "new") return fail();
      state = "starting";
      try {
        await check();
        const reply = await requestDeviceStart({ approvedServer, scope, encryptionKey, timeoutMs: timeout(), signal, active });
        await check();
        if (!reply || typeof reply !== "object" || !("deviceCode" in reply)
          || typeof reply.deviceCode !== "string" || !/^[\x21-\x7e]{1,8192}$/.test(reply.deviceCode)) return fail();
        deviceCode = reply.deviceCode; state = "pending";
        return reply;
      } catch (error) { failed(); return fail(safeError(error)); }
    },
    async poll(code, timeoutMs) {
      if (state !== "pending" || !deviceCode || code !== deviceCode
        || !Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 30_000) return fail();
      state = "polling";
      try {
        await check();
        const issuedAt = issueClock();
        if (!Number.isSafeInteger(issuedAt) || issuedAt < 0) return fail();
        const reply = await requestDevicePoll({ approvedServer, deviceCode, timeoutMs: timeout(timeoutMs), signal, active });
        await check();
        if (reply && typeof reply === "object" && "status" in reply && reply.status === "approved") {
          // Reuse the private grant validator; the actual approval site is bound
          // by authorizeDevice and supplied again at the one-shot commit.
          const candidate = prepareLoginCredential(reply, { scope, approvedServer, generation, encryptionIdentity,
            issuedAt, siteOrigin: new URL(approvedServer).origin });
          approved = Object.freeze({ issuedAt, reply: Object.freeze({ status: "approved", scope,
            accessToken: candidate.token, refreshToken: candidate.refreshToken!, grantId: candidate.grantId!,
            expiresIn: candidate.expiresAt! - issuedAt, refreshExpiresIn: candidate.refreshExpiresAt! - issuedAt,
            accountInitial: candidate.accountInitial }) });
          state = "approved";
        } else if (reply && typeof reply === "object" && "status" in reply && reply.status === "pending") state = "pending";
        else failed();
        return reply;
      } catch (error) { failed(); return fail(safeError(error)); }
    },
    async commit(siteOrigin) {
      if (state !== "approved" || !approved) return fail();
      state = "committing";
      const snapshot = approved; approved = null;
      let dispatched = false;
      try {
        await check();
        const credential = prepareLoginCredential(snapshot.reply, { approvedServer, siteOrigin, scope, generation,
          issuedAt: snapshot.issuedAt, encryptionIdentity });
        await check();
        dispatched = true;
        const result = await commit(credential, signal);
        await check();
        if (result !== null && !(typeof result === "string" && SAFE_ERRORS.has(result))) {
          failed(); return "unverified-commit";
        }
        state = result === null ? "settled" : "failed"; clear();
        return result;
      } catch (error) { failed(); return dispatched ? "unverified-commit" : safeError(error); }
    },
    dispose() { state = "disposed"; clear(); encryptionKey = undefined; disposal.abort(); },
  };
  return Object.freeze(transport);
}

/** Internal composition only. A durable host commit must independently recheck
 * these fences under its own writer transaction; this read grants no write lease. */
export function createStateBackedDeviceLoginTransport(directory: string, receipt: LoginReceipt,
  identity: LoginIdentityResult | null, resources: NamespaceResources, ports: StateDeviceSessionPorts): DeviceLoginTransport {
  try {
    const pinned: LoginReceipt = Object.freeze({ state: receipt.state, scope: receipt.scope, approvedServer: receipt.approvedServer,
      loginId: receipt.loginId, cancellation: receipt.cancellation, directoryIdentity: receipt.directoryIdentity });
    if (pinned.state !== "begun" || identity !== null &&
      (identity.state !== "published" && identity.state !== "existing" || identity.identityId !== pinned.loginId)) return fail();
    const bound = { checkAcl: resources.checkAcl.bind(resources), lock: resources.lock.bind(resources) };
    const allowed = ports.assertAllowed.bind(ports), commit = ports.commit.bind(ports);
    const signal = ports.signal, lockTimeoutMs = ports.lockTimeoutMs;
    return createDeviceLoginTransport({ approvedServer: pinned.approvedServer, scope: pinned.scope, generation: ports.generation,
      encryptionIdentity: identity === null ? null : identity.identityId, encryptionKey: identity?.publicKey }, {
      signal, commit, now: ports.now, issuedNow: ports.issuedNow,
      async assertCurrent(fenceSignal) {
        await allowed(fenceSignal);
        await assertLoginStateCurrent(directory, pinned, bound, { signal: fenceSignal, lockTimeoutMs });
        await allowed(fenceSignal);
      },
    });
  } catch { return fail(); }
}
