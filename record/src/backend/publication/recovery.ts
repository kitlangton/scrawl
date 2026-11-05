import { lstat } from "node:fs/promises";
import type { CaptureClient, CaptureStatus, PublicationExpectation, CapturePublishInput, CapturePublicationPreflight } from "@apparent/engine-sdk/capture";
import { SqlitePublicationBindingStore, type PublicationBinding } from "./store.js";

type PublishInput = CapturePublishInput;
export type PublicationPreflight = (input: PublishInput, timeoutMs?: number) => CapturePublicationPreflight;
const publishTimeoutMs = 10000;

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
  preflight: PublicationPreflight,
): RecordPublicationRecovery {
  if (typeof preflight !== "function") throw new TypeError("Record requires the public SDK publication preflight");
  const store = new SqlitePublicationBindingStore(privateRoot);
  const inflight = new Map<string, { binding: PublicationBinding; promise: Promise<CaptureStatus> }>();

  return {
    async publish(input) {
      const snapshot = structuredClone(input);
      const prepared = preflight(snapshot, publishTimeoutMs);
      const binding = bindingFrom(snapshot, prepared.expected);
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
        await client.handshake();
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
    if (!prior || prior.state === "reserved") {
      const hello = await client.handshake();
      if (hello.implementation !== "native" || !hello.capabilities.publication)
        throw Object.assign(new Error("Record publication is unavailable; no request was dispatched"), { dispatched: false });
    }
    if (!prior) await assertDestinationAbsent(binding.bundlePath);
    const won = await store.reserve(binding);
    if (!sameBinding(won, binding)) throw new Error("Record publication identity conflicts with its durable binding");

    // A prior dispatch may have reached Engine even if its reply was lost.
    // Reconcile first; never allocate replacement identities or dispatch twice.
    if (won.state !== "reserved") {
      if (!/^[0-9a-f]{64}$/u.test(input.ownerToken)) throw new Error("Record publication authority is invalid");
      try {
        await client.handshake();
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
      const status = await client.publish(input, publishTimeoutMs);
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

function bindingFrom(input: PublishInput, expected: PublicationExpectation): PublicationBinding {
  return {
    captureId: input.captureId,
    publicationId: input.publicationId,
    planSha256: input.planFile.planSha256,
    materials: structuredClone(expected.materials),
    leaseId: input.leaseId,
    bundleId: expected.bundleId,
    bundlePath: expected.bundlePath,
    manifestSha256: expected.manifestSha256,
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
