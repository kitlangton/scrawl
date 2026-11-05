import type { JsonValue, ProgressCodec } from "@apparent/app-contracts/host";

export type Stage = "welcome" | "pen" | "permissions" | "shortcut" | "practice-start" | "screen" | "whiteboard" | "board-ink" | "screen-ink" | "stop" | "preparing" | "copied" | "paste" | "locate" | "complete";
export type LegacyMetadata = { tutorial: "skipped" | "watched" | "practiced" | null; relaunchedAt: number | null; reopenFix: boolean; shortcutChosen: boolean; firstCaptureCompleted: boolean; granted?: string[] };
export type OnboardingProgress = { schema: 1; status: "new" | "in-progress" | "deferred" | "completed"; stage: Stage; attemptId: string | null; admissionId: string | null; practiceBaselineIds: string[]; legacy: LegacyMetadata; manualPasteConfirmed: boolean };
export type Revision = string | number;
export type SaveResult = { status: "saved"; revision: Revision } | { status: "conflict"; revision: Revision; current: OnboardingProgress } | { status: "unknown"; message: string };
export interface ProgressStore { read(): Promise<{ revision: Revision; value: OnboardingProgress }>; compareAndSwap(expected: Revision, value: OnboardingProgress): Promise<SaveResult> }
export const initialProgress = (): OnboardingProgress => ({ schema: 1, status: "new", stage: "welcome", attemptId: null, admissionId: null, practiceBaselineIds: [], legacy: { tutorial: null, relaunchedAt: null, reopenFix: false, shortcutChosen: false, firstCaptureCompleted: false }, manualPasteConfirmed: false });
export function normalizeProgress(value: unknown): OnboardingProgress {
  if (!value || typeof value !== "object") return initialProgress();
  const v = value as Record<string, unknown>;
  if (Object.hasOwn(v, "schema") && v.schema !== 1) throw new TypeError("Unsupported onboarding progress schema");
  // Re-reading our normalized schema must preserve the migrated metadata too.
  const metadata = v.legacy && typeof v.legacy === "object" ? v.legacy as Record<string, unknown> : v;
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
  const tutorial = metadata.tutorial === "skipped" || metadata.tutorial === "watched" || metadata.tutorial === "practiced" ? metadata.tutorial : null;
  return { schema: 1, status, stage: completed ? "complete" : safeStage, attemptId: typeof v.attemptId === "string" ? v.attemptId : null, admissionId: typeof v.admissionId === "string" ? v.admissionId : null, practiceBaselineIds: Array.isArray(v.practiceBaselineIds) ? v.practiceBaselineIds.filter((x): x is string => typeof x === "string") : [], legacy: { tutorial, relaunchedAt: typeof metadata.relaunchedAt === "number" && Number.isFinite(metadata.relaunchedAt) ? metadata.relaunchedAt : null, reopenFix: metadata.reopenFix === true, shortcutChosen: metadata.shortcutChosen === true, firstCaptureCompleted: metadata.firstCaptureCompleted === true, ...(Array.isArray(metadata.granted) ? { granted: metadata.granted.filter((x): x is string => typeof x === "string") } : {}) }, manualPasteConfirmed: v.manualPasteConfirmed === true };
}
/** Inspect descriptors before reading values, including array indices. */
function progressJson(value: unknown, ancestors = new Set<object>(), depth = 0): JsonValue {
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (!value || typeof value !== "object" || depth > 32 || ancestors.has(value)) throw new TypeError("Onboarding progress must be finite JSON data without cycles");
  const array = Array.isArray(value);
  if (Object.getPrototypeOf(value) !== (array ? Array.prototype : Object.prototype)) throw new TypeError("Onboarding progress must be plain data");
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const keys = Reflect.ownKeys(descriptors).filter(key => !(array && key === "length"));
  if (array && (keys.length !== value.length || keys.some((key, i) => key !== String(i)))) throw new TypeError("Onboarding progress arrays must be dense without extra fields");
  ancestors.add(value);
  const output: JsonValue[] | Record<string, JsonValue> = array ? [] : {};
  for (const key of keys) {
    if (typeof key !== "string" || ["__proto__", "constructor", "prototype", "toJSON"].includes(key)) throw new TypeError("Unsupported progress key");
    const descriptor = descriptors[key];
    if (!descriptor.enumerable || !("value" in descriptor)) throw new TypeError("Onboarding progress cannot contain accessors or hidden fields");
    Object.defineProperty(output, key, { enumerable: true, configurable: true, writable: true, value: progressJson(descriptor.value, ancestors, depth + 1) });
  }
  ancestors.delete(value);
  return output;
}
/** Owner codec for the host's opaque, owner-namespaced JSON value. */
export const onboardingProgressCodec: ProgressCodec<OnboardingProgress> = {
  decode(value: JsonValue | null): OnboardingProgress {
    if (value === null) return initialProgress();
    value = progressJson(value);
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new TypeError("Invalid onboarding progress object");
    if (typeof value === "object" && value !== null && !Array.isArray(value) && Object.hasOwn(value, "schema")) {
      const v = value as Record<string, unknown>;
      if (v.schema !== 1) throw new TypeError("Unsupported onboarding progress schema");
      const m = v.legacy as Record<string, unknown> | null;
      const fields = ["schema", "status", "stage", "attemptId", "admissionId", "practiceBaselineIds", "legacy", "manualPasteConfirmed"];
      if (Object.keys(v).some(key => !fields.includes(key)) || (v.status === "completed") !== (v.stage === "complete")) throw new TypeError("Invalid onboarding progress state");
      const stages: Stage[] = ["welcome", "pen", "permissions", "shortcut", "practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate", "complete"];
      if (!(v.status === "new" || v.status === "in-progress" || v.status === "deferred" || v.status === "completed")
        || typeof v.stage !== "string" || !stages.includes(v.stage as Stage)
        || !(v.attemptId === null || typeof v.attemptId === "string") || !(v.admissionId === null || typeof v.admissionId === "string")
        || !Array.isArray(v.practiceBaselineIds) || !v.practiceBaselineIds.every(id => typeof id === "string")
        || typeof v.manualPasteConfirmed !== "boolean" || !m || typeof m !== "object"
        || !([null, "skipped", "watched", "practiced"].includes(m.tutorial as never)
          && (m.relaunchedAt === null || (typeof m.relaunchedAt === "number" && Number.isFinite(m.relaunchedAt)))
          && typeof m.reopenFix === "boolean" && typeof m.shortcutChosen === "boolean" && typeof m.firstCaptureCompleted === "boolean"
          && (m.granted === undefined || (Array.isArray(m.granted) && m.granted.every(x => typeof x === "string"))))) {
        throw new TypeError("Invalid onboarding progress schema");
      }
    }
    if (!Object.hasOwn(value, "schema") && Object.keys(value).length > 0 && !["seen", "deferred", "resume", "page", "step", "currentStep", "stage", "tutorial", "granted", "relaunchedAt", "reopenFix", "shortcutChosen", "firstCaptureCompleted"].some(key => Object.hasOwn(value, key))) throw new TypeError("Unrecognized legacy onboarding progress");
    return normalizeProgress(value);
  },
  encode(value: OnboardingProgress): JsonValue {
    return onboardingProgressCodec.decode(progressJson(value));
  },
};
const stages: Stage[] = ["welcome", "pen", "permissions", "shortcut", "practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate", "complete"];
export function nextStage(stage: Stage): Stage { return stages[Math.min(stages.indexOf(stage) + 1, stages.length - 1)]; }
