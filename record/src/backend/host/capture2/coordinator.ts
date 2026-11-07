import { randomUUID } from "node:crypto";
import { lstat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { isDeepStrictEqual } from "node:util";
import type { CaptureClient, MaterialsDocument, MaterialsManifest, SourceDiscovery } from "@apparent/engine-sdk/capture";
import type { Media2Client, Media2JobRequest } from "@apparent/engine-sdk/media2";
import { preflightMedia2Dispatch } from "@apparent/engine-sdk/media2";
import { readFrameVisualObservations } from "@apparent/engine-sdk/observations";
import type { VisualSelectionSample } from "../../selection/visual-keyframes.js";
import { planStandardVisual, finishStandardVisual, type StandardVisualChoice, type StandardVisualPlan, type StandardVisualPreferences } from "./standard-visual.js";
import { createRecordCapabilityProvider } from "../../capabilities/provider.js";
import type { RecordCapabilityProvider, RecordCapabilityProviderOptions, RecordCaptureFactsPort } from "../../capabilities/provider.js";
import type { CaptureFact } from "../../../capabilities/index.js";
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
  selectProcessing(input: { mapping: Readonly<RecordCaptureCoordinatorMapping>; materials: MaterialsDocument }): Promise<RecordCaptureProcessingSelection | StandardVisualChoice> | RecordCaptureProcessingSelection | StandardVisualChoice;
  preflight: PublicationPreflight;
  pollIntervalMs?: number;
  operationTimeoutMs?: number;
}

export interface RecordCaptureProcessingIssue {
  requestId?: string;
  state: "failed" | "outcome-unknown" | "unavailable";
  code: "processing-failed" | "media-failed" | "media-cancelled" | "media-unavailable" | "media-outcome-unknown" |
    "visual-observations-unavailable" | "publication-failed" | "publication-cancelled" | "publication-outcome-unknown" | "startup-recovery-failed";
  at: string;
}

export type RecordCaptureShutdownWorkState = "active" | "pending" | "unknown" | "failed";
export interface RecordCaptureShutdownWork {
  owner: "capture" | "media" | "publication" | "coordinator" | "storage";
  state: RecordCaptureShutdownWorkState;
  requestId?: string;
  captureId?: string;
  publicationId?: string;
  reason: string;
}
export interface RecordCaptureCoordinatorShutdownReport {
  /** Clearance covers this coordinator, not other owners sharing the SDK/runtime. */
  scope: "record-coordinator";
  admission: "closed";
  localDrain: "complete";
  runtimeClearance: "clear" | "unresolved" | "unknown";
  /** This coordinator has no next-shortcut producer; unavailable does not mean idle. */
  nextShortcutAdmission: { state: "unsupported"; reason: string };
  work: readonly RecordCaptureShutdownWork[];
}

export interface RecordCaptureCoordinator {
  runtime: RecordHostRuntimePort & RecordCapture2StartPort;
  capture2Start: RecordCapture2StartPort;
  captureFacts: RecordCaptureFactsPort;
  resolveHandoff: RecordPublicationOutput["resolveHandoff"];
  /** Composes the provider directly with this coordinator's stable capture/fact ports. */
  createProvider(options: Omit<RecordCapabilityProviderOptions, "runtime" | "capture2Start" | "captureFacts">): RecordCapabilityProvider;
  /** Sanitized automatic outcomes; native job/publication journals remain authoritative. */
  processingIssues(): readonly RecordCaptureProcessingIssue[];
  observeProcessingIssues(listener: () => void): () => void;
  /** Explicitly reconciles capture, media and publication using only stored identities. */
  reconcile(): Promise<void>;
  /** Closes admission, drains local calls, then snapshots native/durable ownership without dispatching work. */
  shutdown(): Promise<RecordCaptureCoordinatorShutdownReport>;
  /** Detaches observers/timers; never cancels or closes caller-owned SDK operations. */
  dispose(): Promise<void>;
}

