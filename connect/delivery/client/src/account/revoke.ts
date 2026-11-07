import { decodeStoredCredential, type StoredCredential } from "../credentials/wire.ts";
import { postAccountJson } from "./request.ts";
import { normalizeRecipientServer, RecipientTransportError } from "../recipient/transport.ts";

export type RevokeOptions = Readonly<{ approvedServer: string; timeoutMs?: number; signal?: AbortSignal; active?: () => boolean }>;

/** Revoke exactly this stored device grant. This does not settle local logout state. */
export async function revokeDeviceGrant(credentialInput: StoredCredential, optionsInput: RevokeOptions): Promise<Readonly<{ ok: true }>> {
  // Pin caller-owned values before any asynchronous work.
  const credential = credentialInput;
  const approvedServer = optionsInput?.approvedServer;
  const timeoutMs = optionsInput?.timeoutMs;
  const signal = optionsInput?.signal;
  const active = optionsInput?.active;
  const normalized = normalizeRecipientServer(approvedServer);
  if (!credential || typeof credential !== "object" || credential.server !== approvedServer || credential.scope !== "read" && credential.scope !== "upload")
    throw new RecipientTransportError("invalid_request");
  let bytes: Uint8Array | undefined, validated: StoredCredential;
  try {
    bytes = new TextEncoder().encode(JSON.stringify(credential));
    validated = decodeStoredCredential(bytes, credential.scope, approvedServer);
  } catch { throw new RecipientTransportError("invalid_request"); }
  finally { bytes?.fill(0); }

  // Renewable grants revoke by refresh credential without an access bearer. Legacy grants use
  // the current bearer and omit refreshToken entirely (the service rejects explicit null).
  const reply = validated.refreshToken !== null
    ? await postAccountJson("/v1/token/revoke", normalized, { refreshToken: validated.refreshToken }, timeoutMs, signal, active)
    : await postAccountJson("/v1/token/revoke", normalized, {}, timeoutMs, signal, active, validated.token);
  if (reply === null || typeof reply !== "object" || Array.isArray(reply)
      || Object.keys(reply).length !== 1 || (reply as Record<string, unknown>).ok !== true)
    throw new RecipientTransportError("invalid_response");
  return Object.freeze({ ok: true as const });
}
