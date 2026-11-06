import { randomUUID } from "node:crypto";
import { lstat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { isDeepStrictEqual } from "node:util";
import type { CaptureClient, MaterialsDocument, MaterialsManifest, SourceDiscovery } from "@apparent/engine-sdk/capture";
import type { Media2Client, Media2JobRequest } from "@apparent/engine-sdk/media2";
import { preflightMedia2Dispatch } from "@apparent/engine-sdk/media2";
import { createRecordCapabilityProvider } from "../../capabilities/provider.js";
import type { RecordCapabilityProvider, RecordCapabilityProviderOptions, RecordCaptureFactsPort } from "../../capabilities/provider.js";
import type { RecordLocalClipboardIdentity } from "../../handoff/clipboard.js";
import { createCapture2Host } from "./../capture2.js";
import type { Capture2MappingStore, Capture2StartIntent } from "../capture2.js";
import { createCapture2RecordRuntime } from "./runtime.js";
import type { RecordHostRuntimePort, RecordCapture2StartPort } from "../runtime.js";
import { SqliteCaptureMappingStore } from "../capture-mapping-store.js";
import { AbsentDestinationAllocator } from "../absent-destination-allocator.js";
import { readCaptureMaterials, type CaptureMaterialsLimits } from "../materials.js";
import type { CaptureMapping } from "../../engine/sdk-adapter.js";
import { RecordMedia2Job } from "../../processing/media2-consumer.js";
import type { RecordMedia2JobState } from "../../processing/media2-consumer.js";
import { readRetainedDerivativeArtifacts, type RetainedArtifactMapping } from "../../processing/retained-artifacts.js";
import type { BundleAssemblyInput, CanonicalKeyframeReason, RecordAdmission } from "../../processing/bundle/index.js";
import type { SelectedFrame, ProcessingWarning } from "../../processing/index.js";
import { createRecordPublicationOutput } from "../../publication/output.js";
import type { RecordPublicationOutput } from "../../publication/output.js";
import { SqlitePublicationBindingStore, type ProcessingCoordinatorRecord } from "../../publication/store.js";
import type { PublicationPreflight } from "../../publication/recovery.js";

export interface RecordCaptureCoordinatorMapping {
  requestId: string;
  captureId: string;
  bundleId: string;
  bundlePath: string;
}

export interface RecordCaptureFrameSelection extends Omit<SelectedFrame, "absolutePath"> {
  requestIndex: number;
  bundlePath: string;
  canonicalReason: CanonicalKeyframeReason;
}

/** Every output choice is explicit Record policy and becomes immutable before media dispatch. */
export interface RecordCaptureProcessingSelection {
  request: Pick<Media2JobRequest, "outputDirectory" | "requestedBundleMs" | "limits" | "contactSheet">;
  materialsLimits: Required<CaptureMaterialsLimits>;
  artifactLimits: { maxFileBytes: number; maxAggregateBytes: number };
  frames: RecordCaptureFrameSelection[];
  contactSheetPath: string;
  admission: Omit<RecordAdmission, "captureId" | "materialsId" | "bundleId" | "bundlePath" | "target"> & { target?: RecordAdmission["target"] };
  marks: unknown[];
  requestVideoProof: boolean;
  processingWarnings?: ProcessingWarning[];
  additionalCanonicalFacts?: BundleAssemblyInput["additionalCanonicalFacts"];
  transcriptBounds?: BundleAssemblyInput["transcriptBounds"];
}

export interface RecordCaptureCoordinatorOptions {
  captureClient: CaptureClient;
  mediaClient: Media2Client;
  mappings?: Capture2MappingStore;
  mappingRoot?: string;
  destinations?: AbsentDestinationAllocator;
  bundleRoot?: string;
  privateControlRoot: string;
  createStartIntent(requestId: string, discovery: SourceDiscovery): Omit<Capture2StartIntent, "requestId" | "destinationPath"> & { destinationPath: string };
  /** Separately host-trusted retained root; never inferred from a native manifest or receipt. */
  trustedMaterialsRoot(mapping: Readonly<RecordCaptureCoordinatorMapping>, manifest: MaterialsManifest): string;
  /** Invoked once for a new materials binding. Persisted decisions are reused after restart. */
  selectProcessing(input: { mapping: Readonly<RecordCaptureCoordinatorMapping>; materials: MaterialsDocument }): Promise<RecordCaptureProcessingSelection> | RecordCaptureProcessingSelection;
  preflight: PublicationPreflight;
  pollIntervalMs?: number;
  operationTimeoutMs?: number;
}

export interface RecordCaptureCoordinator {
  runtime: RecordHostRuntimePort & RecordCapture2StartPort;
  capture2Start: RecordCapture2StartPort;
  captureFacts: RecordCaptureFactsPort;
  resolveHandoff: RecordPublicationOutput["resolveHandoff"];
  /** Composes the provider directly with this coordinator's stable capture/fact ports. */
  createProvider(options: Omit<RecordCapabilityProviderOptions, "runtime" | "capture2Start" | "captureFacts">): RecordCapabilityProvider;
  /** Explicitly reconciles capture, media and publication using only stored identities. */
  reconcile(): Promise<void>;
  /** Detaches observers/timers; never cancels or closes caller-owned SDK operations. */
  dispose(): void;
}

interface StoredPlan {
  schema: 1;
  trustedMaterialsRoot: string;
  materialsManifest: MaterialsManifest;
  materialsLimits: Required<CaptureMaterialsLimits>;
  artifactLimits: { maxFileBytes: number; maxAggregateBytes: number };
  request: Media2JobRequest;
  frames: RecordCaptureFrameSelection[];
  contactSheetPath: string;
  admission: RecordAdmission;
  marks: unknown[];
  requestVideoProof: boolean;
  processingWarnings?: ProcessingWarning[];
  additionalCanonicalFacts?: BundleAssemblyInput["additionalCanonicalFacts"];
  transcriptBounds?: BundleAssemblyInput["transcriptBounds"];
  publicationId: string;
  stagingRoot: string;
}

const id = () => randomUUID().replaceAll("-", "");
const publicMapping = (value: CaptureMapping): RecordCaptureCoordinatorMapping => ({ requestId: value.requestId, captureId: value.captureId, bundleId: value.bundleId, bundlePath: value.bundlePath });
const equalMaterials = (a: MaterialsDocument["manifest"]["key"], b: MaterialsDocument["manifest"]["key"]) => a.captureId === b.captureId && a.materialsId === b.materialsId && a.revision === b.revision;

/** Capture/2 -> retained media/2 -> canonical Record publication coordinator. */
export async function createRecordCaptureCoordinator(options: RecordCaptureCoordinatorOptions): Promise<RecordCaptureCoordinator> {
  const mappings = options.mappings ?? (options.mappingRoot ? new SqliteCaptureMappingStore(options.mappingRoot) : undefined);
  const destinations = options.destinations ?? (options.bundleRoot ? new AbsentDestinationAllocator(options.privateControlRoot, options.bundleRoot) : undefined);
  if (!mappings || !destinations || typeof options.trustedMaterialsRoot !== "function" || typeof options.selectProcessing !== "function")
    throw new TypeError("Record capture coordinator requires capture stores, destination policy, trusted material roots, and explicit processing selection");
  const pollIntervalMs = options.pollIntervalMs ?? 1000;
  const operationTimeoutMs = options.operationTimeoutMs ?? 5000;
  if (!Number.isSafeInteger(pollIntervalMs) || pollIntervalMs < 100 || pollIntervalMs > 10000 || !Number.isSafeInteger(operationTimeoutMs) || operationTimeoutMs < 1 || operationTimeoutMs > 60000)
    throw new RangeError("Record coordinator timing limits are invalid");

  const host = createCapture2Host({ client: options.captureClient, mappings, destinations });
  const runtime = await createCapture2RecordRuntime({ host, createStartIntent: options.createStartIntent, pollIntervalMs });
  const output = createRecordPublicationOutput(options.captureClient, options.privateControlRoot, options.preflight);
  const store = new SqlitePublicationBindingStore(options.privateControlRoot);
  const flights = new Map<string, Promise<void>>();
  const pending = new Set<string>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;

  const schedule = () => {
    if (disposed || timer || !pending.size) return;
    timer = setTimeout(() => {
      timer = undefined;
      for (const requestId of [...pending]) void advance(requestId).catch(() => undefined);
      schedule();
    }, pollIntervalMs);
    timer.unref?.();
  };

  const subscriber = host.subscribe(event => {
    if (event.type === "capture-changed" && (event.receipt.phase === "materials-ready" || event.receipt.phase === "processing"))
      void advance(event.receipt.requestId).catch(() => undefined);
  });

  const advance = (requestId: string): Promise<void> => {
    if (disposed || !requestId) return Promise.resolve();
    const existing = flights.get(requestId);
    if (existing) return existing;
    const work = run(requestId).finally(() => {
      flights.delete(requestId);
      schedule();
    });
    flights.set(requestId, work);
    return work;
  };

  const run = async (requestId: string): Promise<void> => {
    pending.delete(requestId);
    const mapping = await mappings!.getByRequestId(requestId);
    if (!mapping?.capture2Intent) return;
    const publicIdentity = publicMapping(mapping);

    // Publication ownership is checked first. Saved handoff never rereads source materials.
    const binding = await store.get(mapping.captureId);
    if (binding) {
      if (binding.output && binding.output.requestId !== requestId) throw new Error("Saved output public request identity conflicts with its capture mapping");
      const saved = binding.state === "saved" && await output.resolveHandoff({ captureId: binding.output?.requestId ?? "", artifactId: binding.bundleId });
      if (saved) return;
      if (!/^[0-9a-f]{64}$/u.test(mapping.ownerToken)) return;
      try {
        await output.reconcile({ captureId: mapping.captureId, ownerToken: mapping.ownerToken, publicationId: binding.publicationId,
          materials: binding.materials, leaseId: binding.leaseId, bundleId: binding.bundleId, bundlePath: binding.bundlePath });
      } catch { /* unresolved publication keeps the original binding and is inspected again */ }
      const after = await store.get(mapping.captureId);
      if (!after || after.state === "saved" || ["failed", "cancelled"].includes(after.state)) return;
      pending.add(requestId);
      return;
    }

    let planRecord = await store.getProcessing(mapping.captureId);
    let materialsRead: Awaited<ReturnType<typeof readCaptureMaterials>> | undefined;
    const jobPath = join(options.privateControlRoot, "media2-jobs", `${mapping.captureId}.sqlite`);
    let sdkStatus: Awaited<ReturnType<CaptureClient["inspect"]>> | undefined;
    if (planRecord) {
      if (planRecord.requestId !== requestId || planRecord.materials.captureId !== mapping.captureId || planRecord.bundlePath !== mapping.bundlePath ||
          planRecord.bundleId !== mapping.bundleId) throw new Error("Capture coordinator policy conflicts with the durable material binding");
      const plan = planRecord.plan as StoredPlan;
      if (!plan || plan.schema !== 1 || plan.materialsManifest.root !== plan.trustedMaterialsRoot || !equalMaterials(planRecord.materials, plan.materialsManifest.key) ||
          planRecord.leaseId !== plan.materialsManifest.leaseId)
        throw new Error("Capture coordinator materials changed from the immutable processing plan");
    } else {
      sdkStatus = await options.captureClient.inspect({ captureId: mapping.captureId, ownerToken: mapping.ownerToken });
      if (disposed || sdkStatus.captureId !== mapping.captureId || sdkStatus.state !== "materials-ready" || !sdkStatus.materials) return;
      const currentManifest = sdkStatus.materials;
      if (currentManifest.key.captureId !== mapping.captureId) throw new Error("Capture materials do not match the persisted Record mapping");
      const trustedRoot = options.trustedMaterialsRoot(publicIdentity, currentManifest);
      materialsRead = await readCaptureMaterials({ manifest: currentManifest, expected: { key: currentManifest.key, leaseId: currentManifest.leaseId, bundleId: mapping.bundleId }, trustedRoot });
      const selection = structuredClone(await options.selectProcessing({ mapping: publicIdentity, materials: materialsRead.materials }));
      const jobId = id();
      const request: Media2JobRequest = { schema: "apparent.retained-frame-request/1", materials: structuredClone(currentManifest.key), leaseId: currentManifest.leaseId,
        jobId, outputDirectory: selection.request.outputDirectory, requestedBundleMs: [...selection.request.requestedBundleMs], limits: structuredClone(selection.request.limits),
        contactSheet: selection.request.contactSheet && structuredClone(selection.request.contactSheet) };
      validateSelection(selection, request);
      preflightMedia2Dispatch({ ownerToken: mapping.ownerToken, request, materials: materialsRead.materials }, operationTimeoutMs);
      const target = confirmedTarget(materialsRead.materials);
      if (selection.admission.target && !isDeepStrictEqual(selection.admission.target, target)) throw new Error("Processing target conflicts with the SDK-confirmed native source geometry");
      const plan: StoredPlan = { schema: 1, trustedMaterialsRoot: trustedRoot, materialsManifest: structuredClone(currentManifest),
        materialsLimits: structuredClone(selection.materialsLimits), artifactLimits: structuredClone(selection.artifactLimits), request, frames: structuredClone(selection.frames),
        contactSheetPath: selection.contactSheetPath, admission: { ...structuredClone(selection.admission), target, captureId: mapping.captureId, materialsId: currentManifest.key.materialsId,
          bundleId: mapping.bundleId, bundlePath: mapping.bundlePath }, marks: structuredClone(selection.marks), requestVideoProof: selection.requestVideoProof,
        ...(selection.processingWarnings ? { processingWarnings: structuredClone(selection.processingWarnings) } : {}),
        ...(selection.additionalCanonicalFacts ? { additionalCanonicalFacts: structuredClone(selection.additionalCanonicalFacts) } : {}),
        ...(selection.transcriptBounds ? { transcriptBounds: structuredClone(selection.transcriptBounds) } : {}),
        publicationId: id(), stagingRoot: join(options.privateControlRoot, "publication-staging") };
      planRecord = await store.reserveProcessing({ schema: 1, captureId: mapping.captureId, requestId, materials: structuredClone(currentManifest.key),
        leaseId: currentManifest.leaseId, bundleId: mapping.bundleId, bundlePath: mapping.bundlePath, plan });
      if (planRecord.requestId !== requestId || !equalMaterials(planRecord.materials, currentManifest.key)) throw new Error("Concurrent processing admission selected another immutable binding");
    }
    if (disposed || !planRecord) return;
    const plan = planRecord.plan as StoredPlan;
    let job: RecordMedia2Job;
    try { job = await RecordMedia2Job.reopenRetained(options.mediaClient, jobPath, mapping.ownerToken); }
    catch (error) {
      let missing = false;
      try { await lstat(jobPath); }
      catch (statError) { missing = !!statError && typeof statError === "object" && (statError as NodeJS.ErrnoException).code === "ENOENT"; }
      if (!missing) throw error;
      const manifest = plan.materialsManifest;
      materialsRead = await readCaptureMaterials({ manifest, expected: { key: planRecord.materials, leaseId: planRecord.leaseId, bundleId: planRecord.bundleId },
        trustedRoot: plan.trustedMaterialsRoot, limits: plan.materialsLimits });
      job = await RecordMedia2Job.create(options.mediaClient, jobPath, { ownerToken: mapping.ownerToken, request: plan.request, materials: materialsRead.materials });
    }
    if (disposed) return;

    if (["submitted", "running", "cancelling", "unknown"].includes(job.state)) {
      try { await job.inspect(mapping.ownerToken, operationTimeoutMs); } catch { /* same original job remains durable and unknown */ }
      if (["submitted", "running", "cancelling", "unknown"].includes(job.state)) { pending.add(requestId); return; }
    } else if (job.state === "ready") {
      try { await job.derive(mapping.ownerToken, operationTimeoutMs); } catch { /* preflight stays undispatched; ambiguous outcomes stay inspect-only */ }
      if (["submitted", "running", "cancelling", "unknown"].includes(job.state)) {
        try { await job.inspect(mapping.ownerToken, operationTimeoutMs); } catch { /* same original job remains unresolved */ }
      }
      if (mediaJobState(job) !== "complete") {
        if (["submitted", "running", "cancelling", "unknown"].includes(job.state)) pending.add(requestId);
        return;
      }
    }
    if (job.state !== "complete" || !job.derivative) return;

    sdkStatus = await options.captureClient.inspect({ captureId: mapping.captureId, ownerToken: mapping.ownerToken });
    if (sdkStatus.state !== "materials-ready" || !sdkStatus.materials || !isDeepStrictEqual(sdkStatus.materials, plan.materialsManifest))
      throw new Error("Capture materials are no longer in the original materials-ready state");

    if (!materialsRead) {
      materialsRead = await readCaptureMaterials({ manifest: plan.materialsManifest, expected: { key: planRecord.materials, leaseId: planRecord.leaseId, bundleId: planRecord.bundleId },
        trustedRoot: plan.trustedMaterialsRoot, limits: plan.materialsLimits });
    }

    const mappingsForRead: RetainedArtifactMapping[] = [
      ...plan.frames.map(frame => ({ bundlePath: frame.bundlePath, selector: { kind: "frame" as const, requestIndex: frame.requestIndex } })),
      { bundlePath: plan.contactSheetPath, selector: { kind: "contact-sheet" as const } },
    ];
    const retained = await readRetainedDerivativeArtifacts({ document: job.derivative, materials: materialsRead.materials,
      trustedRoot: plan.request.outputDirectory, mappings: mappingsForRead, limits: plan.artifactLimits });
    if (disposed) return;
    const assemblyInput: BundleAssemblyInput = {
      manifest: materialsRead.materials.manifest, payloadBytes: materialsRead.payloadBytes, facts: materialsRead.assemblyFacts,
      admission: plan.admission,
      frames: plan.frames.map(frame => ({ index: frame.index, timeMs: frame.timeMs, absolutePath: resolve(mapping.bundlePath, frame.bundlePath), text: frame.text, event: frame.event, reason: frame.reason,
        screenName: frame.screenName, markId: frame.markId, artifactPath: frame.bundlePath, canonicalReason: frame.canonicalReason })),
      marks: structuredClone(plan.marks), artifacts: retained.artifacts,
      materialReferences: [{ role: "video", relativePath: materialsRead.materials.payload.video.file },
        ...(materialsRead.materials.payload.audio ? [{ role: "audio" as const, relativePath: materialsRead.materials.payload.audio.file }] : [])],
      ...(plan.processingWarnings ? { processingWarnings: plan.processingWarnings } : {}),
      ...(plan.additionalCanonicalFacts ? { additionalCanonicalFacts: plan.additionalCanonicalFacts } : {}),
      ...(plan.transcriptBounds ? { transcriptBounds: plan.transcriptBounds } : {}),
    };
    if (disposed) return;
    try {
      const result = await output.publish({ status: sdkStatus, assembly: assemblyInput, ownerToken: mapping.ownerToken, publicationId: plan.publicationId,
        stagingRoot: plan.stagingRoot, destinationPath: mapping.bundlePath, derivatives2: [job.derivative], derivative2Artifacts: retained.derivative2Artifacts,
        requestId, requestVideoProof: plan.requestVideoProof });
      if (result.state !== "saved") pending.add(requestId);
    } catch (error) {
      const durable = await store.get(mapping.captureId);
      if (durable && !["saved", "failed", "cancelled"].includes(durable.state)) pending.add(requestId);
      else throw error;
    }
  };

  const reconcile = async () => {
    if (disposed) return;
    await runtime.reconcileStartup();
    const ids = await mappings!.listRequestIds("capture2");
    if (!disposed) await Promise.all(ids.map(requestId => advance(requestId)));
  };

  // Startup recovery runs detached so callers can still issue Stop/Cancel while
  // retained media or publication RPCs are in flight. Explicit reconcile awaits it.
  void reconcile().catch(() => undefined);

  return {
    runtime,
    capture2Start: runtime,
    captureFacts: output.captureFacts,
    resolveHandoff: output.resolveHandoff,
    createProvider(providerOptions) { return createRecordCapabilityProvider({ ...providerOptions, runtime, capture2Start: runtime, captureFacts: output.captureFacts }); },
    async reconcile() { await reconcile(); },
    dispose() {
      if (disposed) return;
      disposed = true;
      if (timer) clearTimeout(timer);
      timer = undefined;
      pending.clear();
      subscriber();
      runtime.dispose();
      host.dispose();
    },
  };
}

function validateSelection(selection: RecordCaptureProcessingSelection, request: Media2JobRequest): void {
  if (!selection || !Array.isArray(selection.frames) || selection.frames.length === 0 || !selection.contactSheetPath || !Array.isArray(selection.marks) ||
      typeof selection.requestVideoProof !== "boolean" || !Number.isSafeInteger(selection.admission.createdAtMs) || selection.admission.createdAtMs < 0 ||
      !selection.materialsLimits || !selection.artifactLimits || !Number.isSafeInteger(selection.artifactLimits.maxFileBytes) || selection.artifactLimits.maxFileBytes < 1 ||
      !Number.isSafeInteger(selection.artifactLimits.maxAggregateBytes) || selection.artifactLimits.maxAggregateBytes < selection.artifactLimits.maxFileBytes ||
      request.outputDirectory.length === 0 || !request.contactSheet || request.requestedBundleMs.length === 0 || request.requestedBundleMs.length > request.limits.maxFrames)
    throw new TypeError("Record processing selection must explicitly provide frames, sheet layout, limits, output and persisted admission time");
  const frameIndexes = new Set<number>(), bundlePaths = new Set<string>();
  for (const frame of selection.frames) {
    if (!Number.isSafeInteger(frame.requestIndex) || frame.requestIndex < 1 || frame.requestIndex > request.requestedBundleMs.length ||
        request.requestedBundleMs[frame.requestIndex - 1] !== frame.timeMs || frameIndexes.has(frame.requestIndex) || bundlePaths.has(frame.bundlePath))
      throw new Error("Selected processing frames must retain their exact unique SDK request slots and final paths");
    frameIndexes.add(frame.requestIndex); bundlePaths.add(frame.bundlePath);
  }
  if (bundlePaths.has(selection.contactSheetPath)) throw new Error("Contact sheet path conflicts with a selected frame path");
  if (selection.materialsLimits.maxFileBytes === undefined || selection.materialsLimits.maxAggregateBytes === undefined ||
      !Number.isSafeInteger(selection.materialsLimits.maxFileBytes) || !Number.isSafeInteger(selection.materialsLimits.maxAggregateBytes))
    throw new Error("Capture material byte bounds must be explicit and finite");
}

function mediaJobState(job: RecordMedia2Job): RecordMedia2JobState { return job.state; }

function confirmedTarget(materials: MaterialsDocument): RecordAdmission["target"] {
  const source = materials.payload.confirmedSource;
  if (source.kind !== "display" && source.kind !== "area") throw new Error("SDK-confirmed source cannot be represented by Record target metadata");
  return {
    mode: source.kind === "display" ? "display" : "region",
    displayId: source.displayId,
    frame: { ...source.rectangle },
  };
}
