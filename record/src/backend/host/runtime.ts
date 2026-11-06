import { PrivateBundleAllocator } from "./bundle-allocator.js";
import { SqliteCaptureMappingStore } from "./capture-mapping-store.js";
import { EngineSdkAdapter, type EngineSdkPort } from "../engine/sdk-adapter.js";
import { RecordWorkflow, type RecordCommand, type RecordState } from "../workflow/controller.js";

export interface RecordHostRoots {
  mappingRoot: string;
  bundleRoot: string;
}

export type StartupIssue = {
  requestId: string;
  state: "active" | "unknown";
  phase?: string;
};

export type HostStartupState =
  | { state: "ready"; knownRequestIds: readonly string[] }
  | { state: "blocked"; knownRequestIds: readonly string[]; unresolved: readonly StartupIssue[]; storageUnavailable?: true };

export interface OwnershipReport {
  requestIds: readonly string[];
  storageUnavailable: boolean;
  reconciliation: "complete" | "pending" | "interrupted" | "failed";
}

export interface RecordHostRuntime {
  readonly startup: HostStartupState;
  readonly state: RecordState;
  subscribe(listener: (state: RecordState) => void): () => void;
  dispatch(command: RecordCommand): Promise<RecordState>;
  reconcileStartup(): Promise<HostStartupState>;
  unresolvedOwnership(): OwnershipReport;
  /** Detaches observation only; does not stop captures or close the shared SDK. */
  dispose(): OwnershipReport;
}

/** Structural runtime seam shared by the existing v1 runtime and host adapters. */
export interface RecordHostRuntimePort extends Pick<RecordHostRuntime,
  "startup" | "state" | "subscribe" | "dispatch" | "reconcileStartup" | "unresolvedOwnership" | "dispose"> {
  /** Optional adapter facts for runtimes that can retain multiple public requests. */
  captures?(): readonly RecordState[];
  completionEligibleCaptureIds?(): readonly string[];
  canStartCapture?(): boolean;
}
export type RecordStartOutcome = "accepted" | "refused" | "outcome-unknown";
export interface RecordCapture2StartPort {
  startCapture(requestId: string): Promise<{ state: RecordState; outcome: RecordStartOutcome }>;
}

