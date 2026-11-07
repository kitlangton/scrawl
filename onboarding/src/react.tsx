import { useEffect, useRef, useState, type ComponentType, type ButtonHTMLAttributes } from "react";
import type { Command, ControllerState } from "./controller.js";
import { OnboardingController } from "./controller.js";
import type { PenColor, RepairAction } from "@apparent/record/capabilities";
import { permissionName, permissionStatus } from "./permissions/presentation.js";
import { ControlGuide } from "./guides/control-guide.js";
import { GestureGuide } from "./guides/gesture-guide.js";
const frog = new URL("./assets/frog.png", import.meta.url).href;
const appIcon = new URL("./assets/apparent-app-icon.svg", import.meta.url).href;
const penColor = (value: PenColor) => `var(--apparent-pen-${value})`;

export type OnboardingContributionProps = { controller: OnboardingController; Button: ComponentType<ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "default" | "ghost" | "outline"; size?: "default" | "icon" }>; close(): void; onManualPasteInstructions?(): void; onReturnToRecording?(): void; loadOnMount?: boolean };
const pages: Record<string, { title: string; text: string; action?: Command; label?: string }> = {
  welcome: { title: "Welcome.", text: "Set up locally. Sign-in is not available yet.", action: "continue", label: "Get started" },
  pen: { title: "Pick your pen.", text: "" },
  permissions: { title: "Recording access", text: "" },
  shortcut: { title: "Choose your shortcut.", text: "Choose the keys you’ll use to start and finish recording." },
  "practice-start": { title: "Start your first capture.", text: "Press your shortcut to start. This practice stays on this device." },
  screen: { title: "Choose Screen.", text: "Choose Screen in the capture controls. Whiteboard becomes available once recording starts." },
  whiteboard: { title: "Open Whiteboard.", text: "Click Screen, then choose Whiteboard." },
  "board-ink": { title: "Draw on the whiteboard.", text: "Try a circle or an arrow. The guide moves on when you finish your mark." },
  note: { title: "Add a note.", text: "Hold Shift and click a spot. The guide waits for a nonempty saved note." },
  "screen-ink": { title: "Draw on your screen.", text: "Hold Shift and circle something in your window." },
  stop: { title: "Finish your capture.", text: "Press your shortcut again. Your sketch and screen marks stay together." },
  preparing: { title: "Preparing your capture…", text: "Recording has stopped. Your marks and context are being brought together." },
  copied: { title: "Copied. Try pasting it.", text: "Your capture is copied. Paste it into your agent.", action: "continue", label: "Show paste steps" },
  paste: { title: "Paste your capture.", text: "Paste it where you want it, then confirm here. We never inspect the destination.", action: "manual-paste-confirm", label: "I pasted it" },
  locate: { title: "apparent lives here.", text: "Find the apparent icon in your menu bar or system tray. Click it, then choose Open Latest Capture." },
  complete: { title: "Ready for your next idea.", text: "You can record, draw, paste, and find your captures. Your setup is complete." },
};

