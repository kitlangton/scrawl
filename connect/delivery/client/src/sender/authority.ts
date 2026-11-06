import { decodePeerAuthority, validatePeerAuthorityKey, PeerAuthorityError,
  type PeerAuthority, type PeerAuthorityKey } from "../recipient/peer-authority.ts";
import { RecipientTransport, RecipientTransportError, normalizeRecipientServer } from "../recipient/transport.ts";

export type SenderPeerAuthority = Readonly<PeerAuthority & { server: string; scope: "upload" }>;
export type SenderPeerAuthorityInput = {
  /** This server must already be approved by the caller; URL syntax is not approval. */
  server: string;
  accessToken: string;
  grantId: string;
  /** Public key derived from the current local endpoint identity, never the response itself. */
  ownPublicKey: PeerAuthorityKey;
  active(): boolean;
  signal?: AbortSignal;
};
const peerFailures = new Set(["invalid_encryption_authorization", "encryption_approval_required", "encryption_key_binding_mismatch"]);
const transportFailures = new Set(["invalid_server", "invalid_request", "invalid_response", "request_failed", "response_too_large",
  "cancelled", "timeout", "stale_snapshot", "unauthorized", "forbidden", "not_found", "slow_down", "quota_exceeded", "redirect_refused"]);
const fail = (category: ConstructorParameters<typeof RecipientTransportError>[0]): never => { throw new RecipientTransportError(category); };

/** One bounded authenticated GET with no refresh, credential mutation or cache.
 * It validates service peer facts against a caller-pinned local public key.
 * The result is a point-in-time receipt, not durable authority for later sends. */
export async function readSenderPeerAuthority(input: SenderPeerAuthorityInput): Promise<SenderPeerAuthority> {
  try {
    const server = normalizeRecipientServer(input.server), token = input.accessToken, grantId = input.grantId;
    const active = input.active, signal = input.signal;
    const ownPublicKey = validatePeerAuthorityKey(input.ownPublicKey);
    if (typeof active !== "function" || typeof grantId !== "string" || !/^[\x21-\x7e]{1,512}$/.test(grantId)) fail("invalid_request");
    const check = () => {
      if (signal?.aborted) fail("cancelled");
      try { if (!active()) fail("stale_snapshot"); } catch { fail("stale_snapshot"); }
      if (signal?.aborted) fail("cancelled");
    };
    check();
    const transport = new RecipientTransport(server, token, { signal });
    const body = await transport.getJson("/v1/encryption/peers");
    check();
    const authority = decodePeerAuthority(body, { scope: "upload", grantId, ownPublicKey });
    return Object.freeze({ ...authority, scope: "upload", server });
  } catch (error) {
    if (error instanceof PeerAuthorityError && peerFailures.has(error.category)) throw new PeerAuthorityError(error.category);
    if (error instanceof RecipientTransportError && transportFailures.has(error.category)) return fail(error.category);
    return fail("request_failed");
  }
}

export { PeerAuthorityError } from "../recipient/peer-authority.ts";
export type { PeerAuthorityKey, PeerAuthorityPeer, PeerAuthorityFailure } from "../recipient/peer-authority.ts";
