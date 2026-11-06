import { decodeStrictJson } from "../recipient/json.ts";

export type RecipientRenewalRequest = Readonly<{ protocol: 1; operation: Readonly<{
  server: string; generation: string; token: string; cancellation: string | null;
} & ({ kind: "renew_read"; force: boolean } | { kind: "reject_read" })> }>;
export type RecipientRenewalState = "current" | "rejected" | "cancelled" | "auth_required" | "invalid_server" | "offline" | "state_unavailable";
/** Private host port bound to this same state namespace and its authoritative writer.
 * Assembly admits/protects the existing writer executable and bounds its lifetime.
 * Never expose requests (which contain an access token) to a renderer or logs. */
export type RecipientRenewalOwner = (request: RecipientRenewalRequest, signal?: AbortSignal) => Promise<{ state: RecipientRenewalState }>;
const failures = ["cancelled", "auth_required", "invalid_server", "offline", "state_unavailable"] as const;
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);
const keys = (value: Record<string, unknown>, expected: string[]) => JSON.stringify(Object.keys(value).sort()) === JSON.stringify(expected.sort());

/** Decode only the existing recipient executable's one-line renewal reply.
 * No private message, unexpected fields, partial success or exit mismatch escapes. */
export function decodeRecipientRenewalReply(bytes: Uint8Array, exitCode: number,
  operation: "renew_read" | "reject_read" = "renew_read"): { state: RecipientRenewalState } {
  try {
    const value = decodeStrictJson(bytes, 4096);
    if (!object(value) || value.protocol !== 1) return { state: "state_unavailable" };
    if (exitCode === 0 && keys(value, ["protocol", "result"]) && object(value.result)
      && keys(value.result, ["scope", "state"]) && value.result.scope === "read"
      && (operation === "renew_read" && value.result.state === "current" || operation === "reject_read" && value.result.state === "rejected")) {
      return { state: value.result.state as "current" | "rejected" };
    }
    if (exitCode === 1 && keys(value, ["protocol", "error"]) && object(value.error) && keys(value.error, ["code", "message"])
      && (value.error.code === "operation_failed" || value.error.code === "cancelled")
      && failures.includes(value.error.message as typeof failures[number])
      && (value.error.code !== "cancelled" || value.error.message === "cancelled")) {
      return { state: value.error.message as RecipientRenewalState };
    }
  } catch {}
  return { state: "state_unavailable" };
}
