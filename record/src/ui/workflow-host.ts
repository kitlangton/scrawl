import { RecordWorkflow, type RecordState } from "../backend/workflow/controller.js";
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
    for (const observer of observers) {
      try { observer(snapshot); } catch { observers.delete(observer); }
    }
  });

  const host: PillHost = {
    manualDrag: false,
    read: async () => snapshot,
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
  return { revision, phase, clock: options.clock?.(state) ?? "", label, warning, notice, reducedMotion: false, draggable: false, inputLevel: null, noteCount: 0, actions };
}

const hostDisposers = new WeakMap<PillHost, () => void>();
