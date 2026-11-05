export type Stage = "welcome" | "pen" | "permissions" | "shortcut" | "practice-start" | "screen" | "whiteboard" | "board-ink" | "screen-ink" | "stop" | "preparing" | "copied" | "paste" | "locate" | "complete";
export type LegacyMetadata = { tutorial: "skipped" | "watched" | "practiced" | null; relaunchedAt: number | null; reopenFix: boolean; shortcutChosen: boolean; firstCaptureCompleted: boolean };
export type OnboardingProgress = { schema: 1; status: "new" | "in-progress" | "deferred" | "completed"; stage: Stage; attemptId: string | null; practiceBaselineIds: string[]; legacy: LegacyMetadata; manualPasteConfirmed: boolean };
export type Revision = string | number;
export type SaveResult = { status: "saved"; revision: Revision } | { status: "conflict"; revision: Revision; current: OnboardingProgress } | { status: "unknown"; message: string };
export interface ProgressStore { read(): Promise<{ revision: Revision; value: OnboardingProgress }>; compareAndSwap(expected: Revision, value: OnboardingProgress): Promise<SaveResult> }
export const initialProgress = (): OnboardingProgress => ({ schema: 1, status: "new", stage: "welcome", attemptId: null, practiceBaselineIds: [], legacy: { tutorial: null, relaunchedAt: null, reopenFix: false, shortcutChosen: false, firstCaptureCompleted: false }, manualPasteConfirmed: false });
export function normalizeProgress(value: unknown): OnboardingProgress {
  if (!value || typeof value !== "object") return initialProgress();
  const v = value as Record<string, unknown>;
  const legacyLocation = v.resume ?? v.page ?? v.step ?? v.currentStep ?? v.stage;
  const aliases: Record<string, Stage> = { tutorial: "welcome", howItWorks: "welcome", "how-it-works": "welcome", resume: "welcome", record: "welcome", draw: "welcome", finish: "welcome", done: "complete", "try-it": "practice-start", TryIt: "practice-start", Permissions: "permissions", Shortcut: "shortcut", Done: "complete", Welcome: "welcome", HowItWorks: "welcome", "screen-recording": "permissions", microphone: "permissions", "input-monitoring": "permissions", accessibility: "permissions", camera: "permissions" };
  const stage = (aliases[String(legacyLocation)] ?? legacyLocation) as Stage;
  const valid: Stage[] = ["welcome", "pen", "permissions", "shortcut", "practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate", "complete"];
  const parsedStage = v.reopenFix === true ? "permissions" : valid.includes(stage) ? stage : "welcome";
  const completed = v.status === "completed" || v.seen === true || v.completed === true || v.finished === true;
  const deferred = v.status === "deferred" || v.deferred === true;
  const malformedCurrentComplete = parsedStage === "complete" && !completed;
  const safeStage = malformedCurrentComplete ? "welcome" : parsedStage;
  const status = completed ? "completed" : deferred ? "deferred" : v.status === "in-progress" || v.seen === false || malformedCurrentComplete ? "in-progress" : "new";
  const tutorial = v.tutorial === "skipped" || v.tutorial === "watched" || v.tutorial === "practiced" ? v.tutorial : null;
  return { schema: 1, status, stage: completed ? "complete" : safeStage, attemptId: typeof v.attemptId === "string" ? v.attemptId : null, practiceBaselineIds: Array.isArray(v.practiceBaselineIds) ? v.practiceBaselineIds.filter((x): x is string => typeof x === "string") : [], legacy: { tutorial, relaunchedAt: typeof v.relaunchedAt === "number" && Number.isFinite(v.relaunchedAt) ? v.relaunchedAt : null, reopenFix: v.reopenFix === true, shortcutChosen: v.shortcutChosen === true, firstCaptureCompleted: v.firstCaptureCompleted === true }, manualPasteConfirmed: v.manualPasteConfirmed === true };
}
const stages: Stage[] = ["welcome", "pen", "permissions", "shortcut", "practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate", "complete"];
export function nextStage(stage: Stage): Stage { return stages[Math.min(stages.indexOf(stage) + 1, stages.length - 1)]; }
