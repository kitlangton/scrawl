import { randomBytes } from "node:crypto";
import { lstat, unlink } from "node:fs/promises";
import type { BigIntStats } from "node:fs";
import path from "node:path";
import { decodeStrictJson } from "../recipient/json.ts";
import { readRevocationIntentsWithinLease, type RevocationIntentFact } from "./revocation-intent.ts";
import { readStateEvidence, readCancellation, stateFileStamp } from "../state/credential-files.ts";
import { checkSignal, samePathFd, withStateQueue, type NamespaceResources, type StateDirectory } from "../state/queue-transaction.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "../state/replacement.ts";

type Outcome = { state: "prepared"; epoch: string; intents: readonly RevocationIntentFact[]; logout: boolean }
  | { state: "refused"; reason: "intent_unavailable" | "unsafe_state" | "stale_snapshot" | "cancelled" }
  | { state: "uncertain"; proposedEpoch: string };
type Resources = NamespaceResources & StateReplacementResources;
type Marker = { name: "upload.login" | "read.login"; stamp: string | null };
const privateStamp = (stat: BigIntStats) => stateFileStamp(stat);

/** Inactive preparation for a future coordinated writer. This only fences work
 * and invalidates captured login markers; it does not disable configuration,
 * settle intents, or revoke remote grants. Same-UID concurrent path mutation
 * outside queue.lock remains outside the cooperative namespace guarantee. */
