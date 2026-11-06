import { useEffect, useRef, useState, type ComponentType, type ButtonHTMLAttributes } from "react";
import type { Command, ControllerState } from "./controller.js";
import { OnboardingController } from "./controller.js";
import type { PenColor, RepairAction } from "@apparent/record/capabilities";
import { permissionName, permissionStatus } from "./permissions/presentation.js";
const frog = new URL("./assets/frog.png", import.meta.url).href;
const appIcon = new URL("./assets/apparent-app-icon.svg", import.meta.url).href;
const penColor = (value: PenColor) => `var(--apparent-pen-${value})`;

export type OnboardingContributionProps = { controller: OnboardingController; Button: ComponentType<ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "default" | "ghost" | "outline"; size?: "default" | "icon" }>; close(): void; onManualPasteInstructions?(): void; loadOnMount?: boolean };
const pages: Record<string, { title: string; text: string; action?: Command; label?: string }> = {
  welcome: { title: "Welcome.", text: "Set up locally. Sign-in is not available yet.", action: "continue", label: "Get started" },
  pen: { title: "Pick your pen.", text: "" },
  permissions: { title: "We need access to record.", text: "Your operating system remains in charge of permission and recording readiness." },
  shortcut: { title: "Start and stop with your shortcut.", text: "Choose the keys you’ll use to start and finish recording." },
  "practice-start": { title: "Start your first capture.", text: "Press your shortcut to start. This practice stays on this device." },
  screen: { title: "Choose Screen.", text: "Choose Screen in the capture controls. Whiteboard becomes available once recording starts." },
  whiteboard: { title: "Open Whiteboard.", text: "Click Screen, then choose Whiteboard." },
  "board-ink": { title: "Draw on the whiteboard.", text: "Try a circle or an arrow. The guide moves on when you finish your mark." },
  "screen-ink": { title: "Draw on your screen.", text: "Hold Shift and circle something in your window." },
  stop: { title: "Finish your capture.", text: "Press your shortcut again. Your sketch and screen marks stay together." },
  preparing: { title: "Preparing your capture…", text: "Recording has stopped. Your marks and context are being brought together." },
  copied: { title: "Copied. Try pasting it.", text: "Your capture is copied. Paste it into your agent.", action: "continue", label: "Show paste steps" },
  paste: { title: "Paste your capture.", text: "Paste it where you want it, then confirm here. We never inspect the destination.", action: "manual-paste-confirm", label: "I pasted it" },
  locate: { title: "apparent lives here.", text: "Find the apparent icon in your menu bar or system tray. Click it, then choose Open Latest Capture." },
  complete: { title: "Ready for your next idea.", text: "You can record, draw, paste, and find your captures. Your setup is complete." },
};

