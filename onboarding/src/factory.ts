import { createElement, Fragment, useEffect, useState, type ComponentType } from "react";
import type { ActivationReason, HostedContribution, CoachingHost } from "@apparent/app-contracts/host";
import type { RecordClient } from "@apparent/record/capabilities";
import { OnboardingController } from "./controller.js";
import type { ProgressStore } from "./model.js";
import { OnboardingContribution } from "./react.js";
import type { OnboardingContributionProps } from "./react.js";
import { PermissionRepairController, type RepairTimer } from "./permissions/controller.js";
import { PermissionRepairContribution } from "./permissions/react.js";

export type OnboardingHostedContribution = HostedContribution & {
  activation: HostedContribution["activation"] & {
    permissionRepair: { activate(requestedIds: readonly string[]): Promise<void> };
  };
};
export interface OnboardingContributionFactory {
  readonly ownerId: "onboarding";
  create(dependencies: OnboardingFactoryDependencies): Promise<OnboardingHostedContribution>;
}

export type OnboardingFactoryDependencies = Omit<OnboardingContributionProps, "controller"> & {
  progress: ProgressStore;
  record: RecordClient;
  id(): string;
  coaching?: CoachingHost<"onboarding-progress">;
  /** Narrow seam for owner tests; production uses the controller's default timer. */
  repairTimer?: RepairTimer;
  /** Reports the owner's current visible surface. This never opens or closes a host surface. */
  onSurfaceChange?(snapshot: OnboardingSurfaceSnapshot): void;
};

export type OnboardingSurfaceSnapshot = Readonly<{ kind: "hidden" | "tour" | "repair" }>;