export async function prepareRevocationState(directoryPath: string, context: { approvedServer: string },
  resources: Resources, options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<Outcome> {
  // Detach caller-owned context before the first await.
  const ctx = { scope: "upload" as const, approvedServer: context?.approvedServer };
  const opts = { signal: options.signal, lockTimeoutMs: options.lockTimeoutMs };
  const phase: { value: "before_effect" | "effect_possible" } = { value: "before_effect" };
  let proposedEpoch = "";
  try {
    if (typeof ctx.approvedServer !== "string" || !ctx.approvedServer) return { state: "refused", reason: "unsafe_state" };
    return await withStateQueue(directoryPath, ctx, resources, opts, async (directory, lease): Promise<Outcome> => {
      const signal = AbortSignal.any([lease.signal, ...(opts.signal ? [opts.signal] : [])]);
      const check = async () => {
        checkSignal(signal, lease.signal); lease.assertHeld();
        if (!await samePathFd(directory)) throw new Error("unsafe");
        await lease.assertCurrent(); checkSignal(signal, lease.signal); lease.assertHeld();
      };
      const intents = await readRevocationIntentsWithinLease(directory, lease, resources, { signal });
      if (!intents.intents.length) return { state: "refused", reason: "intent_unavailable" };
      const cancellationEvidence = await readStateEvidence(directory, "cancellation", 128, resources, signal, bytes => {
        const value = decodeStrictJson(bytes, 128); if (typeof value !== "string") throw new Error(); return value;
      });
      const cancellation = cancellationEvidence?.value ?? null;
      const cancellationStamp = cancellationEvidence?.expectedStamp ?? null;
      const logins: Marker[] = [];
      for (const name of ["upload.login", "read.login"] as const) {
        const marker = await readStateEvidence(directory, name, 128, resources, signal, bytes => {
          const value = decodeStrictJson(bytes, 128); if (typeof value !== "string") throw new Error(); return value;
        });
        logins.push({ name, stamp: marker?.expectedStamp ?? null });
      }
      proposedEpoch = randomBytes(16).toString("hex");
      const verifyCaptured = async () => {
        await check();
        const currentIntents = await readRevocationIntentsWithinLease(directory, lease, resources, { signal });
        for (const covered of intents.intents) {
          const now = currentIntents.intents.find(item => item.name === covered.name);
          if (!now || now.stamp !== covered.stamp || now.logout !== covered.logout) throw new Error("stale");
        }
        const currentCancellation = await readStateEvidence(directory, "cancellation", 128, resources, signal, bytes => {
          const value = decodeStrictJson(bytes, 128); if (typeof value !== "string") throw new Error(); return value;
        });
        if ((currentCancellation?.expectedStamp ?? null) !== cancellationStamp
          || (currentCancellation?.value ?? null) !== cancellation) throw new Error("stale");
        for (const marker of logins) {
          const now = await lstat(path.join(directory.path, marker.name), { bigint: true }).catch(error => {
            if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; throw error;
          });
          if (marker.stamp === null ? now !== null : !now || privateStamp(now) !== marker.stamp || !now.isFile() || now.isSymbolicLink()) throw new Error("stale");
        }
        await check();
      };
      await verifyCaptured();
      const cancellationFile = path.join(directory.path, "cancellation");
      const current = cancellationEvidence ? await lstat(cancellationFile, { bigint: true }) : null;
      if ((current ? privateStamp(current) : null) !== cancellationStamp) return { state: "refused", reason: "stale_snapshot" };
      const guarded: Resources = {
        checkAcl: resources.checkAcl.bind(resources), lock: resources.lock.bind(resources),
        async replaceFile(parent, source, destination, expected, replaceOptions) {
          try { await verifyCaptured(); } catch { await parent.close().catch(() => {}); return { state: "refused" }; }
          return resources.replaceFile(parent, source, destination, expected,
            { ...replaceOptions, signal: AbortSignal.any([signal, ...(replaceOptions?.signal ? [replaceOptions.signal] : [])]) });
        },
      };
      phase.value = "effect_possible";
      const replacement = await replacePrivateStateFile(directory.path, "cancellation", Buffer.from(JSON.stringify(proposedEpoch)),
        current ? { device: String(current.dev), inode: String(current.ino) } : null, lease, guarded, { signal });
      if (replacement.state === "publication_uncertain") return { state: "uncertain", proposedEpoch };
      if (replacement.state !== "committed") { phase.value = "before_effect"; return { state: "refused", reason: "stale_snapshot" }; }
      await check();
      if (await readCancellation(directory, resources, signal) !== proposedEpoch) return { state: "uncertain", proposedEpoch };
      // A new intent published after our snapshot is intentionally left in place
      // and does not become part of this receipt.
      await verifyCoveredIntentsOnly(directory, lease, resources, signal, intents.intents);
      for (const marker of logins) {
        await check();
        const file = path.join(directory.path, marker.name);
        const named = await lstat(file, { bigint: true }).catch(error => {
          if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; throw error;
        });
        if (marker.stamp === null) {
          if (named !== null) return { state: "uncertain", proposedEpoch };
          continue;
        }
        if (!named || !named.isFile() || named.isSymbolicLink() || privateStamp(named) !== marker.stamp) return { state: "uncertain", proposedEpoch };
        // Cooperative queue exclusion plus stamp checking; unlink is not inode-atomic.
        await unlink(file);
      }
      await directory.handle.sync(); await check();
      if (await readCancellation(directory, resources, signal) !== proposedEpoch) return { state: "uncertain", proposedEpoch };
      for (const marker of logins) if (await lstat(path.join(directory.path, marker.name), { bigint: true }).then(() => true, e => (e as NodeJS.ErrnoException).code !== "ENOENT")) return { state: "uncertain", proposedEpoch };
      return Object.freeze({ state: "prepared", epoch: proposedEpoch, intents: intents.intents, logout: intents.logout });
    });
  } catch (error) {
    if (phase.value === "effect_possible") return { state: "uncertain", proposedEpoch };
    const category = error && typeof error === "object" && "category" in error && typeof (error as { category: unknown }).category === "string" ? (error as { category: string }).category : "";
    return { state: "refused", reason: category === "cancelled" ? "cancelled" : category === "revocation_pending" ? "intent_unavailable" : "unsafe_state" };
  }
}

async function verifyCoveredIntentsOnly(directory: StateDirectory, lease: Parameters<typeof readRevocationIntentsWithinLease>[1],
  resources: NamespaceResources, signal: AbortSignal, covered: readonly RevocationIntentFact[]) {
  const current = await readRevocationIntentsWithinLease(directory, lease, resources, { signal });
  for (const item of covered) {
    const found = current.intents.find(candidate => candidate.name === item.name);
    if (!found || found.stamp !== item.stamp || found.logout !== item.logout) throw new Error("intent_changed");
  }
}
