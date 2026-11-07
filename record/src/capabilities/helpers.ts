import type { CapabilitySnapshot, PenColor } from "./index.js";

/** Approved named palette, matching the donor writer; these are not CSS color keywords. */
export const PEN_COLOR_RGB: Readonly<Record<PenColor, readonly [number, number, number]>> = Object.freeze({
  coral: Object.freeze([217, 76, 68] as const),
  purple: Object.freeze([122, 88, 199] as const),
  blue: Object.freeze([59, 121, 189] as const),
  green: Object.freeze([57, 123, 97] as const),
});

/** Confirmation for the exact capture AND current saved artifact, never dispatch/watch proof. */
export function hasConfirmedArtifactOpen(snapshot: CapabilitySnapshot, captureId: string): boolean {
  const capture = snapshot.captures.find(value => value.captureId === captureId);
  return capture?.phase === "saved" && !!capture.artifactId && capture.outcomes.some(outcome =>
    outcome.kind === "artifact-open" && outcome.ok && outcome.artifactId === capture.artifactId
  );
}

/** A sealed initial interval was padded, not observed as silence. */
export interface AudioInitialGapWarning { kind: "audioInitialGap"; atMs: number }
export function audioInitialGapText(atMs: number): string {
  if (!Number.isSafeInteger(atMs) || atMs < 0) throw new RangeError("Invalid initial audio gap");
  return `Audio gap: [0, ${atMs} ms) was unobserved and padded with zeros, not recorded silence.`;
}