export function OnboardingContribution({ controller, Button, close, onManualPasteInstructions, onReturnToRecording, loadOnMount = true }: OnboardingContributionProps) {
  const [state, setState] = useState<ControllerState>(controller.state);
  const snapshot = state.snapshot;
  const [color, setColor] = useState<PenColor>(snapshot?.pen.effective ?? "coral");
  const [shortcutValue, setShortcutValue] = useState("");
  const [validatedShortcut, setValidatedShortcut] = useState("");
  const [shortcutError, setShortcutError] = useState("");
  const shortcutRun = useRef(0);
  const shortcutRecorder = useRef<HTMLDivElement>(null);
  const [penPhase, setPenPhase] = useState<"idle" | "writing" | "decoding" | "hopping" | "landing" | "completing">("idle");
  const [penError, setPenError] = useState<string | null>(null);
  const [penRetry, setPenRetry] = useState(0);
  const mounted = useRef(true), penRun = useRef(0), penIntent = useRef<string | null>(null);
  const pondFrog = useRef<HTMLImageElement>(null), landingTimer = useRef<ReturnType<typeof setTimeout> | null>(null), finishLanding = useRef<(() => void) | null>(null);
  const penBusy = penPhase !== "idle";
  function invalidatePenAnimation(updateUi = true) {
    penRun.current++;
    penIntent.current = null;
    if (landingTimer.current) clearTimeout(landingTimer.current);
    landingTimer.current = null;
    finishLanding.current?.(); finishLanding.current = null;
    if (updateUi && mounted.current) { setPenPhase("idle"); setPenError(null); }
  }
  useEffect(() => { mounted.current = true; let alive = true; const off = controller.subscribe(s => alive && setState(s)); if (loadOnMount) void controller.load().then(s => alive && setState(s)); return () => { alive = false; mounted.current = false; invalidatePenAnimation(false); off(); }; }, [controller, loadOnMount]);
  const p = state.progress, page = pages[p.stage];
  useEffect(() => { if (snapshot?.pen.effective && !snapshot.pen.supported.includes(color)) setColor(snapshot.pen.effective); }, [snapshot?.pen.effective, snapshot?.pen.supported, color]);
  useEffect(() => { if (snapshot?.shortcut.draft) { shortcutRecorder.current?.focus(); setShortcutValue(""); setValidatedShortcut(""); setShortcutError(""); } }, [snapshot?.shortcut.draft?.sessionId]);
  useEffect(() => {
    if (p.stage !== "pen" || p.status === "deferred") { invalidatePenAnimation(); setPenPhase("idle"); setPenError(null); setPenRetry(0); return; }
    if (penRetry === 0 || !controller.penSelectionConfirmed(color)) return;
    const key = `${snapshot?.providerEpoch ?? ""}:${color}:${penRetry}`;
    if (penIntent.current === key) return;
    penIntent.current = key;
    const run = ++penRun.current;
    const stillCurrent = () => mounted.current && run === penRun.current && p.stage === "pen" && p.status !== "deferred";
    const finish = async () => {
      if (!stillCurrent()) return;
      setPenPhase("completing");
      await controller.completePenSelection();
      if (stillCurrent()) setPenPhase("idle");
    };
    if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) { void finish(); return () => { if (run === penRun.current) invalidatePenAnimation(false); }; }
    const painted = () => new Promise<void>(resolve => {
      if (typeof requestAnimationFrame === "function") requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      else setTimeout(resolve, 0);
    });
    void (async () => {
      try {
        setPenError(null); setPenPhase("decoding");
        const image = pondFrog.current;
        if (!image) throw new Error("Frog image is unavailable");
        await Promise.all([image.decode(), document.fonts?.ready ?? Promise.resolve()]);
        await painted(); if (!stillCurrent()) return;
        setPenPhase("hopping");
        await painted(); if (!stillCurrent()) return;
        const hop = image.getAnimations().find(animation => (animation as CSSAnimation).animationName === "onboarding-pen-hop");
        if (!hop) throw new Error("Frog hop did not start");
        await hop.finished; if (!stillCurrent()) return;
        setPenPhase("landing");
        await new Promise<void>(resolve => {
          finishLanding.current = resolve;
          landingTimer.current = setTimeout(() => { landingTimer.current = null; finishLanding.current = null; resolve(); }, 180);
        });
        if (stillCurrent()) await finish();
      } catch {
        if (!stillCurrent()) return;
        penIntent.current = null; setPenPhase("idle"); setPenError("The frog could not finish its jump. Choose the pen and try again.");
      }
    })();
    return () => { if (run === penRun.current) invalidatePenAnimation(false); };
  }, [controller, color, p.stage, p.status, snapshot?.providerEpoch, snapshot?.pen.effective, penRetry]);
  async function closeSetup() { invalidatePenAnimation(); if (p.status === "completed") { close(); return; } const before = state.progress; const after = await controller.dispatch("defer"); if (after.progress.status === "deferred" && after.publication === "confirmed") close(); else if (after.progress !== before) setState(after); }
  const next = async () => {
    if (p.stage === "pen") {
      if (!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) pondFrog.current?.scrollIntoView?.({ block: "center" });
      setPenRetry(value => value + 1);
      if (controller.penSelectionConfirmed(color)) { setPenError(null); return; }
      setPenError(null); setPenPhase("writing");
      if (!(await controller.selectPen(color))) { setPenPhase("idle"); setPenError(controller.state.error ?? "The color could not be saved. Try choosing it again."); }
    } else await controller.dispatch(page.action ?? "continue");
  };
  const validateShortcut = async (sessionId: string, candidate: string) => {
    const run = ++shortcutRun.current;
    setValidatedShortcut("");
    const result = await controller.editShortcut(sessionId, candidate);
    const draft = result?.snapshot.shortcut.draft;
    if (mounted.current && run === shortcutRun.current && (result?.status === "accepted" || result?.status === "applied") && draft?.sessionId === sessionId && draft.value === candidate && !draft.error) setValidatedShortcut(candidate);
  };
  const beginShortcut = async () => { shortcutRun.current++; setShortcutValue(""); setValidatedShortcut(""); await controller.beginShortcutEdit(); };
  const isPractice = ["practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate"].includes(p.stage);
  const admission = snapshot?.nextShortcutAdmission;
  const practiceArmed = admission?.state === "armed" && admission.admissionId === p.admissionId && admission.delivery === "local-only";
  const practiceShortcut = snapshot?.shortcut.label ?? snapshot?.shortcut.effective;
  const practiceReady = practiceArmed && !!practiceShortcut;
  const requiredPermissionCount = snapshot?.readiness.requirements.filter(requirement => requirement.required).length ?? 0;
  const permissionTitle = requiredPermissionCount === 3 ? "We need three things." : "Review recording access.";
  const showFooter = !isPractice;
  return <main className="onboarding-shell" aria-labelledby="onboarding-title" data-stage={p.stage} data-practice={isPractice}>
    {!isPractice && <header><span>apparent · setup</span><Button variant="ghost" size="icon" aria-label="Close setup" onClick={() => void closeSetup()}>×</Button></header>}
    <section aria-live="polite" className={isPractice ? "onboarding-practice" : p.stage === "pen" ? "onboarding-setup onboarding-pen-picker" : p.stage === "shortcut" ? "onboarding-setup onboarding-shortcut-setup" : "onboarding-setup"}>{isPractice && <img key={p.stage} className="onboarding-coach-frog" src={frog} alt="Frog guide" />}<div className={isPractice ? "onboarding-coach-content" : p.stage === "pen" ? "onboarding-pen-content" : undefined}>{isPractice ? <><p className="onboarding-coach-label">Your first capture</p><h1 id="onboarding-title">{page.title}</h1>{p.stage === "practice-start" && practiceReady ? <><p>Press {practiceShortcut} to start recording.</p><div className="onboarding-coach-actions"><Button className="onboarding-later" variant="ghost" onClick={() => void closeSetup()}>Try later</Button><span className="onboarding-coach-note">Local-only practice</span></div></> : <p>{page.text}</p>}</> : p.stage === "welcome" ? <div className="onboarding-welcome"><h1 id="onboarding-title">Welcome.</h1><img src={appIcon} alt="apparent" /><Button disabled>Sign in with Google</Button><p>Set up locally. Sign-in is not available yet.</p></div> : <><h1 id="onboarding-title">{p.stage === "permissions" ? permissionTitle : page.title}</h1>{p.stage !== "pen" && page.text && <p>{page.text}</p>}{p.stage === "complete" && <div className="onboarding-complete-frog"><img src={frog} alt="" /></div>}</>}
      {p.stage === "practice-start" && practiceReady && <GestureGuide kind="record" shortcut={practiceShortcut!} Button={Button} />}
      {(p.stage === "screen" || p.stage === "whiteboard") && <ControlGuide kind="whiteboard" screenOnly={p.stage === "screen"} Button={Button} />}
      {(p.stage === "board-ink" || p.stage === "screen-ink") && <><GestureGuide kind="draw" shortcut="" Button={Button} />{p.stage === "screen-ink" && <aside className="onboarding-trackpad-hint"><strong>Pressing down on your trackpad?</strong><p>No need. Just hold Shift and glide.</p></aside>}</>}
      {p.stage === "note" && <GestureGuide kind="note" shortcut="" Button={Button} />}
      {p.stage === "stop" && <ControlGuide kind="finish" Button={Button} />}
      {p.stage === "locate" && <ControlGuide kind="locate" Button={Button} />}
      {p.status === "deferred" && <Button onClick={() => void controller.dispatch("reopen")}>Resume setup</Button>}
      {p.stage === "pen" && <><div className="onboarding-pencils" role="group" aria-label="Pen color">{(snapshot?.pen.supported ?? []).map(c => <Button key={c} variant="ghost" disabled={penBusy} aria-pressed={color === c} aria-label={`${c} pen`} onClick={() => { invalidatePenAnimation(); setPenRetry(0); setColor(c); }}><span aria-hidden="true" style={{ "--pencil": penColor(c) } as React.CSSProperties} /></Button>)}</div><div className="onboarding-pond" data-phase={penPhase} data-hop={penPhase === "hopping"} data-landing={penPhase === "hopping" || penPhase === "landing"} style={{ "--pond-colour": snapshot?.pen.effective === color ? penColor(color) : "var(--apparent-muted)" } as React.CSSProperties}><svg className="onboarding-puddle" viewBox="0 0 300 100" role="img" aria-label={`Puddle for ${color}`}><path d="M22 49C9 21 65 11 99 17C133 1 169 18 192 12C229 4 249 28 275 31C310 37 295 69 263 73C244 94 208 86 180 90C144 102 122 84 91 90C48 93 6 78 22 49Z" /><ellipse className="onboarding-puddle-ring" cx="150" cy="49" rx="48" ry="15" /></svg><img ref={pondFrog} className="onboarding-pond-frog" src={frog} alt="Frog beside the puddle" /></div>{(penError || snapshot?.pen.error) && <p role="alert">{penError ?? snapshot?.pen.error}</p>}</>}
      {p.stage === "shortcut" && <div className="onboarding-shortcut"><p className="onboarding-fact">Current shortcut: <kbd>{snapshot?.shortcut.label ?? snapshot?.shortcut.effective ?? "Not confirmed"}</kbd><br />{snapshot?.shortcut.registration === "registered" ? "Ready to use." : snapshot?.shortcut.registration === "externally-managed" ? "Managed by the desktop." : "Shortcut status is not confirmed yet."}</p>{snapshot?.shortcut.externallyManaged ? <><p>The desktop manages this shortcut. Apparent cannot change it here.</p><Button variant="outline" onClick={() => void controller.openDesktopShortcutConfig()}>Open desktop shortcut settings</Button></> : !snapshot?.shortcut.draft ? <Button variant="outline" onClick={() => void beginShortcut()}>{snapshot?.shortcut.effective || snapshot?.shortcut.label ? "Change shortcut" : "Choose shortcut"}</Button> : <><div ref={shortcutRecorder} tabIndex={0} role="group" aria-label="Shortcut recorder" className="onboarding-shortcut-recorder" onKeyDown={event => {
        if (event.key === "Tab" || event.target !== event.currentTarget) return;
        if (event.key === "Escape") { event.preventDefault(); shortcutRun.current++; setShortcutValue(""); setValidatedShortcut(""); void controller.cancelShortcut(snapshot.shortcut.draft!.sessionId); return; }
        event.preventDefault();
        shortcutRun.current++;
        const key = /^(Key[A-Z]|Digit[0-9])$/.test(event.code) ? event.code.replace(/^(Key|Digit)/, "") : "";
        if (!key || !(event.ctrlKey || event.altKey || event.metaKey || event.shiftKey)) { setShortcutValue(""); setValidatedShortcut(""); setShortcutError("Use a letter or number with Ctrl, Alt, Meta or Shift."); return; }
        const modifiers = [event.ctrlKey && "Ctrl", event.altKey && "Alt", event.metaKey && "Meta", event.shiftKey && "Shift"].filter(Boolean);
        const candidate = `${modifiers.join("+")}+${key}`;
        setShortcutValue(candidate); setValidatedShortcut(""); setShortcutError(""); void validateShortcut(snapshot.shortcut.draft!.sessionId, candidate);
      }}><p>Press Ctrl, Alt, Meta or Shift with a letter or number.</p>{shortcutError && <p role="alert">{shortcutError}</p>}<kbd>{shortcutValue || snapshot.shortcut.draft.value || "Waiting for keys"}</kbd></div><Button variant="ghost" onClick={() => { shortcutRun.current++; setShortcutValue(""); setValidatedShortcut(""); void controller.cancelShortcut(snapshot.shortcut.draft!.sessionId); }}>Cancel</Button><Button disabled={!validatedShortcut || validatedShortcut !== shortcutValue || validatedShortcut !== snapshot.shortcut.draft.value || !!snapshot.shortcut.draft.error || state.command === "unknown"} onClick={() => void controller.saveShortcut(snapshot.shortcut.draft!.sessionId)}>Save shortcut</Button>{snapshot.shortcut.draft.error && <p role="alert">{snapshot.shortcut.draft.error}</p>}</>}</div>}
      {p.stage === "shortcut" && !p.setupCompleted && snapshot?.shortcutTrial && ["reserved", "pressing"].includes(snapshot.shortcutTrial.state) && <p role="status">Press and release your saved shortcut to finish setup. This test does not start a recording.</p>}
      {p.stage === "shortcut" && !p.setupCompleted && (snapshot?.connection !== "connected" || !snapshot.shortcutTrialBeginBinding || snapshot.shortcutTrial?.state === "unavailable" || snapshot.shortcutTrial?.state === "unknown") && <p role="status">A physical shortcut check is not available yet. Setup stays pending until Record can confirm it.</p>}
      {p.stage === "shortcut" && !p.setupCompleted && snapshot?.shortcutTrial && ["reserved", "pressing"].includes(snapshot.shortcutTrial.state) && <Button onClick={() => void controller.cancelShortcutTrial()}>Cancel shortcut test</Button>}
      {p.stage === "shortcut" && p.setupCompleted && onReturnToRecording && <Button onClick={onReturnToRecording}>Return to recording</Button>}
      {p.stage === "permissions" && <><div className="onboarding-permissions">{snapshot?.readiness.requirements.map(r => { const connected = snapshot.connection === "connected"; const enabled = connected && r.status === "granted" && r.usableNow && !r.relaunchNeeded; const unknown = !connected || r.status === "unavailable"; return <div className="onboarding-requirement" key={r.id}><div><strong>{permissionName(r.id)}</strong><span className={enabled ? "onboarding-permission-enabled" : undefined}>{unknown ? "Status unknown" : enabled ? "✓ Enabled" : permissionStatus(r)}</span></div>{connected && r.allowedRepairActions.includes("open-system-settings" as RepairAction) && <Button variant="outline" onClick={() => void controller.repair("open-system-settings", r.id)}>Open Settings</Button>}{connected && r.allowedRepairActions.includes("request-permission" as RepairAction) && <Button variant="outline" onClick={() => void controller.repair("request-permission", r.id)}>Request access</Button>}{connected && r.allowedRepairActions.includes("request-safe-relaunch" as RepairAction) && <Button variant="outline" onClick={() => void controller.safeRelaunch()}>Reopen Apparent</Button>}</div>; })}</div>{(snapshot?.connection !== "connected" || snapshot.readiness.requirements.some(r => r.status === "unavailable")) && <p className="onboarding-permission-notice" role="status">Access status is unknown. Reconnect to check again.</p>}<Button variant="outline" onClick={() => void controller.dispatch("refresh")}>Check access</Button></>}
      {p.stage === "copied" && <Button onClick={() => void controller.dispatch(page.action ?? "continue")}>{page.label}</Button>}
      {p.stage === "paste" && <><Button onClick={() => void controller.dispatch("manual-paste-confirm")}>I pasted it</Button><Button variant="outline" onClick={onManualPasteInstructions}>Show paste instructions</Button></>}
      {p.stage === "locate" && <p className="onboarding-waiting" role="status">Open the saved capture from the Apparent menu after confirming you pasted it.</p>}
      {p.stage === "preparing" && p.attemptId && snapshot?.captures.find(c => c.captureId === p.attemptId)?.phase === "saved" && !snapshot.captures.find(c => c.captureId === p.attemptId)?.outcomes.some(o => o.kind === "copy" && o.ok) && <Button variant="outline" onClick={() => void controller.retryPracticeCopy()}>Retry copy</Button>}
      {p.stage === "practice-start" && <>{!practiceReady && <p className="onboarding-waiting" role="status">{practiceArmed ? "Your shortcut label is unavailable. Check status before starting." : p.admissionId ? "Preparing your local practice…" : "The previous practice ended. Try again when you’re ready."}</p>}{!p.admissionId && <Button onClick={() => void controller.dispatch("continue")}>Try again</Button>}</>}
      {p.attemptId && snapshot?.captures.find(c => c.captureId === p.attemptId)?.phase === "unknown" && <p className="onboarding-fact">This capture is not available yet. Reconnect and check again.</p>}
      {state.publication === "unknown" && <p role="status">Your setup changes may not have been saved. Reload before continuing.</p>}
      {state.publication === "unknown" && <Button variant="outline" onClick={() => void controller.dispatch("retry")}>Reload progress</Button>}
      {state.command === "unknown" && <Button variant="outline" onClick={() => void controller.dispatch("refresh")}>Check status</Button>}
      {state.error && <p role="alert">{state.error}</p>}
      {isPractice && !practiceReady && <Button className="onboarding-later" variant="ghost" onClick={() => void closeSetup()}>Try later</Button>}
      </div>{!isPractice && p.stage !== "complete" && <Button className="onboarding-later" variant="ghost" onClick={() => void closeSetup()}>Try later</Button>}</section>{showFooter && <footer className={p.stage === "pen" ? "onboarding-pen-footer" : undefined}><Button variant="ghost" onClick={() => { if (p.stage === "pen") invalidatePenAnimation(); void controller.dispatch("back"); }} disabled={state.saving || state.publication === "unknown" || p.stage === "welcome" || p.stage === "complete" || ["copied", "paste", "locate"].includes(p.stage)}>Back</Button>{["welcome", "permissions", "shortcut"].includes(p.stage) && <span>{p.status === "deferred" ? "Paused" : `${p.stage === "welcome" || p.stage === "pen" ? 1 : p.stage === "permissions" ? 2 : 3} / 3`}</span>}<Button onClick={() => p.stage === "complete" ? void closeSetup() : void next()} disabled={state.saving || state.publication === "unknown" || (p.stage === "pen" && penBusy)}>{p.stage === "complete" ? "Finish setup" : p.stage === "pen" ? "Use this pen" : p.stage === "permissions" ? "Choose recording shortcut" : p.stage === "shortcut" ? (p.setupCompleted ? "Continue to practice" : "Try your shortcut") : p.stage === "welcome" ? "Choose a pen" : page.label ?? "Continue"}</Button></footer>}
  </main>;
}

export function createOnboardingContribution(props: OnboardingContributionProps) { return <OnboardingContribution {...props} />; }
