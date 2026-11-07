import { randomBytes } from "node:crypto";
import { readdir } from "node:fs/promises";
import { admitTrustedDirectory, type DirectoryResources } from "../state/directory.ts";
import { checkSignal, samePathFd, type CurrentNamespaceLease, type NamespaceResources, type StateDirectory } from "../state/queue-transaction.ts";
import { readStateEvidence } from "../state/credential-files.ts";
import { decodeStrictJson } from "../recipient/json.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "../state/replacement.ts";

export type RevocationFence = { assertHeld(): void; readonly signal: AbortSignal };
export type RevocationIntentResources = DirectoryResources & StateReplacementResources;
export type RevocationPublishResult = { state: "committed" | "refused" | "uncertain"; name: string };

const pendingName = (name: string) => name.startsWith("revocation-") && name.endsWith(".intent");
const fail = (): never => { throw new Error("revocation_intent_unavailable"); };
const assertFence = (fence: RevocationFence, signal?: AbortSignal) => {
  checkSignal(signal, fence.signal);
  fence.assertHeld();
};

/** Publish one immutable authorization-blocking intent. This does not settle it
 * or authorize any caller; the host must supply an admitted directory and fence. */
export async function publishRevocationIntent(directoryPath: string, logout: boolean, fence: RevocationFence,
  resources: RevocationIntentResources, options: { signal?: AbortSignal } = {}): Promise<RevocationPublishResult> {
  const signal = options.signal;
  const name = `revocation-${randomBytes(16).toString("hex")}.intent`;
  if (typeof logout !== "boolean") return { state: "refused", name };
  let admitted: Awaited<ReturnType<typeof admitTrustedDirectory>> | undefined;
  let dispatched = false;
  try {
    assertFence(fence, signal);
    admitted = await admitTrustedDirectory(directoryPath, resources, "private-state", signal);
    assertFence(fence, signal);
    if (!await samePathFd(admitted)) return { state: "refused", name };
    // The guarded port is the only route to native dispatch. Keep both the
    // admitted root descriptor and caller fence alive until it returns.
    const guarded: RevocationIntentResources = {
      checkAcl: resources.checkAcl.bind(resources),
      async replaceFile(parent, source, destination, expected, replaceOptions) {
        try {
          assertFence(fence, signal);
          if (destination !== name || expected.destination !== null || !await samePathFd(admitted!)) throw new Error();
          assertFence(fence, signal);
        } catch {
          await parent.close().catch(() => {});
          return { state: "refused" };
        }
        dispatched = true;
        return resources.replaceFile(parent, source, destination, expected, {
          ...replaceOptions,
          signal: AbortSignal.any([fence.signal, ...(signal ? [signal] : [])]),
        });
      },
    };
    const result = await replacePrivateStateFile(admitted.path, name, Buffer.from(JSON.stringify(logout)), null, fence, guarded, { signal });
    if (result.state === "publication_uncertain") return { state: "uncertain", name };
    assertFence(fence, signal);
    if (!await samePathFd(admitted)) return { state: result.state === "committed" ? "uncertain" : "refused", name };
    return { state: result.state === "committed" ? "committed" : "refused", name };
  } catch {
    return { state: dispatched ? "uncertain" : "refused", name };
  } finally { await admitted?.handle.close().catch(() => {}); }
}

export type RevocationIntentFact = Readonly<{ name: string; logout: boolean; stamp: string }>;
export type RevocationIntentSnapshot = Readonly<{ intents: readonly RevocationIntentFact[]; logout: boolean }>;

/** Read all Rust-compatible intent names under the caller's current queue lease.
 * Facts are detached observations only; they never confer authorization. */
export async function readRevocationIntentsWithinLease(directory: StateDirectory, lease: CurrentNamespaceLease,
  resources: NamespaceResources, options: { signal?: AbortSignal } = {}): Promise<RevocationIntentSnapshot> {
  const signal = options.signal;
  const check = async () => {
    checkSignal(signal, lease.signal);
    lease.assertHeld();
    if (!await samePathFd(directory)) fail();
    await lease.assertCurrent();
    checkSignal(signal, lease.signal);
  };
  try {
    await check();
    const names = (await readdir(directory.path)).filter(pendingName);
    if (names.length > 1024) return fail();
    const facts: RevocationIntentFact[] = [];
    for (const name of names) {
      await check();
      const evidence = await readStateEvidence(directory, name, 128, resources, lease.signal, bytes => {
        const value = decodeStrictJson(bytes, 128);
        if (typeof value !== "boolean") throw new Error();
        return value;
      });
      if (!evidence) return fail();
      facts.push(Object.freeze({ name, logout: evidence.value, stamp: evidence.expectedStamp }));
      await check();
    }
    await check();
    const intents = Object.freeze(facts);
    return Object.freeze({ intents, logout: facts.some(item => item.logout) });
  } catch { return fail(); }
}
