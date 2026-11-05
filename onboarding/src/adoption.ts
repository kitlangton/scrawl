import type { ConfigAdoptionSnapshot, LegacyAdoptionBackend } from "@apparent/app-contracts/host";
import { onboardingProgressCodec, type OnboardingProgress } from "./model.js";

export type OnboardingAdoptionResult =
  | { status: "ready"; source: "existing" | "adopted"; revision: number; value: OnboardingProgress }
  | { status: "conflict"; snapshot: ConfigAdoptionSnapshot }
  | { status: "unknown"; message: string };

/** Privileged bootstrap only, after the host quiesces the legacy journey writer.
 * Does not install a route, retry a write, or create a config reader/writer.
 */
export async function adoptOnboardingProgress(
  backend: Pick<LegacyAdoptionBackend, "readForAdoption" | "adoptLegacy">,
): Promise<OnboardingAdoptionResult> {
  let snapshot: ConfigAdoptionSnapshot;
  try { snapshot = await backend.readForAdoption("onboarding"); }
  catch { return { status: "unknown", message: "Setup progress could not be read." }; }
  if (snapshot.owner.present) {
    return { status: "ready", source: "existing", revision: snapshot.owner.revision, value: onboardingProgressCodec.decode(snapshot.owner.value) };
  }
  // Presence stays in the compare condition even though missing/null both decode
  // to a new journey. Stored owner null never falls back to legacy progress.
  const value = onboardingProgressCodec.decode(snapshot.legacy.present ? snapshot.legacy.value : null);
  const encoded = onboardingProgressCodec.encode(value);
  try {
    const result = await backend.adoptLegacy("onboarding", snapshot.legacy, encoded);
    if (result.status === "saved") return { status: "ready", source: "adopted", revision: result.revision, value };
    if (result.status === "conflict") return result;
    return { status: "unknown", message: "Setup progress publication could not be confirmed. Read again before continuing." };
  } catch {
    return { status: "unknown", message: "Setup progress publication could not be confirmed. Read again before continuing." };
  }
}