export function OnboardingContribution({ controller, Button, close, onManualPasteInstructions, loadOnMount = true }: OnboardingContributionProps) {
  const [state, setState] = useState<ControllerState>(controller.state);
  const snapshot = state.snapshot;
  const [color, setColor] = useState<PenColor>(snapshot?.pen.effective ?? "coral");
  const [shortcutValue, setShortcutValue] = useState("");
  const [animation, setAnimation] = useState(false);
  const mounted = useRef(true), penArmed = useRef(false);
  useEffect(() => { mounted.current = true; let alive = true; const off = controller.subscribe(s => alive && setState(s)); if (loadOnMount) void controller.load().then(s => alive && setState(s)); return () => { alive = false; mounted.current = false; off(); }; }, [controller, loadOnMount]);
  const p = state.progress, page = pages[p.stage];
  useEffect(() => { if (snapshot?.pen.effective && !snapshot.pen.supported.includes(color)) setColor(snapshot.pen.effective); }, [snapshot?.pen.effective, snapshot?.pen.supported, color]);
  useEffect(() => {
    if (p.stage !== "pen" || p.status === "deferred" || penArmed.current || !controller.penSelectionConfirmed(color)) return;
    penArmed.current = true;
    if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) void controller.completePenSelection();
    else setAnimation(true);
  }, [controller, color, p.stage, p.status, snapshot, state.command]);
  async function closeSetup() { if (p.status === "completed") { close(); return; } const before = state.progress; const after = await controller.dispatch("defer"); if (after.progress.status === "deferred" && after.publication === "confirmed") close(); else if (after.progress !== before) setState(after); }
  async function choosePen() {
    if (snapshot?.pen.supported.includes(color)) await controller.selectPen(color);
  }
  const next = async () => { if (p.stage === "pen") await choosePen(); else await controller.dispatch(page.action ?? "continue"); };
  const isPractice = ["practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate"].includes(p.stage);
  const admission = snapshot?.nextShortcutAdmission;
  const practiceArmed = admission?.state === "armed" && admission.admissionId === p.admissionId && admission.delivery === "local-only";
  const requiredPermissionCount = snapshot?.readiness.requirements.filter(requirement => requirement.required).length ?? 0;
  const permissionTitle = requiredPermissionCount === 3 ? "We need three things." : "Review recording access.";
  const showFooter = !isPractice;
  return <main className="onboarding-shell" aria-labelledby="onboarding-title" data-stage={p.stage}>
    <header><span>apparent</span><Button variant="ghost" size="icon" aria-label="Close setup" onClick={() => void closeSetup()}>×</Button></header>
    <section aria-live="polite" className={isPractice ? "onboarding-practice" : "onboarding-setup"}>{p.stage === "welcome" ? <img className="onboarding-welcome-icon" src={appIcon} alt="apparent" /> : <div className="onboarding-brand"><img src={frog} alt="Frog guide" /></div>}{p.stage !== "welcome" && <p className="onboarding-eyebrow">{isPractice ? "Your first capture" : "A quick setup"}</p>}<h1 id="onboarding-title">{p.stage === "permissions" ? permissionTitle : page.title}</h1><p>{p.stage === "pen" ? "Choose a color for your pen." : page.text}</p>
      {p.status === "deferred" && <Button onClick={() => void controller.dispatch("reopen")}>Resume setup</Button>}
      {p.stage === "pen" && <><div className="onboarding-pencils" role="group" aria-label="Pen color">{(snapshot?.pen.supported ?? []).map(c => <Button key={c} variant="ghost" disabled={animation} aria-pressed={color === c} aria-label={`${c} pen`} onClick={() => { penArmed.current = false; setColor(c); }}><span aria-hidden="true" style={{ "--pencil": penColor(c) } as React.CSSProperties} /></Button>)}</div><div className="onboarding-pond" data-hop={animation}><svg viewBox="0 0 300 100" role="img" aria-label={`Puddle for ${color}`}><path d="M22 49C9 21 65 11 99 17C133 1 169 18 192 12C229 4 249 28 275 31C310 37 295 69 263 73C244 94 208 86 180 90C144 102 122 84 91 90C48 93 6 78 22 49Z" fill={penColor(color)} /></svg><img className="onboarding-pond-frog" src={frog} alt="Frog beside the puddle" onAnimationEnd={event => { if (event.animationName === "onboarding-pen-hop" && penArmed.current && mounted.current) { setAnimation(false); void controller.completePenSelection(); } }} /></div><p>{snapshot?.pen.error ?? `Selected color: ${snapshot?.pen.effective ?? "not yet confirmed"}`}</p></>}
      {p.stage === "shortcut" && <><p className="onboarding-fact">Effective shortcut: <kbd>{snapshot?.shortcut.label ?? snapshot?.shortcut.effective ?? "Not confirmed"}</kbd>{snapshot?.shortcut.registration === "externally-managed" && " · managed by the desktop"}</p>{snapshot?.shortcut.externallyManaged ? <><p>The desktop manages this shortcut. Apparent cannot change it here.</p><Button variant="outline" onClick={() => void controller.openDesktopShortcutConfig()}>Open desktop shortcut settings</Button></> : !snapshot?.shortcut.draft ? <Button variant="outline" onClick={() => void controller.beginShortcutEdit()}>Change shortcut</Button> : <><label>Shortcut draft<input value={shortcutValue || snapshot.shortcut.draft.value} onChange={e => setShortcutValue(e.target.value)} /></label><Button variant="outline" onClick={() => void controller.editShortcut(snapshot.shortcut.draft!.sessionId, shortcutValue || snapshot.shortcut.draft!.value)}>Use this shortcut</Button><Button variant="ghost" onClick={() => void controller.cancelShortcut(snapshot.shortcut.draft!.sessionId)}>Cancel</Button><Button onClick={() => void controller.saveShortcut(snapshot.shortcut.draft!.sessionId)}>Save shortcut</Button></>}</>}
      {p.stage === "permissions" && <><p className="onboarding-fact">{snapshot?.connection !== "connected" ? "Record is disconnected; readiness is unknown." : "Permission status and usable recording readiness are reported by the operating system."}</p>{snapshot?.readiness.requirements.map(r => <div className="onboarding-requirement" key={r.id}><strong>{permissionName(r.id)}</strong><span>{permissionStatus(r)}</span>{r.allowedRepairActions.includes("open-system-settings" as RepairAction) && <Button variant="outline" onClick={() => void controller.repair("open-system-settings", r.id)}>Open Settings</Button>}{r.allowedRepairActions.includes("request-permission" as RepairAction) && <Button variant="outline" onClick={() => void controller.repair("request-permission", r.id)}>Request access</Button>}{r.allowedRepairActions.includes("request-safe-relaunch" as RepairAction) && <Button variant="outline" onClick={() => void controller.safeRelaunch()}>Reopen Apparent</Button>}</div>)}<Button variant="outline" onClick={() => void controller.dispatch("refresh")}>Check access</Button></>}
      {p.stage === "copied" && <Button onClick={() => void controller.dispatch(page.action ?? "continue")}>{page.label}</Button>}
      {p.stage === "paste" && <><Button onClick={() => void controller.dispatch("manual-paste-confirm")}>I pasted it</Button><Button variant="outline" onClick={onManualPasteInstructions}>Show paste instructions</Button></>}
      {p.stage === "locate" && <p className="onboarding-waiting" role="status">Open the saved capture from the Apparent menu after confirming you pasted it.</p>}
      {p.stage === "preparing" && p.attemptId && snapshot?.captures.find(c => c.captureId === p.attemptId)?.phase === "saved" && !snapshot.captures.find(c => c.captureId === p.attemptId)?.outcomes.some(o => o.kind === "copy" && o.ok) && <Button variant="outline" onClick={() => void controller.retryPracticeCopy()}>Retry copy</Button>}
      {p.stage === "practice-start" && <><p className="onboarding-waiting" role="status">{practiceArmed ? `Ready when you are · ${snapshot?.shortcut.label ?? snapshot?.shortcut.effective}` : p.admissionId ? "Preparing your local practice…" : "The previous practice ended. Try again when you’re ready."}</p>{!p.admissionId && <Button onClick={() => void controller.dispatch("continue")}>Try again</Button>}</>}
      {p.attemptId && snapshot?.captures.find(c => c.captureId === p.attemptId)?.phase === "unknown" && <p className="onboarding-fact">This capture is not available yet. Reconnect and check again.</p>}
      {state.publication === "unknown" && <p role="status">Your setup changes may not have been saved. Reload before continuing.</p>}
      {state.publication === "unknown" && <Button variant="outline" onClick={() => void controller.dispatch("retry")}>Reload progress</Button>}
      {state.command === "unknown" && <Button variant="outline" onClick={() => void controller.dispatch("refresh")}>Check status</Button>}
      {state.error && <p role="alert">{state.error}</p>}
    </section>{showFooter && <footer><Button variant="ghost" onClick={() => void controller.dispatch("back")} disabled={state.saving || state.publication === "unknown" || p.stage === "welcome" || p.stage === "complete" || ["copied", "paste", "locate"].includes(p.stage)}>Back</Button><span>{p.status === "deferred" ? "Paused" : p.stage === "complete" ? "Complete" : "Your first capture"}</span><Button onClick={() => p.stage === "complete" ? void closeSetup() : void next()} disabled={state.saving || state.publication === "unknown" || (p.stage === "pen" && animation)}>{p.stage === "complete" ? "Finish setup" : p.stage === "pen" ? "Use this pen" : page.label ?? "Continue"}</Button></footer>}
    {p.stage !== "complete" && <Button className="onboarding-later" variant="ghost" onClick={() => void closeSetup()}>Try later</Button>}
  </main>;
}

export function createOnboardingContribution(props: OnboardingContributionProps) { return <OnboardingContribution {...props} />; }
