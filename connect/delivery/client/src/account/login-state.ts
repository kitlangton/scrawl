import { randomBytes } from "node:crypto";
import { lstat } from "node:fs/promises";
import path from "node:path";
import { decodeStrictJson } from "../recipient/json.ts";
import { normalizeRecipientServer } from "../recipient/transport.ts";
import { admitTrustedDirectory, DirectoryAdmissionFailure } from "../state/directory.ts";
import { pendingIntent, readCancellation, readOptionalStateFile, readStateEvidence, stateFileStamp } from "../state/credential-files.ts";
import { CredentialSnapshotError, checkSignal, samePathFd, withStateQueue, type NamespaceResources, type StateDirectory } from "../state/queue-transaction.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "../state/replacement.ts";

export type LoginScope = "read" | "upload";
export type LoginResources = NamespaceResources & StateReplacementResources;
export type LoginReceipt = Readonly<{ state: "begun"; scope: LoginScope; approvedServer: string; loginId: string;
  cancellation: string | null; directoryIdentity: string }>;
export type LoginResult = LoginReceipt | Readonly<{ state: "refused"; reason: "invalid_scope" | "upload_consent_required" | "invalid_server" | "cancelled" | "pending_intent" | "unsafe_state" | "state_unavailable" }>
  | Readonly<{ state: "uncertain"; proposedLoginId: string }>;

const refused = (reason: Extract<LoginResult, { state: "refused" }>['reason']): LoginResult => Object.freeze({ state: "refused", reason });
const stamp = async (directory: StateDirectory, name: string, resources: NamespaceResources, signal?: AbortSignal) => {
  const st = await lstat(path.join(directory.path, name), { bigint: true }).catch(e => (e as NodeJS.ErrnoException).code === "ENOENT" ? null : undefined);
  if (st === undefined || st && (!st.isFile() || st.isSymbolicLink() || st.nlink < 1n || st.uid !== BigInt(process.getuid!()) || (st.mode & 0o077n) !== 0n || st.size > 4096n)) throw new Error();
  if (!st) return null;
  const evidence = await readStateEvidence(directory, name, 4096, resources, signal ?? new AbortController().signal, () => true);
  if (!evidence || evidence.expectedStamp !== stateFileStamp(st)) throw new Error();
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
  let pinned: Awaited<ReturnType<typeof admitTrustedDirectory>> | undefined;
  const publication: { value: { state: "not_started" } | { state: "possible"; proposedLoginId: string } } = { value: { state: "not_started" } };
  try {
    // Explicitly bind inherited methods: resource objects may be class instances.
    const guarded: LoginResources = {
      checkAcl: resources.checkAcl.bind(resources), lock: resources.lock.bind(resources), replaceFile: resources.replaceFile.bind(resources),
    };
    pinned = await admitTrustedDirectory(directoryPath, guarded, "private-state", signal);
    const before = { path: pinned.path, identity: pinned.identity };
    if (await pendingIntent(before.path)) return refused("pending_intent");
    const cancellation = await readCancellation(before, guarded, signal);
    const markerName = `${scope}.login`;
    return await withStateQueue(before.path, { scope, approvedServer: server }, guarded, { signal, lockTimeoutMs }, async (directory, lease) => {
      const check = async () => {
        checkSignal(signal, lease.signal); lease.assertHeld(); await lease.assertCurrent();
        if (directory.identity !== before.identity || directory.identity !== pinned!.identity || !await samePathFd(directory)) throw new Error("fence");
        if (await pendingIntent(directory.path)) throw new Pending();
        if (await readCancellation(directory, guarded, signal) !== cancellation) throw new Error("fence");
      };
      try { await check(); } catch (e) { if (e instanceof Pending) return refused("pending_intent"); throw e; }
      const captured = await stamp(directory, markerName, guarded, signal);
      const proposedLoginId = randomBytes(16).toString("hex");
      const bytes = Buffer.from(JSON.stringify(proposedLoginId));
      try {
        await check();
        const dispatchResources: LoginResources = {
          checkAcl: guarded.checkAcl, lock: guarded.lock,
          async replaceFile(parent, source, destination, expected, replacementOptions) {
            try {
              await check();
              const dispatchMarker = await stamp(directory, markerName, guarded, signal);
              if ((dispatchMarker?.stamp ?? null) !== (captured?.stamp ?? null)) {
                await parent.close().catch(() => {}); return { state: "destination_changed" };
              }
              await check();
            } catch { await parent.close().catch(() => {}); return { state: "refused" }; }
            publication.value = { state: "possible", proposedLoginId };
            return guarded.replaceFile(parent, source, destination, expected, replacementOptions);
          },
        };
        const result = await replacePrivateStateFile(directory.path, markerName, bytes,
          captured?.identity ?? null, lease, dispatchResources, { signal });
        if (result.state === "publication_uncertain") return Object.freeze({ state: "uncertain", proposedLoginId });
        if (result.state !== "committed") { publication.value = { state: "not_started" }; return refused("state_unavailable"); }
        await check();
        const observed = await readOptionalStateFile(directory, markerName, 128, guarded, signal, value => decodeStrictJson(value, 128));
        if (observed !== proposedLoginId) throw new Error("readback");
        await check();
        return Object.freeze({ state: "begun", scope, approvedServer: server, loginId: proposedLoginId, cancellation, directoryIdentity: directory.identity });
      } finally { bytes.fill(0); }
    });
  } catch (error) {
    if (publication.value.state === "possible") return Object.freeze({ state: "uncertain", proposedLoginId: publication.value.proposedLoginId });
    if (error instanceof Pending) return refused("pending_intent");
    if (signal?.aborted || error instanceof CredentialSnapshotError && error.category === "cancelled" || error instanceof DirectoryAdmissionFailure && error.category === "cancelled") return refused("cancelled");
    if (error instanceof CredentialSnapshotError && error.category === "revocation_pending") return refused("pending_intent");
    return refused("state_unavailable");
  } finally { await pinned?.handle.close().catch(() => {}); }
}
class Pending extends Error {}
