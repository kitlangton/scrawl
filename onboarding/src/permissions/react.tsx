import { useEffect, useRef, useState, type ButtonHTMLAttributes, type ComponentType } from "react";
import type { RepairState } from "./controller.js";
import { PermissionRepairController } from "./controller.js";
import { permissionName, permissionStatus } from "./presentation.js";

export type PermissionRepairContributionProps = {
  controller: PermissionRepairController;
  requestedIds: readonly string[];
  Button: ComponentType<ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "default" | "ghost" | "outline"; size?: "default" | "icon" }>;
  close(): void;
  /** Owner factory controls suspend/resume; unmount must not imply a repair cancellation. */
  manageLifecycle?: boolean;
};

export function PermissionRepairContribution({ controller, requestedIds, Button, close, manageLifecycle = true }: PermissionRepairContributionProps) {
  const [state, setState] = useState<RepairState>(controller.state);
  const closeNotified = useRef(false);
  const requestKey = requestedIds.join("\0");
  useEffect(() => {
    const off = controller.subscribe(setState);
    controller.activate(requestedIds);
    setState(controller.state);
    return () => { off(); if (manageLifecycle) controller.cancel(); };
  }, [controller, requestKey, manageLifecycle]);
  useEffect(() => {
    if (state.phase !== "closed") closeNotified.current = false;
    else if (controller.state.phase === "closed" && !closeNotified.current) { closeNotified.current = true; close(); }
  }, [state.phase, close, controller]);

  const snapshot = state.snapshot;
  const disconnected = !snapshot || snapshot.connection !== "connected";
  return <main className="onboarding-shell permission-repair" aria-labelledby="permission-repair-title" data-repair-phase={state.phase}>
    <header><span>apparent</span><Button variant="ghost" size="icon" aria-label="Close access help" onClick={() => controller.close()}>×</Button></header>
    <section aria-live="polite" aria-busy={state.phase === "checking"}>
      <p className="onboarding-eyebrow">Access</p>
      <h1 id="permission-repair-title">Let’s get access working.</h1>
      <p>Your operating system controls access. Apparent will check each item you asked about.</p>
      {disconnected && <p className="onboarding-fact" role="status">Access status is unavailable. Reconnect and check again.</p>}
      {!disconnected && state.requestedIds.map(id => {
        const row = snapshot.readiness.requirements.find(item => item.id === id);
        const command = controller.availableAction(id);
        const action = command === "request-permission" ? "Request access" : command === "open-system-settings" ? "Open System Settings" : command === "request-safe-relaunch" ? "Reopen Apparent" : null;
        return <div className="permission-repair-row onboarding-requirement" key={id}>
          <strong>{permissionName(id)}</strong>
          <span>{permissionStatus(row)}</span>
          {!row && <span role="status">Access information is unavailable right now.</span>}
          {row && action && <Button variant="outline" disabled={state.pendingRequirementId === id} onClick={() => void controller.repair(id)}>{state.pendingRequirementId === id ? "Checking…" : action}</Button>}
        </div>;
      })}
      {state.phase === "checking" && <p role="status">Checking access…</p>}
      {state.phase === "ready" && <p role="status">Everything you asked about is ready.</p>}
      {state.error && <p role="alert">{state.error}</p>}
      <Button variant="outline" onClick={() => controller.refresh()} disabled={state.phase === "checking" || state.phase === "closed"}>Check access</Button>
    </section>
    <footer><Button variant="ghost" onClick={() => controller.cancel()}>Later</Button><span>Access help</span></footer>
  </main>;
}

/** Owner-specific activation seam for composition; Record has no onboarding policy dependency. */
export function createPermissionRepairContribution(props: PermissionRepairContributionProps) {
  return <PermissionRepairContribution {...props} />;
}
