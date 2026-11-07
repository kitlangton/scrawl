import { randomBytes } from "node:crypto";
import { lstat } from "node:fs/promises";
import path from "node:path";
import { normalizeRecipientServer } from "../recipient/transport.ts";
import { admitTrustedDirectory, DirectoryAdmissionFailure } from "../state/directory.ts";
import { pendingIntent, readCancellation, readOptionalStateFile, readStateEvidence, stateFileStamp } from "../state/credential-files.ts";
import { CredentialSnapshotError, checkSignal, samePathFd, withStateQueue, type NamespaceResources } from "../state/queue-transaction.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "../state/replacement.ts";

export type LoginScope = "read" | "upload";
export type LoginResources = NamespaceResources & StateReplacementResources;
export type LoginReceipt = Readonly<{ state: "begun"; scope: LoginScope; approvedServer: string; loginId: string;
  cancellation: string | null; directoryIdentity: string }>;
export type LoginResult = LoginReceipt | Readonly<{ state: "refused"; reason: "invalid_scope" | "upload_consent_required" | "invalid_server" | "cancelled" | "pending_intent" | "unsafe_state" | "state_unavailable" }>
  | Readonly<{ state: "uncertain"; proposedLoginId: string }>;

const refused = (reason: Extract<LoginResult, { state: "refused" }>['reason']): LoginResult => Object.freeze({ state: "refused", reason });
const stamp = async (directory: { path: string; identity: string }, name: string, resources: NamespaceResources, signal?: AbortSignal) => {
  const st = await lstat(path.join(directory.path, name), { bigint: true }).catch(e => (e as NodeJS.ErrnoException).code === "ENOENT" ? null : undefined);
  if (st === undefined || st && (!st.isFile() || st.isSymbolicLink() || st.nlink !== 1n || st.uid !== BigInt(process.getuid!()) || (st.mode & 0o077n) !== 0n || st.size > 4096n)) throw new Error();
  if (!st) return null;
  const evidence = await readStateEvidence(directory as { handle: import("node:fs/promises").FileHandle; path: string; identity: string }, name, 4096, resources, signal ?? new AbortController().signal, bytes => new Uint8Array(bytes));
  if (!evidence || evidence.expectedStamp !== stateFileStamp(st)) throw new Error();
  (evidence.value as Uint8Array).fill(0);
  return { stamp: evidence.expectedStamp, identity: { device: String(st.dev), inode: String(st.ino) } };
};

/** Inactive prerequisite only: creates a scope login incarnation, never credentials or identity. */
export async function beginLoginState(directoryPath: string,
  context: { scope: LoginScope; approvedServer: string; uploadConsent: boolean }, resources: LoginResources,
  options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<LoginResult> {
  const scope = context?.scope, serverInput = context?.approvedServer, consent = context?.uploadConsent;
  const signal = options?.signal, lockTimeoutMs = options?.lockTimeoutMs;
  if (scope !== "read" && scope !== "upload") return refused("invalid_scope");
  if (scope === "upload" && consent !== true) return refused("upload_consent_required");
  let server: string;
  try { server = normalizeRecipientServer(serverInput); if (server !== serverInput) return refused("invalid_server"); } catch { return refused("invalid_server"); }
  if (signal?.aborted) return refused("cancelled");
  // Explicitly bind inherited methods: resource objects may be class instances.
  const guarded: LoginResources = {
    checkAcl: resources.checkAcl.bind(resources), lock: resources.lock.bind(resources), replaceFile: resources.replaceFile.bind(resources),
  };
  let pinned: Awaited<ReturnType<typeof admitTrustedDirectory>> | undefined;
  let published = false, proposedLoginId = "";
  try {
    pinned = await admitTrustedDirectory(directoryPath, guarded, "private-state", signal);
    const before = { path: pinned.path, identity: pinned.identity };
    if (await pendingIntent(before.path)) return refused("pending_intent");
    const cancellation = await readCancellation(before, guarded, signal);
    const markerName = `${scope}.login`;
    const captured = await stamp(before, markerName, guarded, signal);
    return await withStateQueue(before.path, { scope, approvedServer: server }, guarded, { signal, lockTimeoutMs }, async (directory, lease) => {
      const check = async () => {
        checkSignal(signal, lease.signal); lease.assertHeld(); await lease.assertCurrent();
        if (directory.identity !== before.identity || directory.identity !== pinned!.identity || !await samePathFd(directory)) throw new Error("fence");
        if (await pendingIntent(directory.path)) throw new Pending();
        if (await readCancellation(directory, guarded, signal) !== cancellation) throw new Error("fence");
      };
      try { await check(); } catch (e) { if (e instanceof Pending) return refused("pending_intent"); throw e; }
      const current = await stamp(directory, markerName, guarded, signal);
      if ((current?.stamp ?? null) !== (captured?.stamp ?? null)) return refused("state_unavailable");
      proposedLoginId = randomBytes(16).toString("hex");
      const bytes = Buffer.from(JSON.stringify(proposedLoginId));
      try {
        await check();
        const dispatchResources: LoginResources = {
          checkAcl: guarded.checkAcl, lock: guarded.lock,
          async replaceFile(parent, source, destination, expected, replacementOptions) {
            await check();
            const dispatchMarker = await stamp(directory, markerName, guarded, signal);
            if ((dispatchMarker?.stamp ?? null) !== (captured?.stamp ?? null)) return { state: "destination_changed" };
            await check();
            return guarded.replaceFile(parent, source, destination, expected, replacementOptions);
          },
        };
        const result = await replacePrivateStateFile(directory.path, markerName, bytes,
          captured?.identity ?? null, lease, dispatchResources, { signal });
        if (result.state === "publication_uncertain") { published = true; return Object.freeze({ state: "uncertain", proposedLoginId }); }
        if (result.state !== "committed") return refused("state_unavailable");
        published = true;
        await check();
        const observed = await readOptionalStateFile(directory, markerName, 128, guarded, signal, value => Buffer.from(value).toString("utf8"));
        if (observed !== JSON.stringify(proposedLoginId)) throw new Error("readback");
        await check();
        return Object.freeze({ state: "begun", scope, approvedServer: server, loginId: proposedLoginId, cancellation, directoryIdentity: directory.identity });
      } finally { bytes.fill(0); }
    });
  } catch (error) {
    if (published) return Object.freeze({ state: "uncertain", proposedLoginId });
    if (error instanceof Pending) return refused("pending_intent");
    if (signal?.aborted || error instanceof CredentialSnapshotError && error.category === "cancelled" || error instanceof DirectoryAdmissionFailure && error.category === "cancelled") return refused("cancelled");
    if (error instanceof CredentialSnapshotError && error.category === "revocation_pending") return refused("pending_intent");
    return refused("state_unavailable");
  } finally { await pinned?.handle.close().catch(() => {}); }
}
class Pending extends Error {}
