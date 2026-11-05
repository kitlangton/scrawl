import { createHash } from "node:crypto";
import { lstat } from "node:fs/promises";
import { isAbsolute, resolve } from "node:path";
import type { CaptureClient, CaptureStatus, PublicationExpectation } from "@apparent/engine-sdk/capture";
import { SqlitePublicationBindingStore, type PublicationBinding } from "./store.js";

type PublishInput = Parameters<CaptureClient["publish"]>[0];

export interface RecordPublicationRecovery {
  publish(input: PublishInput): Promise<CaptureStatus>;
  reconcile(captureId: string, ownerToken: string): Promise<CaptureStatus>;
}

/**
 * Record-owned durable publication identity around the published capture/2
 * client. Materials, plan bytes and derivative bytes remain host supplied.
 */
export function createRecordPublicationRecovery(
  client: CaptureClient,
  privateRoot: string,
): RecordPublicationRecovery {
  const store = new SqlitePublicationBindingStore(privateRoot);
  const inflight = new Map<string, { binding: PublicationBinding; promise: Promise<CaptureStatus> }>();

  return {
    publish(input) {
      const snapshot = structuredClone(input);
      const binding = bindingFrom(snapshot);
      const current = inflight.get(snapshot.captureId);
      if (current) {
        if (!sameBinding(current.binding, binding)) throw new Error("Record publication identity conflicts with its in-flight binding");
        return current.promise.then(status => structuredClone(status));
      }
      const operation = publishOnce(snapshot, binding).finally(() => inflight.delete(snapshot.captureId));
      inflight.set(snapshot.captureId, { binding, promise: operation });
      return operation.then(status => structuredClone(status));
    },
    async reconcile(captureId, ownerToken) {
      if (inflight.has(captureId)) throw new Error("Record publication command is already in flight");
      const binding = await store.get(captureId);
      if (!binding) throw new Error("Record publication binding is unavailable");
      if (!/^[0-9a-f]{64}$/u.test(ownerToken)) throw new Error("Record publication authority is invalid");
      try {
        const status = await client.inspectPublication({
          ownerToken,
          captureId: binding.captureId,
          publicationId: binding.publicationId,
          planSha256: binding.planSha256,
          expected: expectation(binding),
        });
        if (!await store.observe(binding, publicationState(status))) throw new Error("Stale publication observation");
        return structuredClone(status);
      } catch {
        throw new Error("Record publication outcome remains unresolved; inspect the same durable binding again");
      }
    },
  };

  async function publishOnce(input: PublishInput, binding: PublicationBinding): Promise<CaptureStatus> {
    const prior = await store.get(binding.captureId);
    if (prior && !sameBinding(prior, binding)) throw new Error("Record publication identity conflicts with its durable binding");
    if (!prior) await assertDestinationAbsent(binding.bundlePath);
    const won = await store.reserve(binding);
    if (!sameBinding(won, binding)) throw new Error("Record publication identity conflicts with its durable binding");

    // A prior dispatch may have reached Engine even if its reply was lost.
    // Reconcile first; never allocate replacement identities or dispatch twice.
    if (won.state !== "reserved") {
      if (!/^[0-9a-f]{64}$/u.test(input.ownerToken)) throw new Error("Record publication authority is invalid");
      try {
        const status = await client.inspectPublication({
          ownerToken: input.ownerToken,
          captureId: won.captureId,
          publicationId: won.publicationId,
          planSha256: won.planSha256,
          expected: expectation(won),
        });
        if (!await store.observe(won, publicationState(status))) throw new Error("Stale publication observation");
        return structuredClone(status);
      } catch {
        throw new Error("Record publication outcome remains unresolved; reconcile the same durable binding");
      }
    }

    await assertDestinationAbsent(binding.bundlePath);
    if (!await store.beginDispatch(won)) throw new Error("Record publication dispatch is already owned; reconcile the same durable binding");
    try {
      const status = await client.publish(input);
      if (!await store.observe(won, publicationState(status))) throw new Error("Stale publication observation");
      return structuredClone(status);
    } catch {
      // The row deliberately stays dispatching. No timeout/error implies cancel,
      // Saved, or material release.
      throw new Error("Record publication outcome is unknown; reconcile the same durable binding");
    }
  }
}

