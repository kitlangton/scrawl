import { isDeepStrictEqual } from "node:util";
import { createHash } from "node:crypto";
import type { CaptureClient, CaptureStatus, MaterialsDocument } from "@apparent/engine-sdk/capture";
import { validateMaterialsDocument } from "@apparent/engine-sdk/capture";
import { assembleCanonicalBundle, type BundleAssemblyInput } from "../processing/bundle/index.js";
import { prepareRecordPublication, type PreparePublicationInput } from "./staging.js";
import { createRecordPublicationRecoveryInternal, type PublicationBindingExpectation, type PublicationPreflight } from "./recovery.js";
import { SqlitePublicationBindingStore } from "./store.js";

export interface PublishCompletedCaptureInput {
  /** Host-observed SDK status; the manifest and bytes below remain the original materials document. */
  status: CaptureStatus;
  assembly: BundleAssemblyInput;
  ownerToken: string;
  publicationId: string;
  stagingRoot: string;
  destinationPath: string;
  derivatives?: PreparePublicationInput["derivatives"];
  derivatives2?: PreparePublicationInput["derivatives2"];
  derivative2Artifacts?: PreparePublicationInput["derivative2Artifacts"];
  /** Required only by createRecordPublicationOutput; persisted before publication dispatch. */
  outputRequest?: { requestId: string; requestVideoProof: boolean };
}

export interface ReconcileCompletedCaptureInput extends PublicationBindingExpectation {
  captureId: string;
  ownerToken: string;
}

export interface CompletedCapturePublisher {
  publishCompletedCapture(input: PublishCompletedCaptureInput): Promise<CaptureStatus>;
  reconcile(input: ReconcileCompletedCaptureInput): Promise<CaptureStatus>;
}

/** Compose Record assembly, private staging, and durable SDK publication recovery. */
export function createCompletedCapturePublisher(
  client: CaptureClient,
  privateControlRoot: string,
  preflight: PublicationPreflight,
): CompletedCapturePublisher {
  const recovery = createRecordPublicationRecoveryInternal(client, privateControlRoot, preflight);
  const bindings = new SqlitePublicationBindingStore(privateControlRoot);
  const inFlight = new Map<string, { fingerprint: string; promise: Promise<CaptureStatus> }>();

  return {
    async publishCompletedCapture(input) {
      const snapshot = structuredClone(input);
      validateObservedMaterials(snapshot);
      const fingerprint = createHash("sha256").update(JSON.stringify(digestBytes({
        status: snapshot.status,
        ownerTokenSha256: createHash("sha256").update(snapshot.ownerToken).digest("hex"),
        publicationId: snapshot.publicationId,
        stagingRoot: snapshot.stagingRoot,
        destinationPath: snapshot.destinationPath,
        assembly: snapshot.assembly,
        derivatives: snapshot.derivatives,
        derivatives2: snapshot.derivatives2,
        derivative2Artifacts: snapshot.derivative2Artifacts,
        outputRequest: snapshot.outputRequest,
      }))).digest("hex");
      const active = inFlight.get(snapshot.status.captureId);
      if (active) {
        if (active.fingerprint !== fingerprint) return Promise.reject(new Error("Record completed capture identity conflicts with its in-flight publication"));
        return active.promise.then(status => structuredClone(status));
      }
      const operation = publish(snapshot).finally(() => inFlight.delete(snapshot.status.captureId));
      inFlight.set(snapshot.status.captureId, { fingerprint, promise: operation });
      return operation.then(status => structuredClone(status));
    },
    async reconcile(input) {
      const snapshot = structuredClone(input);
      if (snapshot.captureId !== snapshot.materials.captureId) throw new Error("Record reconciliation capture identity does not match materials");
      const status = await recovery.reconcile(snapshot.captureId, snapshot.ownerToken, snapshot);
      return structuredClone(status);
    },
  };

  async function publish(input: PublishCompletedCaptureInput): Promise<CaptureStatus> {
    const assembly = assembleCanonicalBundle(input.assembly);
    // Existing ownership must be recovered using its original plan, never by
    // generating another staging namespace or guessing that dispatch failed.
    if (await bindings.get(input.status.captureId))
      throw new Error("Record capture is already bound to publication; reconcile the original binding instead of restaging");
    const prepared = await prepareRecordPublication({
      ownerToken: input.ownerToken,
      publicationId: input.publicationId,
      stagingRoot: input.stagingRoot,
      destinationPath: input.destinationPath,
      materialsManifest: input.assembly.manifest,
      materialsPayloadBytes: input.assembly.payloadBytes,
      assembly,
      artifactBytes: Object.fromEntries(input.assembly.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
      ...(input.derivatives ? { derivatives: input.derivatives } : {}),
      ...(input.derivatives2 ? { derivatives2: input.derivatives2 } : {}),
      ...(input.derivative2Artifacts ? { derivative2Artifacts: input.derivative2Artifacts } : {}),
    });
    const output = input.outputRequest ? { ...input.outputRequest, processing: assembly.processing } : undefined;
    return output ? recovery.publishWithOutput(prepared.input, output) : recovery.publish(prepared.input);
  }
}

/** Project binary values before JSON traversal; media size must not expand into decimal JSON keys. */
function digestBytes(value: unknown): unknown {
  if (value instanceof Uint8Array)
    return { byteLength: value.byteLength, sha256: createHash("sha256").update(value).digest("hex") };
  if (Array.isArray(value)) return value.map(digestBytes);
  if (value && typeof value === "object")
    return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([key, item]) => [key, digestBytes(item)]));
  return value;
}

function validateObservedMaterials(input: PublishCompletedCaptureInput): MaterialsDocument {
  if (input.status.state !== "materials-ready" || !input.status.materials)
    throw new Error("Record publication requires an observed materials-ready SDK status");
  const materials = validateMaterialsDocument(input.assembly.manifest, input.assembly.payloadBytes);
  if (input.outputRequest && (typeof input.outputRequest.requestId !== "string" || !input.outputRequest.requestId.trim() ||
      input.outputRequest.requestId.length > 4096 || input.outputRequest.requestId === input.status.captureId ||
      typeof input.outputRequest.requestVideoProof !== "boolean"))
    throw new Error("Record output request identity is invalid");
  if (input.status.captureId !== materials.manifest.key.captureId ||
      !isDeepStrictEqual(input.status.materials, materials.manifest) ||
      input.assembly.admission.captureId !== materials.manifest.key.captureId ||
      input.assembly.admission.materialsId !== materials.manifest.key.materialsId ||
      input.assembly.admission.bundleId !== materials.payload.bundleId ||
      input.destinationPath !== input.assembly.admission.bundlePath ||
      materials.payload.leaseId !== materials.manifest.leaseId)
    throw new Error("Record observed status, original materials, admission, or publication destination binding conflicts");
  return materials;
}
