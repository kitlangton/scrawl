import { RecipientTransportError, type RecipientTransportCategory } from "../recipient/transport.ts";
import { normalizeServer, type ScopedRequestOptions, type ScopedAuthorization } from "../sender/request.ts";

const ID = /^[a-f0-9]{32}$/;
const allowedFailures = new Set<string>(["not_found", "unauthorized", "forbidden", "slow_down", "quota_exceeded", "request_failed",
  "response_too_large", "invalid_response", "invalid_server", "invalid_request", "redirect_refused", "cancelled", "timeout",
  "auth_required", "renewal_required", "state_unavailable", "stale_snapshot", "revocation_pending", "offline", "revocation_unverified"]);

/** Shared bounded projection for account transport failures; never carries callback or server text. */
export const safeTransportCategory = (value: unknown): RecipientTransportCategory => typeof value === "string" && allowedFailures.has(value) && value !== "revocation_unverified" ? value as RecipientTransportCategory : "request_failed";

export type ShareAuthorization = ScopedAuthorization & { generation: string };
export type ShareOptions = Omit<ScopedRequestOptions, "authorize"> & {
  /** Caller-approved local login incarnation; renewal may change token, never this binding. */
  generation: string;
  authorize(server: string, scope: "read" | "upload", signal: AbortSignal): Promise<ShareAuthorization>;
};
export type AccountRequestOptions = ShareOptions;

/** Pin the exact caller-approved endpoint, scope and local login generation before asynchronous authorization. */
export function pinAccountRequest(options: AccountRequestOptions) {
  const server = normalizeServer(options.server), scope = options.scope, generation = options.generation, authorize = options.authorize;
  if (typeof generation !== "string" || !ID.test(generation) || scope !== "read" && scope !== "upload" || typeof options.active !== "function" || typeof authorize !== "function") throw new RecipientTransportError("invalid_request");
  return Object.freeze({ server, scope, generation, signal: options.signal, active: options.active,
    async authorize(approvedServer: string, requestedScope: "read" | "upload", signal: AbortSignal): Promise<ShareAuthorization> {
      const auth = await authorize(approvedServer, requestedScope, signal);
      if (!auth || auth.generation !== generation) throw new RecipientTransportError("stale_snapshot");
      return { approvedServer: auth.approvedServer, scope: auth.scope, token: auth.token, generation };
    } });
}