export interface StoredPlan {
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

export type StandardRecordCaptureCoordinatorOptions = Omit<RecordCaptureCoordinatorOptions, "selectProcessing"> & {
  standardProcessing: StandardVisualPreferences;
};
/** Record owns bounded sampling, visual selection and sheet layout; host still owns source/consent. */
export function createStandardRecordCaptureCoordinator(options: StandardRecordCaptureCoordinatorOptions): Promise<RecordCaptureCoordinator> {
  const preferences = structuredClone(options.standardProcessing);
  if (!preferences || typeof preferences.requestVideoProof !== "boolean" || (preferences.createdAtMs !== undefined && (!Number.isSafeInteger(preferences.createdAtMs) || preferences.createdAtMs < 0)))
    throw new TypeError("Standard processing preferences are invalid");
  return createRecordCaptureCoordinator({ ...options, selectProcessing: () => ({ ...preferences, kind: "standard-visual" }) });
}

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
  const projections = new Set<Promise<unknown>>();
  const operations = new Set<Promise<unknown>>();
  const pending = new Set<string>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let admissionClosed = false;
  let disposed = false;
  let shutdownPromise: Promise<RecordCaptureCoordinatorShutdownReport> | undefined;
  let startupRecovery: Promise<void> = Promise.resolve();
  const trackOperation = <T>(operation: Promise<T>): Promise<T> => {
    operations.add(operation);
    void operation.then(() => operations.delete(operation), () => operations.delete(operation));
    return operation;
  };
  const project = <T>(read: () => Promise<T>): Promise<T> => {
    if (admissionClosed || disposed) return Promise.reject(new Error("Record capture coordinator is closing"));
    const operation = read();
    projections.add(operation);
    void operation.then(() => projections.delete(operation), () => projections.delete(operation));
    return operation;
  };
  const issues = new Map<string, RecordCaptureProcessingIssue>();
  const listeners = new Set<() => void>();
  const notify = () => {
    if (disposed) return;
    for (const listener of [...listeners]) if (listeners.has(listener)) {
      try { listener(); } catch { listeners.delete(listener); }
    }
  };
  const report = (requestId: string | undefined, state: RecordCaptureProcessingIssue["state"], code: RecordCaptureProcessingIssue["code"]) => {
    if (disposed) return;
    const key = requestId ?? "";
    const previous = issues.get(key);
    if (previous?.state === state && previous.code === code) return;
    issues.set(key, { ...(requestId ? { requestId } : {}), state, code, at: new Date().toISOString() });
    notify();
  };
  const clearIssue = (requestId: string) => { if (issues.delete(requestId)) notify(); };
  const detachOutput = output.captureFacts.observe?.(notify) ?? (() => undefined);
  let factsRevision = 0;
  let factsFingerprint: string | undefined;
  const captureFacts: RecordCaptureFactsPort = {
    observe(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    readFacts() { return project(async () => {
      const facts = await output.captureFacts.readFacts();
      const captures = new Map(facts.captures.map(fact => [fact.captureId, fact]));
      for (const issue of issues.values()) if (issue.requestId && captures.get(issue.requestId)?.phase !== "saved") {
        const fact: CaptureFact = { captureId: issue.requestId, delivery: "unknown", phase: issue.state === "outcome-unknown" ? "unknown" : "failed",
          outcomes: [{ kind: "processing", ok: false, detail: issue.code, at: issue.at }] };
        captures.set(issue.requestId, fact);
      }
      const snapshot = { ...facts, captures: [...captures.values()] };
      const fingerprint = JSON.stringify(snapshot.captures);
      if (factsFingerprint !== undefined && factsFingerprint !== fingerprint) factsRevision++;
      factsFingerprint = fingerprint;
      return structuredClone({ ...snapshot, revision: factsRevision });
    }); },
  };


  const schedule = () => {
    if (admissionClosed || disposed || timer || !pending.size) return;
    timer = setTimeout(() => {
      timer = undefined;
      for (const requestId of [...pending]) void advance(requestId).catch(() => undefined);
      schedule();
    }, pollIntervalMs);
    timer.unref?.();
  };

  const subscriber = host.subscribe(event => {
    if (!admissionClosed && event.type === "capture-changed" && (event.receipt.phase === "materials-ready" || event.receipt.phase === "processing"))
      void advance(event.receipt.requestId).catch(() => undefined);
  });

  const advance = (requestId: string): Promise<void> => {
    const existing = flights.get(requestId);
    if (existing) return existing;
    if (admissionClosed || disposed || !requestId) return Promise.resolve();
    const work = run(requestId).catch(error => {
      if (!issues.has(requestId)) report(requestId, "failed", "processing-failed");
      throw error;
    }).finally(() => {
      flights.delete(requestId);
      schedule();
    });
    flights.set(requestId, work);
    return work;
  };

  const driveMedia = async (planRecord: ProcessingCoordinatorRecord, mapping: CaptureMapping, plan: Omit<StoredPlan, "request" | "frames">, request: Media2JobRequest, jobPath: string): Promise<RecordMedia2Job | undefined> => {
    const requestId = mapping.requestId;
    let materialsRead: Awaited<ReturnType<typeof readCaptureMaterials>> | undefined;
    let job: RecordMedia2Job;
    try { job = await RecordMedia2Job.reopenRetained(options.mediaClient, jobPath, mapping.ownerToken, {
      request: request, materials: planRecord.materials, leaseId: planRecord.leaseId, bundleId: planRecord.bundleId,
    }); }
    catch (error) {
      let missing = false;
      try { await lstat(jobPath); }
      catch (statError) { missing = !!statError && typeof statError === "object" && (statError as NodeJS.ErrnoException).code === "ENOENT"; }
      if (!missing) throw error;
      const manifest = plan.materialsManifest;
      materialsRead = await readCaptureMaterials({ manifest, expected: { key: planRecord.materials, leaseId: planRecord.leaseId, bundleId: planRecord.bundleId },
        trustedRoot: plan.trustedMaterialsRoot, limits: plan.materialsLimits });
      if (admissionClosed) { pending.add(requestId); return; }
      job = await RecordMedia2Job.create(options.mediaClient, jobPath, { ownerToken: mapping.ownerToken, request: request, materials: materialsRead.materials });
    }
    if (disposed) return;

    if (["ready", "submitted", "running", "cancelling", "unknown"].includes(job.state)) {
      const hello = await options.mediaClient.handshake(operationTimeoutMs);
      if (hello.implementation !== "native" || !hello.capabilities.retainedFrames || !hello.capabilities.nativeJobAuthority ||
          (request.contactSheet && !hello.capabilities.contactSheet))
        { report(requestId, "unavailable", "media-unavailable"); throw new Error("Native retained-media capabilities are unavailable; no media operation was dispatched"); }
      if (disposed) return;
    }
    if (["submitted", "running", "cancelling", "unknown"].includes(job.state)) {
      try { await job.inspect(mapping.ownerToken, operationTimeoutMs); } catch { report(requestId, "outcome-unknown", "media-outcome-unknown"); }
      if (["submitted", "running", "cancelling", "unknown"].includes(job.state)) { pending.add(requestId); return; }
    } else if (job.state === "ready") {
      if (admissionClosed) { pending.add(requestId); return; }
      try { await job.derive(mapping.ownerToken, operationTimeoutMs); } catch { report(requestId, job.state === "ready" || job.state === "unavailable" ? "unavailable" : "outcome-unknown",
        job.state === "ready" || job.state === "unavailable" ? "media-unavailable" : "media-outcome-unknown"); }
      if (["submitted", "running", "cancelling", "unknown"].includes(job.state)) {
        try { await job.inspect(mapping.ownerToken, operationTimeoutMs); } catch { report(requestId, "outcome-unknown", "media-outcome-unknown"); }
      }
      if (mediaJobState(job) !== "complete") {
        if (["submitted", "running", "cancelling", "unknown"].includes(job.state)) { pending.add(requestId); return; }
      }
    }
    if (job.state !== "complete" || !job.derivative) {
      if (job.state === "failed" || job.state === "cancelled") report(requestId, "failed", job.state === "failed" ? "media-failed" : "media-cancelled");
      return;
    }
    return job;
  };

  const run = async (requestId: string): Promise<void> => {
    pending.delete(requestId);
    const mapping = await mappings!.getByRequestId(requestId);
    if (disposed || !mapping?.capture2Intent) return;
    const publicIdentity = publicMapping(mapping);

    // Publication ownership is checked first. Saved handoff never rereads source materials.
    const binding = await store.get(mapping.captureId);
    if (disposed) return;
    if (binding) {
      if (binding.bundleId !== mapping.bundleId || binding.bundlePath !== mapping.bundlePath)
        throw new Error("Publication bundle identity conflicts with its capture mapping");
      if (binding.output && binding.output.requestId !== requestId) throw new Error("Saved output public request identity conflicts with its capture mapping");
      const saved = binding.state === "saved" && await output.resolveHandoff({ captureId: binding.output?.requestId ?? "", artifactId: binding.bundleId });
      if (saved) { clearIssue(requestId); return; }
      if (!/^[0-9a-f]{64}$/u.test(mapping.ownerToken)) return;
      if (admissionClosed) { pending.add(requestId); return; }
      try {
        await output.reconcile({ captureId: mapping.captureId, ownerToken: mapping.ownerToken, publicationId: binding.publicationId,
          materials: binding.materials, leaseId: binding.leaseId, bundleId: binding.bundleId, bundlePath: binding.bundlePath });
      } catch { report(requestId, "outcome-unknown", "publication-outcome-unknown"); }
      const after = await store.get(mapping.captureId);
      if (after?.state === "saved") { clearIssue(requestId); return; }
      if (after?.state === "failed" || after?.state === "cancelled") {
        report(requestId, "failed", after.state === "failed" ? "publication-failed" : "publication-cancelled"); return;
      }
      if (!after) return;
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
      const stored = planRecord.plan as StoredPlan | StandardVisualPlan;
      if (stored.schema === 2 && (stored.kind !== "standard-visual" || stored.policy !== "record-visual/1" || !Array.isArray(stored.samples) || stored.samples.length < 1 || stored.samples.length > 10))
        throw new Error("Unsupported persisted standard visual policy");
      const plan = stored.schema === 2 ? stored.base : stored;
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
      if ("kind" in selection && selection.kind === "standard-visual") {
        const admission: RecordAdmission = { createdAtMs: selection.createdAtMs ?? materialsRead.materials.payload.invocation.observedAtMs,
          ...(selection.destination ? { destination: selection.destination } : {}), target: confirmedTarget(materialsRead.materials), captureId: mapping.captureId,
          materialsId: currentManifest.key.materialsId, bundleId: mapping.bundleId, bundlePath: mapping.bundlePath };
        const base: Omit<StoredPlan, "request" | "frames"> = { schema: 1, trustedMaterialsRoot: trustedRoot, materialsManifest: structuredClone(currentManifest),
          materialsLimits: { maxFileBytes: 8 * 1024 * 1024, maxAggregateBytes: 20 * 1024 * 1024 },
          artifactLimits: { maxFileBytes: 64 * 1024 * 1024, maxAggregateBytes: 128 * 1024 * 1024 }, contactSheetPath: "contact-sheet.png", admission,
          marks: [], requestVideoProof: selection.requestVideoProof, publicationId: id(), stagingRoot: join(options.privateControlRoot, "publication-staging") };
        const standard = planStandardVisual(materialsRead.materials, base, options.privateControlRoot);
        for (const request of standard.samples) preflightMedia2Dispatch({ ownerToken: mapping.ownerToken, request, materials: materialsRead.materials }, operationTimeoutMs);
        planRecord = await store.reserveProcessing({ schema: 1, captureId: mapping.captureId, requestId, materials: structuredClone(currentManifest.key),
          leaseId: currentManifest.leaseId, bundleId: mapping.bundleId, bundlePath: mapping.bundlePath, plan: standard });
      } else {
        const explicit = selection as RecordCaptureProcessingSelection;
        const jobId = id();
        const request: Media2JobRequest = { schema: "apparent.retained-frame-request/1", materials: structuredClone(currentManifest.key), leaseId: currentManifest.leaseId,
          jobId, outputDirectory: explicit.request.outputDirectory, requestedBundleMs: [...explicit.request.requestedBundleMs], limits: structuredClone(explicit.request.limits),
          contactSheet: explicit.request.contactSheet && structuredClone(explicit.request.contactSheet) };
        validateSelection(explicit, request);
        preflightMedia2Dispatch({ ownerToken: mapping.ownerToken, request, materials: materialsRead.materials }, operationTimeoutMs);
        const target = confirmedTarget(materialsRead.materials);
        if (explicit.admission.target && !isDeepStrictEqual(explicit.admission.target, target)) throw new Error("Processing target conflicts with the SDK-confirmed native source geometry");
        const plan: StoredPlan = { schema: 1, trustedMaterialsRoot: trustedRoot, materialsManifest: structuredClone(currentManifest),
          materialsLimits: structuredClone(explicit.materialsLimits), artifactLimits: structuredClone(explicit.artifactLimits), request, frames: structuredClone(explicit.frames),
          contactSheetPath: explicit.contactSheetPath, admission: { ...structuredClone(explicit.admission), target, captureId: mapping.captureId, materialsId: currentManifest.key.materialsId,
            bundleId: mapping.bundleId, bundlePath: mapping.bundlePath }, marks: structuredClone(explicit.marks), requestVideoProof: explicit.requestVideoProof,
          ...(explicit.processingWarnings ? { processingWarnings: structuredClone(explicit.processingWarnings) } : {}),
          ...(explicit.additionalCanonicalFacts ? { additionalCanonicalFacts: structuredClone(explicit.additionalCanonicalFacts) } : {}),
          ...(explicit.transcriptBounds ? { transcriptBounds: structuredClone(explicit.transcriptBounds) } : {}),
          publicationId: id(), stagingRoot: join(options.privateControlRoot, "publication-staging") };
        planRecord = await store.reserveProcessing({ schema: 1, captureId: mapping.captureId, requestId, materials: structuredClone(currentManifest.key),
          leaseId: currentManifest.leaseId, bundleId: mapping.bundleId, bundlePath: mapping.bundlePath, plan });
        if (planRecord.requestId !== requestId || !equalMaterials(planRecord.materials, currentManifest.key)) throw new Error("Concurrent processing admission selected another immutable binding");
      }
    }
    if (disposed || !planRecord) return;
    const stored = planRecord.plan as StoredPlan | StandardVisualPlan;
    if (stored.schema === 2 && planRecord.visualSelection === undefined) {
      const base = stored.base;
      materialsRead ??= await readCaptureMaterials({ manifest: base.materialsManifest, expected: { key: planRecord.materials, leaseId: planRecord.leaseId, bundleId: planRecord.bundleId },
        trustedRoot: base.trustedMaterialsRoot, limits: base.materialsLimits });
      const samples: VisualSelectionSample[] = [];
      for (const request of stored.samples) {
        const sampled = await driveMedia(planRecord, mapping, base, request, join(options.privateControlRoot, "media2-jobs", `${mapping.captureId}-sample-${request.jobId}.sqlite`));
        if (!sampled?.derivative || disposed) return;
        const retained = await readRetainedDerivativeArtifacts({ document: sampled.derivative, materials: materialsRead.materials, trustedRoot: request.outputDirectory,
          mappings: request.requestedBundleMs.map((_, index) => ({ bundlePath: `sample-${index + 1}.png`, selector: { kind: "frame" as const, requestIndex: index + 1 } })), limits: base.artifactLimits });
        for (let index = 0; index < retained.artifacts.length; index++) {
          const observation = readFrameVisualObservations({ document: sampled.derivative, materials: materialsRead.materials, requestIndex: index + 1,
            pngBytes: retained.artifacts[index]!.bytes, maxFileBytes: base.artifactLimits.maxFileBytes });
          if (observation.state !== "measured") {
            report(requestId, "unavailable", "visual-observations-unavailable");
            throw new Error("Native measured visual observations are unavailable");
          }
          samples.push({ id: `${request.jobId}:${index + 1}`, timeMs: request.requestedBundleMs[index]!, width: observation.pixels.width,
            height: observation.pixels.height, thumbnail: [...observation.thumbnail.cells] });
        }
      }
      const selected = finishStandardVisual(stored, materialsRead.materials, samples);
      validateSelection(selected, selected.request);
      preflightMedia2Dispatch({ ownerToken: mapping.ownerToken, request: selected.request, materials: materialsRead.materials }, operationTimeoutMs);
      if (admissionClosed) { pending.add(requestId); return; }
      planRecord = await store.selectVisualProcessing(planRecord, selected);
    }
    const plan = (planRecord.visualSelection ?? planRecord.plan) as StoredPlan;
    if (stored.schema === 2 && (plan.schema !== 1 || !isDeepStrictEqual(plan.materialsManifest, stored.base.materialsManifest) ||
        plan.publicationId !== stored.base.publicationId || plan.request.jobId !== stored.finalJob.jobId || plan.request.outputDirectory !== stored.finalJob.outputDirectory))
      throw new Error("Final visual selection conflicts with the original persisted plan");
    const job = await driveMedia(planRecord, mapping, plan, plan.request, jobPath);
    if (!job?.derivative || disposed) return;
    clearIssue(requestId);

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
      if (admissionClosed) { pending.add(requestId); return; }
      const result = await output.publish({ status: sdkStatus, assembly: assemblyInput, ownerToken: mapping.ownerToken, publicationId: plan.publicationId,
        stagingRoot: plan.stagingRoot, destinationPath: mapping.bundlePath, derivatives2: [job.derivative], derivative2Artifacts: retained.derivative2Artifacts,
        requestId, requestVideoProof: plan.requestVideoProof });
      if (result.state === "saved") clearIssue(requestId);
      else if (result.state === "failed" || result.state === "cancelled") report(requestId, "failed", result.state === "failed" ? "publication-failed" : "publication-cancelled");
      else pending.add(requestId);
    } catch (error) {
      const durable = await store.get(mapping.captureId);
      if (durable && !["saved", "failed", "cancelled"].includes(durable.state)) {
        report(requestId, "outcome-unknown", "publication-outcome-unknown"); pending.add(requestId);
      }
      else throw error;
    }
  };