/** Composes Record host storage around an SDK client constructed and owned by its caller. */
export async function createRecordHostRuntime(
  sdk: EngineSdkPort,
  roots: RecordHostRoots,
): Promise<RecordHostRuntime> {
  const mappings = new SqliteCaptureMappingStore(roots.mappingRoot);
  const allocator = new PrivateBundleAllocator(roots.bundleRoot);
  const adapter = new EngineSdkAdapter(sdk, { mappings, allocateBundle: id => allocator.allocate(id) });
  const workflow = new RecordWorkflow(adapter);
  let startup: HostStartupState = { state: "blocked", knownRequestIds: [], unresolved: [] };
  let disposed = false;
  let scan: Promise<HostStartupState> | undefined;
  let revision = 0;
  let commandState: "idle" | "dispatching" = "idle";
  let reconciliation: OwnershipReport["reconciliation"] = "complete";

  const freezeStartup = (value: HostStartupState): HostStartupState => Object.freeze(value.state === "ready"
    ? { state: "ready", knownRequestIds: Object.freeze([...value.knownRequestIds]) }
    : { state: "blocked", knownRequestIds: Object.freeze([...value.knownRequestIds]),
      unresolved: Object.freeze(value.unresolved.map(item => Object.freeze({ ...item }))),
      ...(value.storageUnavailable ? { storageUnavailable: true as const } : {}) });
  startup = freezeStartup(startup);

  const reconcileStartup = (): Promise<HostStartupState> => {
    if (disposed) return Promise.resolve(startup);
    if (scan) return scan;
    const scanRevision = ++revision;
    reconciliation = "pending";
    const work = (async (): Promise<HostStartupState> => {
      let requestIds: string[];
      try { requestIds = await mappings.listRequestIds("v1"); }
      catch {
        const blocked = freezeStartup({ state: "blocked", knownRequestIds: startup.knownRequestIds,
          unresolved: startup.knownRequestIds.map(requestId => ({ requestId, state: "unknown" })), storageUnavailable: true });
        if (!disposed && scanRevision === revision) { startup = blocked; reconciliation = "failed"; }
        return blocked;
      }
      if (disposed || scanRevision !== revision) return startup;
      // Publish discovered identities before awaiting inspection, and retain any
      // command reservation created since the database enumeration began.
      startup = freezeStartup({ ...startup, knownRequestIds: [...new Set([...startup.knownRequestIds, ...requestIds])].sort() });
      const unresolved: StartupIssue[] = [];
      for (const requestId of requestIds) {
        if (disposed || scanRevision !== revision) break;
        try {
          const receipt = await adapter.inspect(requestId);
          if (["starting", "recording", "processing"].includes(receipt.phase))
            unresolved.push({ requestId, state: "active", phase: receipt.phase });
        } catch {
          unresolved.push({ requestId, state: "unknown" });
        }
      }
      if (disposed || scanRevision !== revision) return startup;
      const knownRequestIds = [...new Set([...startup.knownRequestIds, ...requestIds])].sort();
      const result = freezeStartup(unresolved.length
        ? { state: "blocked", knownRequestIds, unresolved }
        : { state: "ready", knownRequestIds });
      if (!disposed && scanRevision === revision) startup = result;
      return result;
    })();
    scan = work;
    void work.finally(() => {
      if (scan === work) { scan = undefined; if (disposed) reconciliation = "interrupted"; else if (reconciliation === "pending") reconciliation = "complete"; }
    }).catch(() => undefined);
    return work;
  };

  await reconcileStartup();

  const unresolvedOwnership = (): OwnershipReport => {
    const ids = new Set(startup.state === "blocked" ? startup.unresolved.map(item => item.requestId) : []);
    if (reconciliation !== "complete") for (const id of startup.knownRequestIds) ids.add(id);
    const state = workflow.state;
    if ("captureId" in state && !["ready", "cancelled", "failed"].includes(state.phase)) ids.add(state.captureId);
    return Object.freeze({ requestIds: Object.freeze([...ids]), storageUnavailable: startup.state === "blocked" && startup.storageUnavailable === true, reconciliation });
  };

  return {
    get startup() { return freezeStartup(startup); },
    get state() { return workflow.state; },
    subscribe: listener => workflow.subscribe(listener),
    async dispatch(command) {
      if (disposed || commandState !== "idle") return workflow.state;
      if (command.type === "start") {
        if (scan || startup.state !== "ready" || !["idle", "ready", "cancelled", "failed"].includes(workflow.state.phase)) return workflow.state;
        if (startup.knownRequestIds.includes(command.requestId)) return workflow.state;
        // Reserve synchronously so concurrent commands cannot reuse an identity.
        startup = freezeStartup({ ...startup, knownRequestIds: [...startup.knownRequestIds, command.requestId].sort() });
      }
      // Startup gates fresh admission only. The workflow must retain stop/cancel
      // and reconciliation authority over captures it already owns during scans.
      commandState = "dispatching";
      try {
        const result = await workflow.dispatch(command);
        if (command.type === "start" && result.phase === "idle") {
          const mapping = await mappings.getByRequestId(command.requestId).catch(() => undefined);
          if (!mapping) startup = freezeStartup({ ...startup, knownRequestIds: startup.knownRequestIds.filter(id => id !== command.requestId) });
        }
        return result;
      } finally { commandState = "idle"; }
    },
    reconcileStartup,
    unresolvedOwnership,
    dispose() {
      if (!disposed) {
        if (reconciliation === "pending") {
          reconciliation = "interrupted";
          startup = freezeStartup({ state: "blocked", knownRequestIds: startup.knownRequestIds,
            unresolved: startup.knownRequestIds.map(requestId => ({ requestId, state: "unknown" })),
            ...(startup.state === "blocked" && startup.storageUnavailable ? { storageUnavailable: true } : {}) });
        }
        disposed = true;
        ++revision;
        workflow.dispose();
        adapter.dispose();
      }
      return unresolvedOwnership();
    },
  };
}
