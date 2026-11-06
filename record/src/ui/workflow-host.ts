import { RecordWorkflow, type RecordState } from "../backend/workflow/controller.js";
import type { CapabilitySnapshot, RecordClient, CaptureFact } from "../capabilities/index.js";
import type { PillHost, PillView } from "./types.js";

export type WorkflowPillHostOptions = {
  /** Supplies collision-resistant request IDs from the embedding host. */
  createRequestId: () => string;
  /** Optional host clock string. No timer or clock is started by this adapter. */
  clock?: (state: RecordState) => string;
};

/** Injects the authoritative RecordWorkflow into the existing, consumer-neutral pill API. */
export function createWorkflowPillHost(workflow: RecordWorkflow, options: WorkflowPillHostOptions): PillHost {
  let revision = 0;
  let snapshot = present(workflow.state, ++revision, options);
  let disposed = false;
  const observers = new Set<(view: PillView) => void>();
  const unsubscribeWorkflow = workflow.subscribe(state => {
    if (disposed) return;
    snapshot = present(state, ++revision, options);
    const published = snapshot;
    for (const observer of observers) {
      try { observer(structuredClone(published)); } catch { observers.delete(observer); }
    }
  });

  const host: PillHost = {
    manualDrag: false,
    read: async () => structuredClone(snapshot),
    layout: async () => {},
    beginDrag: async () => {},
    beginPaste: async () => {},
    cancelPaste: async () => {},
    subscribe: callback => {
      if (!disposed) observers.add(callback);
      return () => observers.delete(callback);
    },
    act: async (expectedRevision, action) => {
      if (disposed || expectedRevision !== snapshot.revision) return;
      const current = workflow.state;
      if (action === "refresh" && current.phase === "idle") {
        await workflow.dispatch({ type: "refresh-readiness" });
      } else if (action === "record" && isStartable(current)) {
        await workflow.dispatch({ type: "start", requestId: options.createRequestId() });
      } else if (action === "finish" && current.phase === "recording") {
        await workflow.dispatch({ type: "stop" });
      } else if (action === "cancel" && (current.phase === "starting" || current.phase === "recording")) {
        await workflow.dispatch({ type: "cancel" });
      } else if (action === "inspect" && (current.phase === "uncertain" || current.phase === "stop-failed")) {
        await workflow.dispatch({ type: "reconcile" });
      } else if (action === "retry-stop" && current.phase === "stop-failed") {
        await workflow.dispatch({ type: "retry-stop" });
      }
    },
  };
  hostDisposers.set(host, () => { disposed = true; unsubscribeWorkflow(); observers.clear(); });
  return host;
}

/** Detaches only this presentation adapter; it deliberately does not dispose/stop workflow capture. */
export function disposeWorkflowPillHost(host: PillHost): void {
  hostDisposers.get(host)?.();
  hostDisposers.delete(host);
}

export type RecordClientPillHostOptions = {
  createRequestId: () => string;
  clock?: (snapshot: CapabilitySnapshot, capture?: CaptureFact) => string;
};