  const reconcile = async () => {
    if (disposed) return;
    await runtime.reconcileStartup();
    if (disposed) return;
    const ids = await mappings!.listRequestIds("capture2");
    if (!disposed) await Promise.all(ids.map(requestId => advance(requestId)));
  };

  // Startup recovery runs detached so callers can still issue Stop/Cancel while
  // retained media or publication RPCs are in flight. Explicit reconcile awaits it.
  startupRecovery = reconcile().catch(() => { if (!issues.size) report(undefined, "failed", "startup-recovery-failed"); });

  const shutdown = (): Promise<RecordCaptureCoordinatorShutdownReport> => {
    if (shutdownPromise) return shutdownPromise;
    // Close admission synchronously; already-started calls and request flights remain owned and are drained below.
    admissionClosed = true;
    if (timer) clearTimeout(timer);
    timer = undefined;
    shutdownPromise = (async () => {
      while (true) {
        const active = [...operations, ...flights.values(), ...projections, startupRecovery];
        await Promise.allSettled(active);
        if (!operations.size && !flights.size && !projections.size) break;
      }

      const work: RecordCaptureShutdownWork[] = [];
      const addWork = (item: RecordCaptureShutdownWork) => {
        const key = `${item.owner}\0${item.requestId ?? ""}\0${item.captureId ?? ""}\0${item.publicationId ?? ""}\0${item.reason}`;
        if (!work.some(value => `${value.owner}\0${value.requestId ?? ""}\0${value.captureId ?? ""}\0${value.publicationId ?? ""}\0${value.reason}` === key))
          work.push(Object.freeze(item));
      };
      try { await runtime.reconcileStartup(); }
      catch { addWork({ owner: "capture", state: "unknown", reason: "native-capture-reconciliation-failed-during-shutdown" }); }

      const ownership = runtime.unresolvedOwnership();
      const captureStates = new Map<string, string>();
      try {
        for (const state of runtime.captures?.() ?? []) {
          if (!("captureId" in state)) continue;
          captureStates.set(state.captureId, state.phase);
          if (["starting", "recording", "stopping", "cancelling", "stop-failed"].includes(state.phase))
            addWork({ owner: "capture", state: "active", requestId: state.captureId, reason: `native-capture-${state.phase}` });
          else if (state.phase === "uncertain")
            addWork({ owner: "capture", state: "unknown", requestId: state.captureId, reason: "native-capture-state-uncertain" });
        }
      } catch { addWork({ owner: "capture", state: "unknown", reason: "native-capture-snapshot-unavailable" }); }
      // Stop native observation before the remaining durable reads. Its cached
      // snapshot remains available, and no later poll can change this report.
      runtime.dispose();
      await runtime.drainObservation();
      host.dispose();
      if (ownership.storageUnavailable) addWork({ owner: "storage", state: "unknown", reason: "capture-mapping-storage-unavailable" });
      if (ownership.reconciliation !== "complete")
        addWork({ owner: "coordinator", state: "unknown", reason: `capture-startup-reconciliation-${ownership.reconciliation}` });
      for (const requestId of ownership.requestIds)
        if (!captureStates.has(requestId)) addWork({ owner: "capture", state: "unknown", requestId, reason: "native-capture-ownership-has-no-current-state" });

      let requestIds: string[] = [];
      const mappingsByRequest = new Map<string, CaptureMapping>();
      try {
        requestIds = await mappings!.listRequestIds("capture2");
        for (const requestId of requestIds) {
          try {
            const mapping = await mappings!.getByRequestId(requestId);
            if (!mapping?.capture2Intent) addWork({ owner: "storage", state: "unknown", requestId, reason: "capture-mapping-is-missing-or-not-capture2" });
            else mappingsByRequest.set(requestId, mapping);
            if (!captureStates.has(requestId)) addWork({ owner: "capture", state: "unknown", requestId, captureId: mapping?.captureId,
              reason: "durable-capture-mapping-has-no-native-inspection-result" });
          } catch { addWork({ owner: "storage", state: "unknown", requestId, reason: "capture-mapping-read-failed" }); }
        }
      } catch { addWork({ owner: "storage", state: "unknown", reason: "capture-mapping-list-failed" }); }

      let processingRecords: ProcessingCoordinatorRecord[] = [];
      try { processingRecords = await store.listProcessing(); }
      catch { addWork({ owner: "storage", state: "unknown", reason: "processing-plan-store-read-failed" }); }
      let publicationBindings: Awaited<ReturnType<SqlitePublicationBindingStore["list"]>> = [];
      try { publicationBindings = await store.list(); }
      catch { addWork({ owner: "storage", state: "unknown", reason: "publication-binding-store-read-failed" }); }
      const publicationsByCapture = new Map(publicationBindings.map(binding => [binding.captureId, binding]));

      for (const binding of publicationBindings) {
        const requestId = binding.output?.requestId ?? [...mappingsByRequest.values()].find(value => value.captureId === binding.captureId)?.requestId;
        if (binding.state === "reserved") addWork({ owner: "publication", state: "pending", requestId, captureId: binding.captureId,
          publicationId: binding.publicationId, reason: "publication-reserved-before-dispatch" });
        else if (binding.state === "dispatching") addWork({ owner: "publication", state: "unknown", requestId, captureId: binding.captureId,
          publicationId: binding.publicationId, reason: "publication-dispatch-may-have-reached-engine" });
        else if (binding.state === "publishing") addWork({ owner: "publication", state: "active", requestId, captureId: binding.captureId,
          publicationId: binding.publicationId, reason: "native-publication-remains-pending" });
      }

      const plansByCapture = new Map(processingRecords.map(record => [record.captureId, record]));
      for (const record of processingRecords) {
        const mapping = mappingsByRequest.get(record.requestId);
        if (!mapping || mapping.captureId !== record.captureId) {
          addWork({ owner: "storage", state: "unknown", requestId: record.requestId, captureId: record.captureId, reason: "processing-plan-has-no-matching-capture-mapping" });
          continue;
        }
        const stored = record.plan as StoredPlan | StandardVisualPlan;
        const selected = record.visualSelection as StoredPlan | undefined;
        const base = stored?.schema === 2 ? stored.base : stored;
        if (!base || base.schema !== 1 || !base.materialsManifest || (stored.schema !== 2 && !stored.request)) {
          addWork({ owner: "media", state: "unknown", requestId: record.requestId, captureId: record.captureId, reason: "persisted-processing-plan-is-invalid" });
          continue;
        }
        const jobs = stored.schema === 2 ? [
          ...stored.samples.map(request => ({ request, path: join(options.privateControlRoot, "media2-jobs", `${record.captureId}-sample-${request.jobId}.sqlite`), sample: true })),
          ...(selected ? [{ request: selected.request, path: join(options.privateControlRoot, "media2-jobs", `${record.captureId}.sqlite`), sample: false }] : []),
        ] : [{ request: stored.request, path: join(options.privateControlRoot, "media2-jobs", `${record.captureId}.sqlite`), sample: false }];
        for (const entry of jobs) {
          const jobPath = entry.path;
          try { await lstat(jobPath); }
          catch (error) {
            if ((error as NodeJS.ErrnoException).code === "ENOENT")
              addWork({ owner: "media", state: "pending", requestId: record.requestId, captureId: record.captureId, reason: "processing-plan-exists-without-a-retained-media-journal" });
            else addWork({ owner: "media", state: "unknown", requestId: record.requestId, captureId: record.captureId, reason: "retained-media-journal-stat-failed" });
            continue;
          }
          try {
            const job = await RecordMedia2Job.reopenRetained(options.mediaClient, jobPath, mapping.ownerToken, {
              request: entry.request, materials: record.materials, leaseId: record.leaseId, bundleId: record.bundleId,
            });
            if (["submitted", "running", "cancelling"].includes(job.state))
              addWork({ owner: "media", state: "active", requestId: record.requestId, captureId: record.captureId, reason: `retained-media-${job.state}-requires-native-inspection` });
            else if (job.state === "unknown")
              addWork({ owner: "media", state: "unknown", requestId: record.requestId, captureId: record.captureId, reason: "retained-media-dispatch-outcome-is-unknown" });
            else if (job.state === "ready" || job.state === "unavailable")
              addWork({ owner: "media", state: "pending", requestId: record.requestId, captureId: record.captureId, reason: `retained-media-${job.state}-has-no-dispatch-receipt` });
            else if (job.state === "complete" && !publicationsByCapture.has(record.captureId) && (!entry.sample || !selected))
              addWork({ owner: "publication", state: "pending", requestId: record.requestId, captureId: record.captureId, reason: "completed-media-has-no-durable-publication-binding" });
            else if (job.state === "failed" || job.state === "cancelled")
              addWork({ owner: "media", state: "failed", requestId: record.requestId, captureId: record.captureId, reason: `retained-media-${job.state}` });
          } catch { addWork({ owner: "media", state: "unknown", requestId: record.requestId, captureId: record.captureId, reason: "retained-media-journal-could-not-be-validated" }); }
      }
      }

      for (const requestId of pending) addWork({ owner: "coordinator", state: "pending", requestId, reason: "automatic-record-recovery-remains-pending" });
      for (const issue of issues.values()) {
        const mapping = issue.requestId ? mappingsByRequest.get(issue.requestId) : undefined;
        const binding = mapping ? publicationsByCapture.get(mapping.captureId) : undefined;
        // Durable reserved/publishing states respectively prove pre-dispatch
        // ownership or a confirmed pending native publication. A stale issue
        // cannot turn either known state into unknown ownership.
        if (issue.code === "publication-outcome-unknown" && (binding?.state === "reserved" || binding?.state === "publishing")) continue;
        addWork({ owner: "coordinator", state: issue.state === "failed" ? "failed" : issue.state === "unavailable" ? "pending" : "unknown",
          ...(issue.requestId ? { requestId: issue.requestId } : {}), reason: `record-processing-${issue.code}` });
      }
      for (const requestId of requestIds) {
        if (captureStates.get(requestId) === "processing" && !plansByCapture.has(mappingsByRequest.get(requestId)?.captureId ?? "") &&
            !publicationsByCapture.has(mappingsByRequest.get(requestId)?.captureId ?? ""))
          addWork({ owner: "coordinator", state: "pending", requestId, captureId: mappingsByRequest.get(requestId)?.captureId,
            reason: "materials-ready-capture-has-no-durable-processing-plan" });
      }

      const clearance: RecordCaptureCoordinatorShutdownReport["runtimeClearance"] = work.some(item => item.state === "unknown") ? "unknown"
        : work.some(item => item.state === "active" || item.state === "pending") ? "unresolved" : "clear";
      const snapshot = Object.freeze({ scope: "record-coordinator" as const, admission: "closed" as const, localDrain: "complete" as const, runtimeClearance: clearance,
        nextShortcutAdmission: Object.freeze({ state: "unsupported" as const, reason: "Capture/2 coordinator has no next-shortcut reservation producer" }),
        work: Object.freeze([...work]) });
      disposed = true;
      subscriber();
      detachOutput();
      listeners.clear();
      pending.clear();
      return snapshot;
    })();
    return shutdownPromise;
  };