async function assertDestinationAbsent(path: string): Promise<void> {
  // Record only reserves private control metadata, never a bundle directory.
  try {
    await lstat(path);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw new Error("Record publication destination is unavailable");
  }
  throw new Error("Record publication destination already exists");
}

function bindingFrom(input: PublishInput): PublicationBinding {
  if (!/^[0-9a-f]{64}$/u.test(input.ownerToken)) throw new Error("Record publication authority is invalid");
  if (input.leaseId !== input.materials.manifest.leaseId) throw new Error("Record publication lease does not match the supplied materials manifest");
  const bytes = Buffer.from(input.planBytes);
  const planSha256 = createHash("sha256").update(bytes).digest("hex");
  const envelope = input.planFile;
  if (envelope.planSha256 !== planSha256 || envelope.planFile.sha256 !== planSha256 || envelope.planFile.identity.size !== bytes.byteLength)
    throw new Error("Record publication plan bytes do not match the sealed plan binding");
  let plan: unknown;
  try { plan = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)); }
  catch { throw new Error("Record publication plan binding is invalid"); }
  if (!plan || typeof plan !== "object" || Array.isArray(plan)) throw new Error("Record publication plan binding is invalid");
  const p = plan as Record<string, unknown>;
  const destination = p.destination as Record<string, unknown> | undefined;
  const materials = p.materials as Record<string, unknown> | undefined;
  const key = input.materials.manifest.key;
  const entries = p.entries;
  const manifest = Array.isArray(entries) ? entries.find((entry) => entry && typeof entry === "object" && (entry as Record<string, unknown>).relativePath === "apparent.json") as Record<string, unknown> | undefined : undefined;
  const manifestSha256 = manifest?.sha256;
  if (p.schema !== "apparent.bundle-publication/1" || p.publicationId !== input.publicationId ||
      p.bundleId !== input.materials.payload.bundleId || destination?.mode !== "absent" ||
      typeof destination.path !== "string" || !materials || materials.captureId !== key.captureId ||
      materials.materialsId !== key.materialsId || materials.revision !== key.revision ||
      typeof manifestSha256 !== "string" || !/^[0-9a-f]{64}$/u.test(manifestSha256))
    throw new Error("Record publication plan does not match the supplied immutable materials binding");
  const path = destination.path;
  if (!isAbsolute(path) || resolve(path) !== path || path.includes("\0") || path.length > 4096) throw new Error("Record publication destination is invalid");
  return {
    captureId: input.captureId,
    publicationId: input.publicationId,
    planSha256,
    materials: structuredClone(key),
    leaseId: input.leaseId,
    bundleId: p.bundleId as string,
    bundlePath: path,
    manifestSha256,
    state: "reserved",
  };
}

function expectation(binding: PublicationBinding): PublicationExpectation {
  return {
    materials: structuredClone(binding.materials),
    bundleId: binding.bundleId,
    bundlePath: binding.bundlePath,
    manifestSha256: binding.manifestSha256,
  };
}

function publicationState(status: CaptureStatus): "publishing" | "saved" | "failed" | "cancelled" {
  if (["publishing", "saved", "failed", "cancelled"].includes(status.state))
    return status.state as "publishing" | "saved" | "failed" | "cancelled";
  throw new Error("Engine returned a non-publication observation");
}

function sameBinding(left: PublicationBinding, right: PublicationBinding): boolean {
  return left.captureId === right.captureId && left.publicationId === right.publicationId &&
    left.planSha256 === right.planSha256 && JSON.stringify(left.materials) === JSON.stringify(right.materials) &&
    left.leaseId === right.leaseId && left.bundleId === right.bundleId && left.bundlePath === right.bundlePath &&
    left.manifestSha256 === right.manifestSha256;
}
