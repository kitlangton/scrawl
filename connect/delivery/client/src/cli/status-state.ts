import { projectLocalStatus, type LocalStatusFacts, type LocalStatus, type StatusGrantFacts } from "./status.ts";
import { readQueueWithinLease } from "../queue/store.ts";
import { readCredential, pendingIntent } from "../state/credential-files.ts";
import { CredentialSnapshotError, withStateQueue, checkSignal, type NamespaceResources } from "../state/queue-transaction.ts";

export type LocalStatusSettings = Readonly<Omit<LocalStatusFacts, "jobs" | "queuePath" | "readGrant" | "uploadGrant" | "revocationPending">>;

/** Read-only local status composition. The caller admits fresh settings and the
 * approved server and owns the resource-provider lifetime. No HTTP or renewal.
 * The existing stable lock may be created; no product JSON is written. */
export async function readLocalStatus(directoryPath: string, settings: LocalStatusSettings,
  resources: NamespaceResources, options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<LocalStatus> {
  const signal = options.signal, lockTimeoutMs = options.lockTimeoutMs;
  try {
    // Detach and validate policy inputs before filesystem work or the first await.
    const projected = projectLocalStatus({ ...settings, queuePath: directoryPath, jobs: [],
      readGrant: null, uploadGrant: null, revocationPending: false });
    const facts = { server: projected.server, serverValid: settings.serverValid, hostedSync: settings.hostedSync,
      cloudEnabled: settings.cloudEnabled, uploadConsent: projected.uploadConsent,
      encryption: projected.encryption, nowSeconds: settings.nowSeconds };
    return await withStateQueue(directoryPath, { scope: "read", approvedServer: facts.server }, resources,
      { signal, lockTimeoutMs }, async (directory, lease) => {
        async function grant(scope: "read" | "upload"): Promise<StatusGrantFacts | null> {
          checkSignal(signal, lease.signal); lease.assertHeld();
          try {
            const c = await readCredential(directory, { scope, approvedServer: facts.server }, resources, signal);
            return { scope, server: c.server, authRequired: c.authRequired, hasRefresh: c.refreshToken !== null,
              expiresAt: c.expiresAt, refreshExpiresAt: c.refreshExpiresAt };
          } catch (error) {
            // Missing, malformed or mismatched stored grants are signed out in
            // the donor. Unsafe descriptor/ACL or lease failures refuse status.
            if (error instanceof CredentialSnapshotError && error.category === "invalid_credential") return null;
            throw error;
          }
        }
        const revocationPending = await pendingIntent(directory.path);
        const readGrant = await grant("read"), uploadGrant = await grant("upload");
        const jobs = await readQueueWithinLease(directory, lease, resources, signal);
        return projectLocalStatus({ ...facts, queuePath: directoryPath, jobs, readGrant, uploadGrant,
          revocationPending: revocationPending || await pendingIntent(directory.path) });
      });
  } catch (error) {
    if (error instanceof Error && error.message === "invalid_local_status_facts") throw new Error("invalid_local_status_facts");
    if (signal?.aborted) throw new Error("cancelled");
    // State exceptions may contain private paths; local status exposes no raw diagnostics.
    throw new Error("local_status_unavailable");
  }
}

export { projectLocalStatus } from "./status.ts";
export type { LocalStatusFacts, LocalStatus, StatusGrantFacts } from "./status.ts";
