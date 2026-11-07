import { decodeStrictJson } from "../recipient/json.ts";
import { RecipientTransportError } from "../recipient/transport.ts";
import { checkActive, scopedRequest } from "../sender/request.ts";
import { pinAccountRequest, safeTransportCategory, type AccountRequestOptions } from "./authority.ts";

export type { AccountRequestOptions, ShareAuthorization as AccountAuthorization } from "./authority.ts";

export type AccountStatus = Readonly<{ beta: "active" | "waitlist"; quotaBytes: number; usedBytes: number }>;

function invalid(): never { throw new RecipientTransportError("invalid_response"); }
function parseStatus(bytes: Uint8Array): AccountStatus {
  let value: unknown;
  try { value = decodeStrictJson(bytes, 2_000_000); } catch { return invalid(); }
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  const body = value as Record<string, unknown>;
  if (!Object.hasOwn(body, "beta") || !Object.hasOwn(body, "quotaBytes") || !Object.hasOwn(body, "usedBytes")) return invalid();
  if (body.beta !== "active" && body.beta !== "waitlist") return invalid();
  if (typeof body.quotaBytes !== "number" || !Number.isSafeInteger(body.quotaBytes) || body.quotaBytes < 0
      || typeof body.usedBytes !== "number" || !Number.isSafeInteger(body.usedBytes) || body.usedBytes < 0) return invalid();
  return Object.freeze({ beta: body.beta, quotaBytes: body.quotaBytes, usedBytes: body.usedBytes });
}

/** Read one status snapshot using caller-selected, locally approved account authority. */
export async function readAccountStatus(options: AccountRequestOptions): Promise<AccountStatus> {
  try {
    const pinned = pinAccountRequest(options), signal = pinned.signal ?? new AbortController().signal;
    const bytes = await scopedRequest(pinned, pinned.server, "/v1/account", "GET");
    let result: AccountStatus;
    try { result = parseStatus(bytes); } finally { bytes.fill(0); }
    checkActive(pinned, signal);
    return result;
  } catch (error) {
    throw new RecipientTransportError(safeTransportCategory(error instanceof RecipientTransportError ? error.category : undefined));
  }
}