/** Binds the existing pill to the public, revisioned RecordClient contract. */
export function createRecordClientPillHost(client: RecordClient, options: RecordClientPillHostOptions): PillHost {
  let snapshot: CapabilitySnapshot | undefined;
  const retiredEpochs = new Set<string>();
  let revision = 0;
  let disposed = false;
  let view: PillView | null = null;
  const observers = new Set<(value: PillView) => void>();
  const publish = (next: CapabilitySnapshot) => {
    if (disposed || retiredEpochs.has(next.providerEpoch) || (snapshot && next.providerEpoch === snapshot.providerEpoch && next.revision < snapshot.revision)) return;
    if (snapshot && next.providerEpoch !== snapshot.providerEpoch) retiredEpochs.add(snapshot.providerEpoch);
    snapshot = structuredClone(next);
    view = presentClient(snapshot, ++revision, options);
    for (const observer of [...observers]) try { observer(structuredClone(view)); } catch { observers.delete(observer); }
  };
  const unsubscribe = client.observe(publish);
  const host: PillHost = {
    manualDrag: false, read: async () => view && structuredClone(view), layout: async () => {}, beginDrag: async () => {},
    beginPaste: async () => {}, cancelPaste: async () => {},
    subscribe: callback => { if (!disposed) observers.add(callback); return () => observers.delete(callback); },
    act: async (expectedRevision, action) => {
      const observed = snapshot;
      const currentView = view;
      if (disposed || !observed || !currentView || expectedRevision !== currentView.revision) return;
      const base = { requestId: options.createRequestId(), expected: { providerEpoch: observed.providerEpoch, revision: observed.revision } };
      let command: import("../capabilities/index.js").RecordCommand | undefined;
      const owned = observed.captures.find(item => item.captureId === currentViewCaptureId(observed));
      if (action === "refresh") command = { ...base, type: "refresh" };
      else if (action === "record" && canStart(observed)) command = { ...base, type: "start-capture", delivery: "host-default" };
      else if (owned && action === "finish" && owned.phase === "recording") command = { ...base, type: "stop-capture", captureId: owned.captureId };
      else if (owned && action === "cancel" && ["starting", "recording"].includes(owned.phase)) command = { ...base, type: "cancel-capture", captureId: owned.captureId };
      else if (owned && action === "inspect" && ["unknown", "failed"].includes(owned.phase)) command = { ...base, type: "inspect-capture", captureId: owned.captureId };
      if (command) await client.execute(command);
    },
  };
  hostDisposers.set(host, () => { disposed = true; unsubscribe(); observers.clear(); });
  return host;

  function currentViewCaptureId(value: CapabilitySnapshot): string | undefined {
    return activeCapture(value)?.captureId ?? value.captures.filter(item => item.phase === "saved").at(-1)?.captureId;
  }
}

function activeCapture(value: CapabilitySnapshot): CaptureFact | undefined {
  const active = value.captures.filter(item => ["starting", "recording", "processing", "unknown"].includes(item.phase));
  return active.length === 1 ? active[0] : undefined;
}
function canStart(value: CapabilitySnapshot): boolean {
  return value.connection === "connected" && value.readiness.requirements.every(item => !item.required || (item.usableNow && !item.relaunchNeeded)) &&
    !value.captures.some(item => ["starting", "recording", "processing", "unknown"].includes(item.phase));
}
function presentClient(value: CapabilitySnapshot, revision: number, options: RecordClientPillHostOptions): PillView {
  const active = activeCapture(value);
  const hasAmbiguousOwnership = value.captures.filter(item => ["starting", "recording", "processing", "unknown"].includes(item.phase)).length > 1;
  const saved = !active && !hasAmbiguousOwnership ? [...value.captures].reverse().find(item => item.phase === "saved" && item.artifactId) : undefined;
  const failed = !active && !hasAmbiguousOwnership ? [...value.captures].reverse().find(item => item.phase === "failed") : undefined;
  const state: RecordState = active ? active.phase === "starting" ? { phase: "starting", captureId: active.captureId, readiness: { ready: true } }
    : active.phase === "recording" ? { phase: "recording", captureId: active.captureId, readiness: { ready: true } }
      : active.phase === "processing" ? { phase: "processing", captureId: active.captureId, readiness: { ready: true } }
        : { phase: "uncertain", captureId: active.captureId, readiness: { ready: false, reason: "Capture state unknown" }, message: "Capture state unknown" }
    : failed ? { phase: "failed", captureId: failed.captureId, readiness: { ready: true }, message: "Capture failed" }
      : { phase: "idle", readiness: { ready: canStart(value), reason: value.readiness.requirements.find(item => item.required && (!item.usableNow || item.relaunchNeeded))?.id ?? "Record capture unavailable" } };
  const result = present(state, revision, { createRequestId: () => "", clock: () => options.clock?.(value, active ?? saved ?? failed) ?? "" });
  if (hasAmbiguousOwnership) {
    result.phase = "selecting"; result.label = "Capture ownership needs reconciliation"; result.status = result.label;
    result.warning = "Multiple captures are active or unresolved";
    result.actions = [{ id: "refresh", label: "Check capture", enabled: true, selected: false, primary: true }];
  }
  if (saved) { result.phase = "completed"; result.label = "Recording saved"; result.actions = [{ id: "record", label: "Record again", enabled: canStart(value), selected: false, primary: true }]; }
  if (active?.phase === "unknown") result.actions = [{ id: "inspect", label: "Check status", enabled: true, selected: false, primary: true }];
  if (saved) result.notice = saved.artifactId ?? null;
  if (active?.delivery === "unknown" || active?.delivery === "local-only")
    result.warning = result.warning ? `${result.warning}; delivery status unavailable` : "Delivery status unavailable";
  return result;
}