// The host binds this owner factory and its narrow progress capability. It owns neither
// the Record client lifetime nor any native/window side effects.
export const onboardingContributionFactory: OnboardingContributionFactory = {
  ownerId: "onboarding",
  async create(deps): Promise<OnboardingHostedContribution> {
    const controller = new OnboardingController({ progress: deps.progress, record: deps.record, id: deps.id });
    const permissionRepair = new PermissionRepairController({ progress: deps.progress, record: deps.record, id: deps.id, timer: deps.repairTimer });
    type Surface = { kind: "hidden" | "tour" } | { kind: "repair"; returnTo: "hidden" | "tour"; session: number };
    let surface: Surface = { kind: "hidden" };
    let repairSession = 0, mountedScreens = 0;
    let closeError: string | null = null;
    let phase: "ready" | "dispose-failed" | "disposed" = "ready";
    const activationListeners = new Set<() => void>();
    const notify = () => { for (const listener of activationListeners) { try { listener(); } catch { activationListeners.delete(listener); } } };
    let observedKind: OnboardingSurfaceSnapshot["kind"] | undefined;
    const publishSurface = (force = false) => {
      if (force || observedKind !== surface.kind) {
        observedKind = surface.kind;
        try { deps.onSurfaceChange?.(Object.freeze({ kind: surface.kind })); } catch { /* Host observers cannot break owner lifecycle. */ }
      }
    };
    publishSurface(true);
    const setSurface = (next: Surface) => {
      surface = next;
      publishSurface();
    };
    const failedClose = () => { if (phase !== "ready") return; closeError = "Setup could not be closed. Please try again."; notify(); };
    let lifecycle = Promise.resolve();
    const serialize = <T>(operation: () => Promise<T>): Promise<T> => {
      const result = lifecycle.then(operation, operation);
      lifecycle = result.then(() => undefined, () => undefined);
      return result;
    };
    let previousRepairPhase = permissionRepair.state.phase;
    permissionRepair.subscribe(state => {
      const becameClosed = state.phase === "closed" && previousRepairPhase !== "closed";
      previousRepairPhase = state.phase;
      if (becameClosed && surface.kind === "repair" && mountedScreens > 0) void restoreAfterRepair(surface.session).catch(failedClose);
    });
    const Screen: ComponentType = () => {
      const [view, setView] = useState({ surface, closeError });
      useEffect(() => {
        mountedScreens++;
        const update = () => setView({ surface, closeError });
        activationListeners.add(update);
        if (phase === "ready" && surface.kind === "repair" && permissionRepair.state.phase === "closed") permissionRepair.activate(permissionRepair.state.requestedIds);
        update();
        return () => {
          activationListeners.delete(update); mountedScreens--;
          if (mountedScreens === 0 && surface.kind === "repair") {
            surface = { ...surface, session: ++repairSession };
            permissionRepair.cancel();
          }
        };
      }, []);
      const tour = createElement("div", { hidden: view.surface.kind !== "tour", "aria-hidden": view.surface.kind !== "tour" },
        createElement(OnboardingContribution, { controller, Button: deps.Button, close: () => { void closeTour().catch(failedClose); }, onManualPasteInstructions: deps.onManualPasteInstructions, loadOnMount: false }));
      const repair = view.surface.kind === "repair"
        ? createElement(PermissionRepairContribution, { controller: permissionRepair, requestedIds: permissionRepair.state.requestedIds, Button: deps.Button, close() {}, manageLifecycle: false })
        : null;
      const error = view.closeError ? createElement("div", { role: "alert" }, view.closeError, createElement(deps.Button, { onClick: () => {
        void (surface.kind === "repair" ? restoreAfterRepair(surface.session) : closeTour()).catch(failedClose);
      } }, "Try closing again")) : null;
      return createElement(Fragment, null, tour, repair, error);
    };
    const Settings: ComponentType = () => {
      const [busy, setBusy] = useState(false);
      const [error, setError] = useState<string | null>(null);
      const [status, setStatus] = useState(controller.state.progress.status);
      useEffect(() => controller.subscribe(state => setStatus(state.progress.status)), []);
      return createElement("section", { "aria-label": "Onboarding settings" },
        createElement("h2", null, "Setup"),
        createElement(deps.Button, { disabled: busy, onClick: () => {
          setBusy(true); setError(null);
          void contribution.activation.activate({ reason: "resume" }).catch(() => {
            setError("Setup could not be opened. Please try again.");
          }).finally(() => setBusy(false));
        } }, status === "completed" ? "Replay welcome tour" : "Resume setup"),
        error ? createElement("p", { role: "alert" }, error) : null);
    };
    const contribution: OnboardingHostedContribution = {
      module: { id: "onboarding", Screen, Settings },
      activation: {
        async activate({ reason }: { reason: ActivationReason }) {
          return serialize(async () => {
            if (phase !== "ready") throw new Error("Disposed onboarding contribution cannot be activated");
            if (surface.kind === "repair") throw new Error("Generic onboarding activation is unavailable during permission repair");
            await waitForProgress();
            await controller.load();
            await waitForProgress();
            const loaded = controller.state;
            if (loaded.publication === "unknown") throw new Error("Onboarding progress could not be loaded");
            if (reason === "firstLaunch" && (loaded.progress.status === "completed" || loaded.progress.status === "deferred")) {
              await hideCoach();
              setSurface({ kind: "hidden" }); closeError = null; notify();
              return;
            }
            if ((reason === "settings" || reason === "resume") && loaded.progress.status !== "in-progress") {
              const reopened = await controller.dispatch("reopen");
              if (reopened.publication !== "confirmed" || reopened.progress.status !== "in-progress") throw new Error("Onboarding could not be resumed safely");
            }
            setSurface({ kind: "tour" }); closeError = null; notify();
          });
        },
        permissionRepair: {
          async activate(requestedIds: readonly string[]) {
            const ids = [...new Set(requestedIds)];
            return serialize(async () => {
              if (phase !== "ready") throw new Error("Disposed onboarding contribution cannot be activated");
              await waitForProgress();
              const returnTo = surface.kind === "repair" ? surface.returnTo : surface.kind;
              const same = surface.kind === "repair" && permissionRepair.state.phase !== "closed"
                && ids.length === permissionRepair.state.requestedIds.length && ids.every(id => permissionRepair.state.requestedIds.includes(id));
              const session = same && surface.kind === "repair" ? surface.session : ++repairSession;
              permissionRepair.activate(ids);
              setSurface({ kind: "repair", returnTo, session }); closeError = null; notify();
            });
          },
        },
        async dispose() {
          return serialize(async () => {
            if (phase === "disposed") return;
            phase = "dispose-failed";
            ++repairSession;
            permissionRepair.dispose();
            controller.dispose();
            try { await hideCoach(); }
            catch (error) { phase = "dispose-failed"; throw error; }
            setSurface({ kind: "hidden" }); notify();
            phase = "disposed";
          });
        },
      },
    };
    async function waitForProgress(): Promise<void> {
      if (!controller.state.saving) return;
      await new Promise<void>(resolve => {
        const off = controller.subscribe(state => { if (!state.saving) { off(); resolve(); } });
        if (!controller.state.saving) { off(); resolve(); }
      });
    }
    async function hideCoach() {
      if (!deps.coaching) return;
      const result = await deps.coaching.hide("onboarding-progress");
      if (result.status === "unavailable") throw new Error("Onboarding coaching view could not be removed");
    }
    async function closeTour() {
      return serialize(async () => {
        if (phase !== "ready" || surface.kind !== "tour") return;
        await hideCoach();
        deps.close();
        setSurface({ kind: "hidden" }); closeError = null; notify();
      });
    }
    async function restoreAfterRepair(session: number) {
      return serialize(async () => {
        if (phase !== "ready" || surface.kind !== "repair" || surface.session !== session) return;
        if (surface.returnTo === "tour") {
          setSurface({ kind: "tour" });
        } else {
          deps.close();
          setSurface({ kind: "hidden" });
        }
        closeError = null; notify();
      });
    }
    return contribution;
  },
};
