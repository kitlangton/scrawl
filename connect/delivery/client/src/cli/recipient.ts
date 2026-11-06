import { retrieveRecipientCapture, RecipientRetrievalError, type RecipientReadCredential, type RetrievedRecipientCapture } from "../recipient/retrieve.ts";
import { RecipientTransportError } from "../recipient/transport.ts";

const ID = /^[a-f0-9]{32}$/;
export type RecipientCommand =
  | { kind: "fetch"; id: string; out: string; waitSeconds: number; media: boolean }
  | { kind: "inspect-local"; source: string }
  | { kind: "inspect-remote"; id: string; out: string; waitSeconds: number };
export type CliFailure = { category: string; message: string };
export type RecipientCliDependencies = {
  /** Caller supplies a fresh read credential; this boundary must not discover or persist secrets. */
  loadCredential(): Promise<RecipientReadCredential>;
  /** Must publish privately without overwrite and return only after verified output is committed. */
  publish(capture: RetrievedRecipientCapture, destination: string): Promise<
    | { state: "committed"; path: string; evidenceOnlyReceipt?: boolean }
    | { state: "publication-uncertain"; path?: string }
  >;
  inspectLocal(source: string): Promise<string>;
  cancelled?: AbortSignal;
  allowLegacyPlaintext?: boolean;
  resolveIdentity?: () => Promise<{ age: string; signing: Uint8Array }>;
};
export type RecipientCliResult = { ok: true; path: string } | { ok: false; error: CliFailure };

function invalid(message: string): never { throw new CliArgumentError(message); }
export class CliArgumentError extends Error { readonly category = "invalid_arguments"; }
const COMMAND_FAILURES = ["auth_required", "renewal_required", "encryption_identity_missing", "encryption_identity_unavailable",
  "state_unavailable", "stale_snapshot", "revocation_pending", "invalid_server", "cancelled", "destination_exists",
  "publication_refused", "unsafe_directory", "unsafe_file", "operation_failed"] as const;
export type RecipientCommandFailure = typeof COMMAND_FAILURES[number];
/** Closed adapter errors; arbitrary diagnostics never reach CLI output. */
export class RecipientCommandError extends Error {
  readonly category: RecipientCommandFailure;
  constructor(category: RecipientCommandFailure) {
    const admitted = COMMAND_FAILURES.includes(category) ? category : "operation_failed";
    super(admitted); this.name = "RecipientCommandError"; this.category = admitted;
  }
}

/** Parse only fetch/inspect recipient forms; does no credential, filesystem, or network work. */
export function parseRecipientCommand(args: readonly string[]): RecipientCommand {
  if (!args.length) invalid("expected fetch or inspect");
  const [verb, ...rest] = args;
  if (verb === "fetch") {
    let id: string | undefined, out: string | undefined, waitSeconds = 0, media = false, seenWait = false;
    for (let i = 0; i < rest.length; i++) {
      const arg = rest[i]!;
      if (arg === "--media") { if (media) invalid("duplicate --media"); media = true; }
      else if (arg === "--out" || arg === "--wait") {
        const value = rest[++i]; if (value === undefined || value.startsWith("--")) invalid(`missing value for ${arg}`);
        if (arg === "--out") { if (out !== undefined) invalid("duplicate --out"); out = value; }
        else { if (seenWait) invalid("duplicate --wait"); seenWait = true; if (!/^(0|[1-9][0-9]*)$/.test(value)) invalid("invalid --wait"); waitSeconds = Number(value); }
      } else if (arg.startsWith("-") || id !== undefined) invalid("unexpected fetch argument");
      else id = arg;
    }
    if (!id || !ID.test(id)) invalid("capture ID must be 32 lowercase hexadecimal characters");
    if (!out || !out.trim()) invalid("--out is required");
    if (waitSeconds > 900) invalid("--wait must be between 0 and 900 seconds");
    return { kind: "fetch", id, out, waitSeconds, media };
  }
  if (verb === "inspect") {
    let source = "latest", seen = false;
    for (const arg of rest) {
      if (arg.startsWith("-") || seen) invalid("unexpected inspect argument");
      source = arg; seen = true;
    }
    // Match the legacy adapter: only a bare exact canonical ID is remote.
    return ID.test(source)
      ? { kind: "inspect-remote", id: source, out: `./${source}.evidence.apparent`, waitSeconds: 120 }
      : { kind: "inspect-local", source };
  }
  invalid("expected fetch or inspect");
}

export async function runRecipientCommand(command: RecipientCommand, dependencies: RecipientCliDependencies): Promise<RecipientCliResult> {
  try { command = { ...command }; validateCommand(command); } catch (error) { return { ok: false, error: safeFailure(error) }; }
  if (dependencies.cancelled?.aborted) return { ok: false, error: { category: "cancelled", message: "cancelled" } };
  if (command.kind === "inspect-local") {
    try { return { ok: true, path: await dependencies.inspectLocal(command.source) }; }
    catch (error) { return { ok: false, error: safeFailure(error) }; }
  }
  try {
    const credential = await dependencies.loadCredential();
    if (dependencies.cancelled?.aborted) throw new RecipientRetrievalError("cancelled");
    const retrieved = await retrieveRecipientCapture({
      credential, captureId: command.id,
      evidenceOnly: command.kind === "inspect-remote",
      waitSeconds: command.waitSeconds,
      signal: dependencies.cancelled,
      allowLegacyPlaintext: dependencies.allowLegacyPlaintext,
      resolveIdentity: dependencies.resolveIdentity,
    });
    if (dependencies.cancelled?.aborted) throw new RecipientRetrievalError("cancelled");
    return await publishVerified(retrieved, command.out, dependencies.publish);
  } catch (error) { return { ok: false, error: safeFailure(error) }; }
}

function validateCommand(command: RecipientCommand): void {
  if (!command || typeof command !== "object") invalid("invalid command");
  if (command.kind === "inspect-local") {
    if (typeof command.source !== "string" || !command.source) invalid("invalid local source");
    return;
  }
  if ((command.kind !== "fetch" && command.kind !== "inspect-remote") || !ID.test(command.id)
    || typeof command.out !== "string" || !command.out.trim()
    || !Number.isInteger(command.waitSeconds) || command.waitSeconds < 0 || command.waitSeconds > 900
    || command.kind === "fetch" && typeof command.media !== "boolean") invalid("invalid recipient command");
}

/** Project only a committed publication; uncertain outcomes are never retried or cleaned up here. */
export async function publishVerified(
  capture: RetrievedRecipientCapture,
  destination: string,
  publish: RecipientCliDependencies["publish"],
): Promise<RecipientCliResult> {
  const publication = await publish(capture, destination);
  if (publication.state !== "committed") return { ok: false, error: { category: "publication_uncertain", message: "publication outcome is uncertain; inspect the destination before taking further action" } };
  if (capture.evidenceOnly && publication.evidenceOnlyReceipt !== true) return { ok: false, error: { category: "publication_unverified", message: "evidence-only publication lacks its required receipt" } };
  return { ok: true, path: publication.path };
}

function safeFailure(error: unknown): CliFailure {
  if (error instanceof RecipientCommandError) {
    const category = COMMAND_FAILURES.includes(error.category) ? error.category : "operation_failed";
    return { category, message: category };
  }
  if (error instanceof CliArgumentError) return { category: error.category, message: error.message };
  if (error instanceof RecipientRetrievalError || error instanceof RecipientTransportError) return { category: error.category, message: error.category };
  return { category: "operation_failed", message: "recipient operation failed" };
}
