import { useEffect, useRef, useState, type ComponentType, type ButtonHTMLAttributes } from "react";
import type { Command, ControllerState } from "./controller.js";
import { OnboardingController } from "./controller.js";
import type { RepairAction } from "@apparent/record/capabilities";
const frog = new URL("./assets/frog.png", import.meta.url).href;
const penColor = (value: string) => ["coral", "purple", "blue", "green"].includes(value) ? `var(--apparent-pen-${value})` : "var(--apparent-muted)";

export type OnboardingContributionProps = { controller: OnboardingController; Button: ComponentType<ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "default" | "ghost" | "outline"; size?: "default" | "icon" }>; close(): void; onManualPasteInstructions?(): void };
const pages: Record<string, { title: string; text: string; action?: Command; label?: string }> = {
  welcome: { title: "Welcome.", text: "Set up locally. No sign-in is needed.", action: "continue", label: "Choose a pen" },
  pen: { title: "Pick your pen.", text: "" },
  permissions: { title: "We need access to record.", text: "Your operating system remains in charge of permission and recording readiness." },
  shortcut: { title: "Start and stop with your shortcut.", text: "Use the effective shortcut shown by Record. A draft key choice is not active until the host confirms it." },
  "practice-start": { title: "Start your first capture.", text: "Press the effective shortcut to start. Menu starts do not count for this practice." },
  screen: { title: "Choose Screen.", text: "Choose Screen from Record's capture controls." },
  whiteboard: { title: "Open Whiteboard.", text: "Choose Whiteboard from the capture controls." },
  "board-ink": { title: "Draw on the whiteboard.", text: "Make a mark. The guide advances only after Record confirms saved ink." },
  "screen-ink": { title: "Draw on your screen.", text: "Hold Shift and draw a mark. Saved strokes are confirmed by Record." },
  stop: { title: "Finish your capture.", text: "Press the shortcut again. Your screen and drawing stay together." },
  preparing: { title: "Preparing your capture…", text: "Record is finishing the bundle. This can take a moment." },
  copied: { title: "Copied. Try pasting it.", text: "A successful copy is ready to paste. A saved file by itself is not a copy.", action: "continue", label: "Show paste steps" },
  paste: { title: "Paste your capture.", text: "Paste into your own chosen destination, then confirm here. Apparent does not inspect private composers.", action: "manual-paste-confirm", label: "I pasted it" },
  locate: { title: "Find your capture.", text: "Open the matching practice artifact from Apparent. If the host cannot prove it opened, this step stays pending." },
  complete: { title: "Ready for your next idea.", text: "Setup is complete. Your practice capture was saved and handed off." },
};

