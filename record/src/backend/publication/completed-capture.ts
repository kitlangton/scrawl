import { isDeepStrictEqual } from "node:util";
import { createHash } from "node:crypto";
import type { CaptureClient, CaptureStatus, MaterialsDocument } from "@apparent/engine-sdk/capture";
import { validateMaterialsDocument } from "@apparent/engine-sdk/capture";
import { assembleCanonicalBundle, type BundleAssemblyInput } from "../processing/bundle/index.js";
import { prepareRecordPublication } from "./staging.js";
import { createRecordPublicationRecovery, type PublicationBindingExpectation, type PublicationPreflight } from "./recovery.js";

export interface PublishCompletedCaptureInput {
  /** Host-observed SDK status; the manifest and bytes below remain the original materials document. */
  status: CaptureStatus;
  assembly: BundleAssemblyInput;
  ownerToken: string;
  publicationId: string;
  stagingRoot: string;
  destinationPath: string;
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
  const recovery = createRecordPublicationRecovery(client, privateControlRoot, preflight);
  const inFlight = new Map<string, { fingerprint: string; promise: Promise<CaptureStatus> }>();

  return {
    async publishCompletedCapture(input) {
      const snapshot = structuredClone(input);
      validateObservedMaterials(snapshot);
      const fingerprint = createHash("sha256").update(JSON.stringify({
        status: snapshot.status,
        ownerTokenSha256: createHash("sha256").update(snapshot.ownerToken).digest("hex"),
        publicationId: snapshot.publicationId,
        stagingRoot: snapshot.stagingRoot,
        destinationPath: snapshot.destinationPath,
        assembly: snapshot.assembly,
      })).digest("hex");
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
    const prepared = await prepareRecordPublication({
      ownerToken: input.ownerToken,
      publicationId: input.publicationId,
      stagingRoot: input.stagingRoot,
      destinationPath: input.destinationPath,
      materialsManifest: input.assembly.manifest,
      materialsPayloadBytes: input.assembly.payloadBytes,
      assembly,
      artifactBytes: Object.fromEntries(input.assembly.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    });
    return recovery.publish(prepared.input);
  }
}

function validateObservedMaterials(input: PublishCompletedCaptureInput): MaterialsDocument {
  if (input.status.state !== "materials-ready" || !input.status.materials)
    throw new Error("Record publication requires an observed materials-ready SDK status");
  const materials = validateMaterialsDocument(input.assembly.manifest, input.assembly.payloadBytes);
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
