import { createHash } from "node:crypto";
import { formatLocalHandoff, type LocalHandoffInput } from "./index.js";

export interface RecordLocalClipboardIdentity {
  captureId: string;
  artifactId: string;
}

export interface RecordLocalHandoffResolution extends RecordLocalClipboardIdentity {
  handoff: LocalHandoffInput;
}

/** Structural subset of the published Platform native-host clipboard port. */
export interface NativeClipboardPortInput {
  writeText(text: string, observed: { processEpoch: string; revision: number }): Promise<NativeClipboardWriteResultInput>;
}

type NativeClipboardResult<K extends string> =
  | { status: K; digest: string; byteLength: number }
  | { status: "refused"; reason: "stale" | "busy" | "unavailable" | "invalid-text" | "too-large" }
  | { status: "outcome-unknown" };

export type NativeClipboardWriteResultInput = NativeClipboardResult<"applied">;
/** Local write submission plus matching independent byte observation; no compositor ACK, readback-origin, ownership or paste guarantee. */
export type NativeClipboardTextReadbackResultInput = NativeClipboardResult<"written-readback">;
export interface NativeClipboardTextReadbackPortInput {
  writeTextReadback(text: string, observed: { processEpoch: string; revision: number }): Promise<NativeClipboardTextReadbackResultInput>;
}

/** Structural read-only subset of the published NativeHostPort. */
export interface NativeClipboardHostInput {
  readSnapshot(): Promise<{ processEpoch: string; revision: number }>;
}

export type RecordLocalClipboardRefusalReason =
  | "invalid-identity"
  | "not-saved"
  | "identity-mismatch"
  | "invalid-handoff"
  | "invalid-text"
  | "too-large"
  | "stale"
  | "busy"
  | "unavailable";

type RecordLocalResult<K extends string> =
  | { status: K; captureId: string; artifactId: string; digest: string; byteLength: number }
  | { status: "refused"; captureId?: string; artifactId?: string; reason: RecordLocalClipboardRefusalReason }
  | { status: "outcome-unknown"; captureId: string; artifactId: string };

export type RecordLocalClipboardResult = RecordLocalResult<"applied">;
export type RecordLocalClipboardTextReadbackResult = RecordLocalResult<"written-readback">;
export interface RecordLocalClipboardTextReadbackPort {
  copyLocal(identity: RecordLocalClipboardIdentity): Promise<RecordLocalClipboardTextReadbackResult>;
}
export type RecordLocalClipboardTextReadbackPortInput = Omit<RecordLocalClipboardPortInput, "clipboard"> & { clipboard: NativeClipboardTextReadbackPortInput };

export interface RecordLocalClipboardPort {
  copyLocal(identity: RecordLocalClipboardIdentity): Promise<RecordLocalClipboardResult>;
}

export interface RecordLocalClipboardPortInput {
  clipboard: NativeClipboardPortInput;
  nativeHost: NativeClipboardHostInput;
  /** Trusted composition resolves only the exact capture/artifact pair from authenticated Saved facts. */
  resolveHandoff(identity: Readonly<RecordLocalClipboardIdentity>): Promise<RecordLocalHandoffResolution | null>;
  /** Pass the active producer's MAX_UTF8_TEXT_BYTES. The adapter supplies no default. */
  maximumTextBytes: number;
}

function validIdentity(value: unknown): value is RecordLocalClipboardIdentity {
  return !!value && typeof value === "object" && !Array.isArray(value)
    && typeof (value as Record<string, unknown>).captureId === "string" && (value as Record<string, unknown>).captureId !== ""
    && typeof (value as Record<string, unknown>).artifactId === "string" && (value as Record<string, unknown>).artifactId !== "";
}

function validNativeState(value: unknown): value is { processEpoch: string; revision: number } {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const state = value as Record<string, unknown>;
  return typeof state.processEpoch === "string" && /^[a-zA-Z0-9-]{1,64}$/.test(state.processEpoch)
    && Number.isSafeInteger(state.revision) && Number(state.revision) >= 0;
}

function exactKeys(value: object, keys: string[]): boolean {
  const actual = Object.keys(value);
  return actual.length === keys.length && keys.every(key => Object.hasOwn(value, key));
}

function validUnicodeText(text: string): boolean {
  for (let index = 0; index < text.length; index++) {
    const unit = text.charCodeAt(index);
    if (unit === 0) return false;
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = text.charCodeAt(index + 1);
      if (Number.isNaN(next) || next < 0xdc00 || next > 0xdfff) return false;
      index++;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
  }
  return true;
}

