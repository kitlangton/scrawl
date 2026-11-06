import type { NativeRequirement } from "@apparent/record/capabilities";

const names: Record<string, string> = {
  "screen-capture": "Screen Recording",
  "screen-recording": "Screen & System Audio Recording",
  microphone: "Microphone",
  accessibility: "Accessibility",
  camera: "Camera",
  "speech-recognition": "Speech Recognition",
};

/** Friendly labels for known provider IDs; unknown IDs never become user-facing copy. */
export function permissionName(id: string): string { return Object.hasOwn(names, id) ? names[id] : "Access"; }

/** Describe provider facts without treating permission acceptance as usable readiness. */
export function permissionStatus(requirement: NativeRequirement | undefined): string {
  if (!requirement) return "Unavailable";
  if (requirement.relaunchNeeded) return "Reopen Apparent to use this access";
  if (requirement.status === "granted" && requirement.usableNow) return "Ready";
  if (requirement.status === "granted") return "Granted · not ready";
  if (requirement.status === "denied") return "Not allowed";
  if (requirement.status === "not-determined") return "Not set up";
  return "Not ready";
}