function isStartable(state: RecordState): boolean {
  return state.phase === "idle" || state.phase === "ready" || state.phase === "cancelled" || state.phase === "failed";
}

function present(state: RecordState, revision: number, options: WorkflowPillHostOptions): PillView {
  const actions: PillView["actions"] = [];
  let phase: PillView["phase"] = "selecting";
  let label = "Select capture";
  let warning: string | null = null;
  let notice: string | null = null;
  if (state.phase === "idle") {
    label = state.readiness.ready ? "Ready to record" : "Capture unavailable";
    if (!state.readiness.ready) {
      warning = state.readiness.reason;
      actions.push({ id: "refresh", label: "Check capture", enabled: true, selected: false, primary: true });
    } else actions.push({ id: "record", label: "Record", enabled: true, selected: false, primary: true });
  } else if (state.phase === "checking" || state.phase === "starting") {
    label = state.phase === "checking" ? "Checking capture readiness…" : "Starting capture…";
    if (state.phase === "starting") actions.push({ id: "cancel", label: "Cancel", enabled: true, selected: false, primary: false });
  } else if (state.phase === "recording") {
    phase = "recording"; label = "Recording";
    actions.push({ id: "finish", label: "Finish", enabled: true, selected: false, primary: true }, { id: "cancel", label: "Cancel", enabled: true, selected: false, primary: false });
  } else if (state.phase === "processing" || state.phase === "stopping" || state.phase === "cancelling") {
    phase = "processing"; label = state.phase === "processing" ? "Processing recording…" : state.phase === "stopping" ? "Finishing recording…" : "Cancelling capture…";
  } else if (state.phase === "ready") {
    phase = "completed"; label = "Recording saved"; notice = state.bundlePath;
    actions.push({ id: "record", label: "Record again", enabled: true, selected: false, primary: true });
  } else if (state.phase === "cancelled") {
    label = "Capture cancelled"; actions.push({ id: "record", label: "Record", enabled: true, selected: false, primary: true });
  } else if (state.phase === "stop-failed" || state.phase === "uncertain") {
    label = state.phase === "stop-failed" ? "Finish could not be confirmed" : state.phase === "uncertain" ? "Capture status unknown" : "Capture failed";
    warning = state.message;
    if (state.phase === "stop-failed") actions.push({ id: "retry-stop", label: "Retry finish", enabled: true, selected: false, primary: true });
    actions.push({ id: "inspect", label: "Check status", enabled: true, selected: false, primary: false });
  } else if (state.phase === "failed") {
    label = "Capture failed";
    warning = state.message;
    actions.push({ id: "record", label: "Try again", enabled: true, selected: false, primary: true });
  }
  const pocket: PillView["pocket"] = phase === "selecting" && warning
    ? { tone: state.phase === "failed" ? "failure" : warning ? "warning" : "info", title: label, description: warning ?? "", actionIds: [] }
    : undefined;
  let clock = "";
  try { clock = options.clock?.(state) ?? ""; } catch { /* Host decoration cannot interrupt authoritative updates. */ }
  return { revision, phase, clock, label, status: phase === "selecting" ? label : undefined, warning, notice, pocket, reducedMotion: false, draggable: false, inputLevel: null, noteCount: 0, actions };
}

const hostDisposers = new WeakMap<PillHost, () => void>();
