import { createHash } from "node:crypto";
import path from "node:path";
import type { RetrievedRecipientCapture } from "./retrieve.ts";
import type { Asset } from "./crypto.ts";
import { recipientAssets, validateRecipientReferences } from "./manifest.ts";
import { PrivateDirectoryError, publishPrivateDirectory, type PrivateDirectoryResources, type OwnedPrivateFile } from "./private-directory.ts";

/** The deliberately small privileged boundary. Both methods consume their handles. */
export type RecipientPublicationResources = PrivateDirectoryResources;
export type RecipientPublicationResult =
  | { state: "committed"; path: string; evidenceOnlyReceipt?: boolean }
  | { state: "publication-uncertain" };
export type RecipientPublicationFailure = "invalid_capture" | "invalid_destination" | "unsafe_directory" | "destination_exists" | "cancelled" | "publication_refused";
export class RecipientPublicationError extends Error {
  readonly category: RecipientPublicationFailure;
  constructor(category: RecipientPublicationFailure) { super(category); this.name = "RecipientPublicationError"; this.category = category; }
}
const fail = (category: RecipientPublicationFailure): never => { throw new RecipientPublicationError(category); };
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const cancelled = (signal?: AbortSignal) => { if (signal?.aborted) fail("cancelled"); };
const badUnicode = (s: string) => s.includes("\0") || /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(s);
const safeAssetPath = (name: string) => !badUnicode(name) && !name.startsWith("/") && !name.includes("\\") && name.split("/").every((part) => part !== "" && part !== "." && part !== "..");

/** Recipient policy and evidence marker remain here; only admitted bytes reach the private directory core. */
export async function publishRecipientCapture(
  capture: RetrievedRecipientCapture, destination: string, resources: RecipientPublicationResources,
  options: { signal?: AbortSignal } = {},
): Promise<RecipientPublicationResult> {
  const { signal } = options; cancelled(signal);
  let parentPath: string, basename: string;
  if (typeof destination !== "string" || badUnicode(destination) || destination.split(path.sep).includes("..")) fail("invalid_destination");
  parentPath = path.resolve(path.dirname(destination)); basename = path.basename(destination);
  if (!basename || basename === "." || basename === ".." || badUnicode(basename) || basename.includes(path.sep)) fail("invalid_destination");
  if (!capture || !/^[a-f0-9]{32}$/.test(capture.captureId) || typeof capture.server !== "string" || typeof capture.evidenceOnly !== "boolean" || !Array.isArray(capture.files)) fail("invalid_capture");
  const captureId = capture.captureId, server = capture.server, evidenceOnly = capture.evidenceOnly;
  const validationGroup = evidenceOnly ? "evidence" as const : undefined;
  let copies: OwnedPrivateFile[] = [];
  try {
    const assets: Asset[] = (() => { try { return recipientAssets(capture.files.map(({ path: p, size, sha256 }) => ({ path: p, size, sha256 })), validationGroup); } catch { return fail("invalid_capture"); } })();
    if (assets.length !== capture.files.length) fail("invalid_capture");
    for (let i = 0; i < capture.files.length; i++) {
      const file = capture.files[i]!;
      const asset = assets[i]!;
      if (!safeAssetPath(file.path) || file.path !== asset.path || !(file.bytes instanceof Uint8Array) || file.bytes.length !== asset.size) fail("invalid_capture");
      const bytes = new Uint8Array(file.bytes); copies.push({ path: asset.path, bytes });
      if (hash(bytes) !== asset.sha256) fail("invalid_capture");
    }
    try { validateRecipientReferences(copies.map((file, i) => ({ ...assets[i]!, bytes: file.bytes })), { origin: server, accountId: "", captureId, senderGrantId: "", ...(validationGroup ? { group: validationGroup } : {}) }); }
    catch { fail("invalid_capture"); }
    if (evidenceOnly) {
      const marker = Buffer.from(JSON.stringify({ version: 1, captureId, server,
        media: "not downloaded", guidance: "Use apparent fetch <id> --media --out <new-directory> for the complete recording" }));
      if (marker.length > 2_000_000) { marker.fill(0); fail("invalid_capture"); }
      copies.push({ path: ".evidence-only.json", bytes: marker });
    }
    const result = await publishPrivateDirectory(parentPath, path.resolve(destination), copies, resources, { signal });
    if (result.state === "publication-uncertain") return result;
    return { ...result, ...(evidenceOnly ? { evidenceOnlyReceipt: true } : {}) };
  } catch (error) {
    if (error instanceof RecipientPublicationError) throw error;
    if (error instanceof PrivateDirectoryError) {
      if (error.category === "invalid_input") return fail("invalid_capture");
      return fail(error.category);
    }
    fail("publication_refused");
  } finally { for (const file of copies) file.bytes.fill(0); }
  return fail("publication_refused");
}
