export type Stage = "welcome" | "pen" | "permissions" | "shortcut" | "practice-start" | "screen" | "whiteboard" | "board-ink" | "screen-ink" | "stop" | "preparing" | "copied" | "paste" | "locate" | "complete";
export type OnboardingProgress = { schema: 1; status: "new" | "in-progress" | "deferred" | "completed"; stage: Stage; attemptId: string | null; completedMilestones: string[]; manualPasteConfirmed: boolean };
export type Revision = string | number;
export type SaveResult = { status: "saved"; revision: Revision } | { status: "conflict"; revision: Revision; current: OnboardingProgress } | { status: "unknown"; message: string };
export interface ProgressStore { read(): Promise<{ revision: Revision; value: OnboardingProgress }>; compareAndSwap(expected: Revision, value: OnboardingProgress): Promise<SaveResult> }
export const initialProgress = (): OnboardingProgress => ({ schema: 1, status: "new", stage: "welcome", attemptId: null, completedMilestones: [], manualPasteConfirmed: false });
export function normalizeProgress(value: unknown): OnboardingProgress {
  if (!value || typeof value !== "object") return initialProgress();
  const v = value as Partial<OnboardingProgress> & { page?: string; seen?: boolean; deferred?: boolean };
  const aliases: Record<string, Stage> = { tutorial: "welcome", howItWorks: "welcome", resume: "welcome", done: "complete", "try-it": "practice-start" };
  const stage = (aliases[v.stage as string] ?? v.stage) as Stage;
  const valid: Stage[] = ["welcome", "pen", "permissions", "shortcut", "practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate", "complete"];
  const safeStage = valid.includes(stage) ? stage : valid.includes(v.page as Stage) ? v.page as Stage : "welcome";
  const status = v.status === "completed" || v.seen || safeStage === "complete" ? "completed" : v.status === "deferred" || v.deferred ? "deferred" : v.status === "in-progress" ? "in-progress" : "new";
  return { schema: 1, status, stage: safeStage, attemptId: typeof v.attemptId === "string" ? v.attemptId : null, completedMilestones: Array.isArray(v.completedMilestones) ? v.completedMilestones.filter((x): x is string => typeof x === "string") : [], manualPasteConfirmed: v.manualPasteConfirmed === true };
}
const stages: Stage[] = ["welcome", "pen", "permissions", "shortcut", "practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate", "complete"];
export function nextStage(stage: Stage): Stage { return stages[Math.min(stages.indexOf(stage) + 1, stages.length - 1)]; }