function nativeResult<K extends "applied" | "written-readback">(
  value: unknown,
  expectedDigest: string,
  expectedByteLength: number,
  success: K,
): NativeClipboardResult<K> | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const result = value as Record<string, unknown>;
  if (result.status === success && exactKeys(result, ["status", "digest", "byteLength"])
    && typeof result.digest === "string" && /^[0-9a-f]{64}$/.test(result.digest)
    && result.digest === expectedDigest && Number.isSafeInteger(result.byteLength)
    && result.byteLength === expectedByteLength) {
    return { status: success, digest: result.digest, byteLength: expectedByteLength };
  }
  if (result.status === "refused" && exactKeys(result, ["status", "reason"])
    && ["stale", "busy", "unavailable", "invalid-text", "too-large"].includes(String(result.reason))) {
    return { status: "refused", reason: result.reason as Extract<NativeClipboardWriteResultInput, { status: "refused" }>["reason"] };
  }
  if (result.status === "outcome-unknown" && exactKeys(result, ["status"])) return { status: "outcome-unknown" };
  return undefined;
}

/** One exact formatted handoff write over caller-owned public clipboard and host ports. */
export function createRecordLocalClipboardPort(input: RecordLocalClipboardPortInput): RecordLocalClipboardPort {
  return createClipboardAdapter(input, "applied");
}

export function createRecordLocalClipboardTextReadbackPort(input: RecordLocalClipboardTextReadbackPortInput): RecordLocalClipboardTextReadbackPort {
  if (!input?.clipboard || typeof input.clipboard.writeTextReadback !== "function") throw new TypeError("Invalid text readback port");
  const clipboard = input.clipboard;
  return createClipboardAdapter({ ...input, clipboard: { writeText: (text, observed) => clipboard.writeTextReadback(text, observed) } }, "written-readback");
}

function createClipboardAdapter<K extends "applied" | "written-readback">(input: Omit<RecordLocalClipboardPortInput, "clipboard"> & { clipboard: { writeText(text: string, observed: {processEpoch:string;revision:number}): Promise<NativeClipboardResult<K>> } }, success: K): { copyLocal(identity: RecordLocalClipboardIdentity): Promise<RecordLocalResult<K>> } {
  if (!input || !input.clipboard || typeof input.clipboard.writeText !== "function"
    || !input.nativeHost || typeof input.nativeHost.readSnapshot !== "function"
    || typeof input.resolveHandoff !== "function"
    || !Number.isSafeInteger(input.maximumTextBytes) || input.maximumTextBytes <= 0) {
    throw new TypeError("Invalid local clipboard adapter inputs");
  }
  const { clipboard, nativeHost, resolveHandoff, maximumTextBytes } = input;

  return {
    async copyLocal(callerIdentity) {
      // Snapshot the correlation tuple before the resolver or any other await can run.
      if (!validIdentity(callerIdentity)) return { status: "refused", reason: "invalid-identity" };
      const captureId = callerIdentity.captureId;
      const artifactId = callerIdentity.artifactId;
      const correlation = { captureId, artifactId };

      let resolution: RecordLocalHandoffResolution | null;
      try {
        const candidate = await resolveHandoff(Object.freeze({ ...correlation }));
        if (candidate === null) return { status: "refused", ...correlation, reason: "not-saved" };
        resolution = structuredClone(candidate);
      } catch {
        return { status: "refused", ...correlation, reason: "unavailable" };
      }

      if (!resolution || typeof resolution !== "object"
        || resolution.captureId !== captureId || resolution.artifactId !== artifactId) {
        return { status: "refused", ...correlation, reason: "identity-mismatch" };
      }
      if (!resolution.handoff || resolution.handoff.savedBundle?.status !== "saved"
        || resolution.handoff.savedBundle.bundlePath !== resolution.handoff.processing?.bundlePath) {
        return { status: "refused", ...correlation, reason: "identity-mismatch" };
      }

      let text: string;
      try {
        text = formatLocalHandoff(resolution.handoff).text;
      } catch {
        return { status: "refused", ...correlation, reason: "invalid-handoff" };
      }
      if (!validUnicodeText(text)) return { status: "refused", ...correlation, reason: "invalid-text" };
      const byteLength = Buffer.byteLength(text, "utf8");
      if (byteLength > maximumTextBytes) return { status: "refused", ...correlation, reason: "too-large" };
      const digest = createHash("sha256").update(text, "utf8").digest("hex");

      let state: { processEpoch: string; revision: number };
      try {
        const snapshot = structuredClone(await nativeHost.readSnapshot());
        if (!validNativeState(snapshot)) return { status: "refused", ...correlation, reason: "unavailable" };
        state = { processEpoch: snapshot.processEpoch, revision: snapshot.revision };
      } catch {
        return { status: "refused", ...correlation, reason: "unavailable" };
      }

      let write: unknown;
      try {
        // No await or other side effect separates the fresh observation from this single dispatch.
        write = await clipboard.writeText(text, state);
      } catch {
        return { status: "outcome-unknown", ...correlation };
      }

      let result: NativeClipboardResult<K> | undefined;
      try { result = nativeResult(write, digest, byteLength, success); }
      catch { result = undefined; }
      if (!result || result.status === "outcome-unknown") return { status: "outcome-unknown", ...correlation };
      if (result.status === "refused" && "reason" in result) return { status: "refused", ...correlation, reason: result.reason };
      return { status: success, ...correlation, digest, byteLength };
    },
  };
}
