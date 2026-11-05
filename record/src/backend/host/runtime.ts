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

export interface RecordHostRuntime {
  readonly startup: HostStartupState;
  readonly state: RecordState;
  subscribe(listener: (state: RecordState) => void): () => void;
  dispatch(command: RecordCommand): Promise<RecordState>;
  reconcileStartup(): Promise<HostStartupState>;
  unresolvedOwnership(): { requestIds: readonly string[]; storageUnavailable: boolean };
  /** Detaches observation only; does not stop captures or close the shared SDK. */
  dispose(): { requestIds: readonly string[]; storageUnavailable: boolean };
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
    const work = (async (): Promise<HostStartupState> => {
      let requestIds: string[];
      try { requestIds = await mappings.listRequestIds(); }
      catch {
        const blocked = freezeStartup({ state: "blocked", knownRequestIds: startup.knownRequestIds, unresolved: [], storageUnavailable: true });
        if (!disposed && scanRevision === revision) startup = blocked;
        return blocked;
      }
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
      const result = freezeStartup(unresolved.length
        ? { state: "blocked", knownRequestIds: [...requestIds], unresolved }
        : { state: "ready", knownRequestIds: [...requestIds] });
      if (!disposed && scanRevision === revision) startup = result;
      return result;
    })();
    scan = work;
    void work.finally(() => { if (scan === work) scan = undefined; }).catch(() => undefined);
    return work;
  };

  await reconcileStartup();

  const unresolvedOwnership = (): { requestIds: readonly string[]; storageUnavailable: boolean } => {
    const ids = new Set(startup.state === "blocked" ? startup.unresolved.map(item => item.requestId) : []);
    const state = workflow.state;
    if ("captureId" in state && !["ready", "cancelled", "failed"].includes(state.phase)) ids.add(state.captureId);
    return Object.freeze({ requestIds: Object.freeze([...ids]), storageUnavailable: startup.state === "blocked" && startup.storageUnavailable === true });
  };

  return {
    get startup() { return freezeStartup(startup); },
    get state() { return workflow.state; },
    subscribe: listener => workflow.subscribe(listener),
    async dispatch(command) {
      if (disposed || scan || startup.state !== "ready") return workflow.state;
      if (command.type === "start") {
        if (startup.knownRequestIds.includes(command.requestId)) return workflow.state;
        // Reserve synchronously so concurrent commands cannot reuse an identity.
        startup = freezeStartup({ ...startup, knownRequestIds: [...startup.knownRequestIds, command.requestId].sort() });
        const result = await workflow.dispatch(command);
        if (result.phase === "idle") {
          const mapping = await mappings.getByRequestId(command.requestId).catch(() => undefined);
          if (!mapping) startup = freezeStartup({ ...startup, knownRequestIds: startup.knownRequestIds.filter(id => id !== command.requestId) });
        }
        return result;
      }
      return workflow.dispatch(command);
    },
    reconcileStartup,
    unresolvedOwnership,
    dispose() {
      if (!disposed) {
        disposed = true;
        ++revision;
        workflow.dispose();
        adapter.dispose();
      }
      return unresolvedOwnership();
    },
  };
}
