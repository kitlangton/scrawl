import { randomUUID } from "node:crypto";
import type { CaptureClient } from "@apparent/engine-sdk/capture";
import type { CaptureFact } from "../../capabilities/index.js";
import type { RecordCaptureFacts, RecordCaptureFactsPort } from "../capabilities/provider.js";
import type { RecordLocalClipboardIdentity, RecordLocalHandoffResolution } from "../handoff/clipboard.js";
import { SqlitePublicationBindingStore } from "./store.js";
import { createCompletedCapturePublisher, type CompletedCapturePublisher, type PublishCompletedCaptureInput } from "./completed-capture.js";
import type { PublicationPreflight } from "./recovery.js";

export type RecordPublicationOutputInput = Omit<PublishCompletedCaptureInput, "outputRequest"> & {
  requestId: string;
  requestVideoProof: boolean;
};

export interface RecordPublicationOutput {
  publish(input: RecordPublicationOutputInput): ReturnType<CompletedCapturePublisher["publishCompletedCapture"]>;
  reconcile(input: Parameters<CompletedCapturePublisher["reconcile"]>[0]): ReturnType<CompletedCapturePublisher["reconcile"]>;
  captureFacts: RecordCaptureFactsPort;
  resolveHandoff(identity: Readonly<RecordLocalClipboardIdentity>): Promise<RecordLocalHandoffResolution | null>;
}

/** Durable, authenticated Saved projection over the existing publication binding journal. */
export function createRecordPublicationOutput(client: CaptureClient, privateControlRoot: string, preflight: PublicationPreflight): RecordPublicationOutput {
  const publisher = createCompletedCapturePublisher(client, privateControlRoot, preflight);
  const bindings = new SqlitePublicationBindingStore(privateControlRoot);
  const providerEpoch = randomUUID();
  let revision = 0;
  let fingerprint = "";
  let factsInitialized = false;
  const listeners = new Set<() => void>();
  const notify = () => {
    for (const listener of [...listeners]) if (listeners.has(listener)) {
      try { listener(); } catch { listeners.delete(listener); }
    }
  };

  const readSaved = async () => (await bindings.list()).filter(binding => binding.state === "saved" && binding.output && binding.savedReceipt &&
    binding.savedReceipt.artifactProvenance === "native" && binding.savedReceipt.artifactVersion === 1 &&
    binding.savedReceipt.captureId === binding.captureId && binding.savedReceipt.publicationId === binding.publicationId &&
    binding.savedReceipt.planSha256 === binding.planSha256 && binding.savedReceipt.bundleId === binding.bundleId &&
    binding.savedReceipt.bundlePath === binding.bundlePath && binding.savedReceipt.manifestSha256 === binding.manifestSha256 &&
    JSON.stringify(binding.savedReceipt.materials) === JSON.stringify(binding.materials));

  return {
    async publish(input) {
      const { requestId, requestVideoProof, ...capture } = structuredClone(input);
      try { return await publisher.publishCompletedCapture({ ...capture, outputRequest: { requestId, requestVideoProof } }); }
      finally { notify(); }
    },
    async reconcile(input) {
      try { return await publisher.reconcile(structuredClone(input)); }
      finally { notify(); }
    },
    captureFacts: {
      observe(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
      async readFacts(): Promise<RecordCaptureFacts> {
        const saved = await readSaved();
        const captures: CaptureFact[] = saved.map(binding => ({ captureId: binding.output!.requestId, artifactId: binding.bundleId, delivery: "unknown", phase: "saved",
          outcomes: [{ kind: "save", ok: true, at: binding.savedObservedAt! }] }));
        const nextFingerprint = JSON.stringify(captures);
        if (factsInitialized && fingerprint !== nextFingerprint) revision++;
        fingerprint = nextFingerprint;
        factsInitialized = true;
        return structuredClone({ version: 1, providerEpoch, revision, captures, nextShortcutAdmission: { state: "unavailable" as const } });
      },
    },
    async resolveHandoff(identity) {
      if (!identity || typeof identity.captureId !== "string" || typeof identity.artifactId !== "string") return null;
      const matches = (await readSaved()).filter(binding => binding.output!.requestId === identity.captureId && binding.bundleId === identity.artifactId);
      if (matches.length !== 1) return null;
      const binding = matches[0]!;
      return structuredClone({ captureId: identity.captureId, artifactId: identity.artifactId, handoff: {
        savedBundle: { status: "saved" as const, bundlePath: binding.bundlePath },
        processing: binding.output!.processing,
        requestVideoProof: binding.output!.requestVideoProof,
      } });
    },
  };
}