  const coordinatedRuntime: RecordHostRuntimePort & RecordCapture2StartPort = {
    get startup() { return runtime.startup; },
    get state() { return runtime.state; },
    subscribe(listener) { return admissionClosed ? () => undefined : runtime.subscribe(listener); },
    dispatch(command) {
      if (admissionClosed) return Promise.reject(new Error("Record capture coordinator is closing"));
      return trackOperation(runtime.dispatch(command));
    },
    startCapture(requestId) {
      if (admissionClosed) return Promise.resolve({ state: runtime.state, outcome: "refused" });
      return trackOperation(runtime.startCapture(requestId));
    },
    reconcileStartup() {
      if (admissionClosed) return Promise.reject(new Error("Record capture coordinator is closing"));
      return trackOperation(runtime.reconcileStartup());
    },
    unresolvedOwnership() { return runtime.unresolvedOwnership(); },
    // Runtime consumers may detach independently; only the coordinator owner
    // closes admission through shutdown()/dispose().
    dispose() { return runtime.dispose(); },
    captures() { return runtime.captures?.() ?? []; },
    completionEligibleCaptureIds() { return runtime.completionEligibleCaptureIds?.() ?? []; },
    canStartCapture() { return !admissionClosed && (runtime.canStartCapture?.() ?? false); },
  };

  return {
    runtime: coordinatedRuntime,
    capture2Start: coordinatedRuntime,
    captureFacts,
    processingIssues: () => structuredClone([...issues.values()]),
    observeProcessingIssues(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    resolveHandoff: identity => project(() => output.resolveHandoff(structuredClone(identity))),
    createProvider(providerOptions) {
      if (admissionClosed) throw new Error("Record capture coordinator is closing");
      return createRecordCapabilityProvider({ ...providerOptions, runtime: coordinatedRuntime, capture2Start: coordinatedRuntime, captureFacts });
    },
    async reconcile() {
      if (admissionClosed) throw new Error("Record capture coordinator is closing");
      await trackOperation(reconcile());
    },
    shutdown,
    async dispose() { await shutdown(); },
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
