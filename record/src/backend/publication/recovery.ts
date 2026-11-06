import { lstat } from "node:fs/promises";
import type { CaptureClient, CaptureStatus, PublicationExpectation, CapturePublishInput, CapturePublicationPreflight } from "@apparent/engine-sdk/capture";
import { SqlitePublicationBindingStore, type PublicationBinding, type PublicationOutputMetadata } from "./store.js";

type PublishInput = CapturePublishInput;
export type PublicationPreflight = (input: PublishInput, timeoutMs?: number) => CapturePublicationPreflight;
const publishTimeoutMs = 10000;

export interface RecordPublicationRecovery {
  publish(input: PublishInput): Promise<CaptureStatus>;
  reconcile(captureId: string, ownerToken: string, expected?: PublicationBindingExpectation): Promise<CaptureStatus>;
}
export interface RecordPublicationRecoveryInternal extends RecordPublicationRecovery {
  publishWithOutput(input: PublishInput, output: PublicationOutputMetadata): Promise<CaptureStatus>;
}

export interface PublicationBindingExpectation {
  publicationId: string;
  materials: PublicationBinding["materials"];
  leaseId: string;
  bundleId: string;
  bundlePath: string;
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
  const internal = createRecordPublicationRecoveryInternal(client, privateRoot, preflight);
  return { publish: input => internal.publish(input), reconcile: (captureId, ownerToken, expected) => internal.reconcile(captureId, ownerToken, expected) };
}

/** Module-internal composition hook; only the completed publisher supplies assembler-authored processing. */
export function createRecordPublicationRecoveryInternal(
  client: CaptureClient,
  privateRoot: string,
  preflight: PublicationPreflight,
): RecordPublicationRecoveryInternal {
  if (typeof preflight !== "function") throw new TypeError("Record requires the public SDK publication preflight");
  const store = new SqlitePublicationBindingStore(privateRoot);
  const inflight = new Map<string, { binding: PublicationBinding; promise: Promise<CaptureStatus> }>();

  return {
    async publish(input) { return this.publishWithOutput(input, undefined as never); },
    async publishWithOutput(input, output) {
      const snapshot = structuredClone(input);
      const outputSnapshot = output && structuredClone(output);
      const prepared = preflight(snapshot, publishTimeoutMs);
      const binding = bindingFrom(snapshot, prepared.expected, outputSnapshot);
      const current = inflight.get(snapshot.captureId);
      if (current) {
        if (!sameBinding(current.binding, binding)) throw new Error("Record publication identity conflicts with its in-flight binding");
        return current.promise.then(status => structuredClone(status));
      }
      const operation = publishOnce(snapshot, binding).finally(() => inflight.delete(snapshot.captureId));
      inflight.set(snapshot.captureId, { binding, promise: operation });
      return operation.then(status => structuredClone(status));
    },
    async reconcile(captureId, ownerToken, expected) {
      const original = expected && structuredClone(expected);
      if (inflight.has(captureId)) throw new Error("Record publication command is already in flight");
      const binding = await store.get(captureId);
      if (!binding) throw new Error("Record publication binding is unavailable");
      if (original && !matchesExpected(binding, original)) throw new Error("Record publication binding conflicts with the supplied original identity");
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
        if (!await store.observe(binding, checkedState(status, binding), status.publication)) throw new Error("Stale publication observation");
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
        if (!await store.observe(won, checkedState(status, won), status.publication)) throw new Error("Stale publication observation");
        return structuredClone(status);
      } catch {
        throw new Error("Record publication outcome remains unresolved; reconcile the same durable binding");
      }
    }

    await assertDestinationAbsent(binding.bundlePath);
    if (!await store.beginDispatch(won)) throw new Error("Record publication dispatch is already owned; reconcile the same durable binding");
    try {
      const status = await client.publish(input, publishTimeoutMs);
      if (!await store.observe(won, checkedState(status, won), status.publication)) throw new Error("Stale publication observation");
      return structuredClone(status);
    } catch {
      // The row deliberately stays dispatching. No timeout/error implies cancel,
      // Saved, or material release.
      throw new Error("Record publication outcome is unknown; reconcile the same durable binding");
    }
  }
}

function matchesExpected(binding: PublicationBinding, expected: PublicationBindingExpectation): boolean {
  return binding.publicationId === expected.publicationId && binding.materials.captureId === expected.materials.captureId &&
    binding.materials.materialsId === expected.materials.materialsId && binding.materials.revision === expected.materials.revision &&
    binding.leaseId === expected.leaseId && binding.bundleId === expected.bundleId && binding.bundlePath === expected.bundlePath;
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

function bindingFrom(input: PublishInput, expected: PublicationExpectation, output?: PublicationOutputMetadata): PublicationBinding {
  return {
    captureId: input.captureId,
    publicationId: input.publicationId,
    planSha256: input.planFile.planSha256,
    materials: structuredClone(expected.materials),
    leaseId: input.leaseId,
    bundleId: expected.bundleId,
    bundlePath: expected.bundlePath,
    manifestSha256: expected.manifestSha256,
    ...(output ? { output } : {}),
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
  if (!status || typeof status !== "object") throw new Error("Engine returned an invalid publication observation");
  if (["publishing", "saved", "failed", "cancelled"].includes(status.state))
    return status.state as "publishing" | "saved" | "failed" | "cancelled";
  throw new Error("Engine returned a non-publication observation");
}

function checkedState(status: CaptureStatus, binding: PublicationBinding): "publishing" | "saved" | "failed" | "cancelled" {
  const state = publicationState(status);
  if (status.captureId !== binding.captureId || (status.publicationId !== undefined && status.publicationId !== binding.publicationId) ||
      (status.planSha256 !== undefined && status.planSha256 !== binding.planSha256) ||
      (status.materials !== undefined && JSON.stringify(status.materials.key) !== JSON.stringify(binding.materials)))
    throw new Error("Engine publication observation conflicts with the durable binding");
  if (state === "saved") {
    const receipt = status.publication;
    if (!receipt || receipt.captureId !== binding.captureId || JSON.stringify(receipt.materials) !== JSON.stringify(binding.materials) ||
      receipt.publicationId !== binding.publicationId || receipt.planSha256 !== binding.planSha256 || receipt.bundleId !== binding.bundleId ||
      receipt.bundlePath !== binding.bundlePath || receipt.manifestSha256 !== binding.manifestSha256 || receipt.artifactVersion !== 1 || receipt.artifactProvenance !== "native")
      throw new Error("Engine Saved receipt conflicts with the durable publication binding");
    if (!/^[0-9a-f]{64}$/u.test(receipt.inventorySha256) || Object.keys(receipt).sort().join(",") !==
      ["artifactProvenance", "artifactVersion", "bundleId", "bundlePath", "captureId", "inventorySha256", "manifestSha256", "materials", "planSha256", "publicationId"].join(","))
      throw new Error("Engine Saved receipt shape is invalid");
  }
  return state;
}

function sameBinding(left: PublicationBinding, right: PublicationBinding): boolean {
  return left.captureId === right.captureId && left.publicationId === right.publicationId &&
    left.planSha256 === right.planSha256 && JSON.stringify(left.materials) === JSON.stringify(right.materials) &&
    left.leaseId === right.leaseId && left.bundleId === right.bundleId && left.bundlePath === right.bundlePath &&
    left.manifestSha256 === right.manifestSha256 && JSON.stringify(left.output) === JSON.stringify(right.output);
}