export function OnboardingContribution({ controller, Button, close, onManualPasteInstructions }: OnboardingContributionProps) {
  const [state, setState] = useState<ControllerState>(controller.state);
  const snapshot = state.snapshot;
  const [color, setColor] = useState(snapshot?.pen.effective ?? "coral");
  const [shortcutValue, setShortcutValue] = useState("");
  const [animation, setAnimation] = useState(false);
  const mounted = useRef(true), penArmed = useRef(false);
  useEffect(() => { mounted.current = true; let alive = true; const off = controller.subscribe(s => alive && setState(s)); void controller.load().then(s => alive && setState(s)); return () => { alive = false; mounted.current = false; off(); }; }, [controller]);
  const p = state.progress, page = pages[p.stage];
  useEffect(() => { if (snapshot?.pen.effective && !snapshot.pen.supported.includes(color)) setColor(snapshot.pen.effective); }, [snapshot?.pen.effective, snapshot?.pen.supported, color]);
  async function closeSetup() { if (p.status !== "completed") await controller.dispatch("defer"); close(); }
  async function choosePen() {
    if (snapshot?.pen.supported.includes(color) && await controller.selectPen(color) && mounted.current) {
      if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) await controller.completePenSelection();
      else { penArmed.current = true; setAnimation(true); }
    }
  }
  const next = async () => { if (p.stage === "pen") await choosePen(); else if (p.stage === "paste") onManualPasteInstructions?.(); else await controller.dispatch(page.action ?? "continue"); };
  return <main className="onboarding-shell" aria-labelledby="onboarding-title" data-stage={p.stage}>
    <header><span>apparent · setup</span><Button variant="ghost" size="icon" aria-label="Close setup" onClick={() => void closeSetup()}>×</Button></header>
    <section aria-live="polite"><div className="onboarding-brand"><img src={frog} alt="Frog guide" /></div><p className="onboarding-eyebrow">Your first capture</p><h1 id="onboarding-title">{page.title}</h1><p>{p.stage === "pen" ? (snapshot?.pen.supported.every(c => ["coral", "purple", "blue", "green"].includes(c)) ? "Choose a colour, then watch the frog hop into your puddle." : "Choose a supported pen style. Record confirms the effective setting before you continue.") : page.text}</p>
      {p.stage === "pen" && <><div className="onboarding-pencils" role="group" aria-label="Pen choice">{(snapshot?.pen.supported ?? []).map(c => <Button key={c} variant="ghost" disabled={animation} aria-pressed={snapshot?.pen.effective === c} aria-label={`${c} pen`} onClick={async () => { if (!await controller.selectPen(c) || !mounted.current) return; setColor(c); if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) await controller.completePenSelection(); else { penArmed.current = true; setAnimation(true); } }}><span style={{ background: penColor(c) }} /><small>{c}</small></Button>)}</div><div className="onboarding-pond" data-hop={animation}><svg viewBox="0 0 300 100" role="img" aria-label={`Puddle for ${color}`}><path d="M22 49C9 21 65 11 99 17C133 1 169 18 192 12C229 4 249 28 275 31C310 37 295 69 263 73C244 94 208 86 180 90C144 102 122 84 91 90C48 93 6 78 22 49Z" fill={penColor(color)} /></svg><img className="onboarding-pond-frog" src={frog} alt="Frog beside the puddle" onAnimationEnd={event => { if (event.animationName === "onboarding-pen-hop" && penArmed.current && mounted.current) { penArmed.current = false; setAnimation(false); void controller.completePenSelection(); } }} /></div><p>{snapshot?.pen.error ?? `Effective pen: ${snapshot?.pen.effective ?? "unknown"}`}</p></>}
      {p.stage === "shortcut" && <><p className="onboarding-fact">Effective shortcut: <kbd>{snapshot?.shortcut.label ?? snapshot?.shortcut.effective ?? "Not confirmed"}</kbd>{snapshot?.shortcut.registration === "externally-managed" && " · managed by the desktop"}</p>{snapshot?.shortcut.externallyManaged ? <><p>The desktop manages this shortcut. Apparent cannot change it here.</p><Button variant="outline" onClick={() => void controller.openDesktopShortcutConfig()}>Open desktop shortcut settings</Button></> : !snapshot?.shortcut.draft ? <Button variant="outline" onClick={() => void controller.beginShortcutEdit()}>Change shortcut</Button> : <><label>Shortcut draft<input value={shortcutValue || snapshot.shortcut.draft.value} onChange={e => setShortcutValue(e.target.value)} /></label><Button variant="outline" onClick={() => void controller.editShortcut(snapshot.shortcut.draft!.sessionId, shortcutValue || snapshot.shortcut.draft!.value)}>Use this shortcut</Button><Button variant="ghost" onClick={() => void controller.cancelShortcut(snapshot.shortcut.draft!.sessionId)}>Cancel</Button><Button onClick={() => void controller.saveShortcut(snapshot.shortcut.draft!.sessionId)}>Save shortcut</Button></>}</>}
      {p.stage === "permissions" && <><p className="onboarding-fact">{snapshot?.connection !== "connected" ? "Record is disconnected; readiness is unknown." : "Permission status and usable recording readiness are reported by the operating system."}</p>{snapshot?.readiness.requirements.map(r => <div className="onboarding-requirement" key={r.id}><strong>{r.id}</strong><span>{r.status}{r.relaunchNeeded ? " · reopen Apparent required" : r.usableNow ? " · usable" : " · not usable"}</span>{r.allowedRepairActions.includes("open-system-settings" as RepairAction) && <Button variant="outline" onClick={() => void controller.repair("open-system-settings", r.id)}>Open Settings</Button>}{r.allowedRepairActions.includes("request-permission" as RepairAction) && <Button variant="outline" onClick={() => void controller.repair("request-permission", r.id)}>Request access</Button>}{r.allowedRepairActions.includes("request-safe-relaunch" as RepairAction) && <Button variant="outline" onClick={() => void controller.safeRelaunch()}>Reopen Apparent</Button>}</div>)}<Button variant="outline" onClick={() => void controller.dispatch("refresh")}>Check access</Button></>}
      {p.stage === "paste" && <Button variant="outline" onClick={onManualPasteInstructions}>Show paste instructions</Button>}
      {p.stage === "practice-start" && <p>Waiting for a shortcut-started capture and its first frame. Menu starts and unrelated recordings do not count.</p>}
      {p.attemptId && <p className="onboarding-fact">Practice capture: {p.attemptId}{snapshot?.captures.find(c => c.captureId === p.attemptId)?.phase === "unknown" ? " · history unknown" : ""}</p>}
      {state.publication === "unknown" && <p role="status">Progress save outcome is unknown. Reload to reconcile before continuing.</p>}
      {state.error && <p role="alert">{state.error}</p>}
    </section><footer><Button variant="ghost" onClick={() => void controller.dispatch("back")} disabled={state.saving || p.stage === "welcome" || p.stage === "complete"}>Back</Button><span>{p.status === "deferred" ? "Paused" : `${["welcome", "pen", "permissions", "shortcut", "practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate", "complete"].indexOf(p.stage) + 1} / 15`}</span><Button onClick={() => p.stage === "complete" ? void closeSetup() : void next()} disabled={state.saving}>{p.stage === "complete" ? "Finish setup" : page.label ?? "Continue"}</Button></footer>
    {p.stage !== "complete" && <Button className="onboarding-later" variant="ghost" onClick={() => void closeSetup()}>Try later</Button>}
  </main>;
}

export function createOnboardingContribution(props: OnboardingContributionProps) { return <OnboardingContribution {...props} />; }
