import { createElement, useEffect, useState, type ComponentType } from "react";
import type { ActivationReason, ContributionFactory, HostedContribution, CoachingHost } from "@apparent/app-contracts/host";
import type { RecordClient } from "@apparent/record/capabilities";
import { OnboardingController } from "./controller.js";
import type { ProgressStore } from "./model.js";
import { OnboardingContribution } from "./react.js";
import type { OnboardingContributionProps } from "./react.js";

export type OnboardingFactoryDependencies = Omit<OnboardingContributionProps, "controller"> & {
  progress: ProgressStore;
  record: RecordClient;
  id(): string;
  coaching?: CoachingHost<"onboarding-progress">;
};

// The host binds this owner factory and its narrow progress capability. It owns neither
// the Record client lifetime nor any native/window side effects.
export const onboardingContributionFactory: ContributionFactory<OnboardingFactoryDependencies> = {
  ownerId: "onboarding",
  async create(deps): Promise<HostedContribution> {
    const controller = new OnboardingController({ progress: deps.progress, record: deps.record, id: deps.id });
    let active = false;
    let phase: "ready" | "dispose-failed" | "disposed" = "ready";
    const activationListeners = new Set<(value: boolean) => void>();
    const setActive = (value: boolean) => { active = value; for (const listener of activationListeners) listener(value); };
    let lifecycle = Promise.resolve();
    const serialize = <T>(operation: () => Promise<T>): Promise<T> => {
      const result = lifecycle.then(operation, operation);
      lifecycle = result.then(() => undefined, () => undefined);
      return result;
    };
    const Screen: ComponentType = () => {
      const [visible, setVisible] = useState(active);
      useEffect(() => { activationListeners.add(setVisible); setVisible(active); return () => { activationListeners.delete(setVisible); }; }, []);
      return visible ? createElement(OnboardingContribution, { controller, Button: deps.Button, close: deps.close, onManualPasteInstructions: deps.onManualPasteInstructions, loadOnMount: false }) : null;
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
    const contribution: HostedContribution = {
      module: { id: "onboarding", Screen, Settings },
      activation: {
        async activate({ reason }: { reason: ActivationReason }) {
          return serialize(async () => {
            if (phase !== "ready") throw new Error("Disposed onboarding contribution cannot be activated");
            await waitForProgress();
            await controller.load();
            await waitForProgress();
            const loaded = controller.state;
            if (loaded.publication === "unknown") throw new Error("Onboarding progress could not be loaded");
            if (reason === "firstLaunch" && (loaded.progress.status === "completed" || loaded.progress.status === "deferred")) {
              setActive(false);
              await hideCoach();
              return;
            }
            if ((reason === "settings" || reason === "resume") && loaded.progress.status !== "in-progress") {
              const reopened = await controller.dispatch("reopen");
              if (reopened.publication !== "confirmed" || reopened.progress.status !== "in-progress") throw new Error("Onboarding could not be resumed safely");
            }
            setActive(true);
          });
        },
        async dispose() {
          return serialize(async () => {
            if (phase === "disposed") return;
            setActive(false);
            try { await hideCoach(); }
            catch (error) { phase = "dispose-failed"; throw error; }
            phase = "disposed";
            controller.dispose();
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
    return contribution;
  },
};
