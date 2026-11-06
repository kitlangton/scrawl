import path from "node:path";
import { parseRecipientCommand, type RecipientCliResult } from "./recipient.ts";
import { runRecipientCommandFromState, type StateRecipientOptions } from "./recipient-state.ts";
import type { RecipientRenewalOwner } from "../state/renewal-owner.ts";
import { normalizeRecipientServer } from "../recipient/transport.ts";

type Target = "linux-x64" | "macos-x64";
export type RecipientResourceAdmission = Readonly<{
  artifactDirectory: string; trustedManifestSha256: string; expectedSourceSha: string; target: Target;
}>;
export type RecipientHostResources = StateRecipientOptions["resources"] & {
  readonly protocol: 1; readonly target: Target; readonly sourceSha: string; readonly manifestSha256: string;
  close(): Promise<void>;
};
export type RecipientInspection = Readonly<{ json: boolean; marks: boolean; experiments: readonly string[]; cwd: string; signal?: AbortSignal }>;
export type RecipientHostOptions = {
  /** Trusted composition resolves config and admits the existing sole Rust writer.
   * Stored credential URLs and argv do not establish server or executable approval. */
  resolveRemote(): Promise<{
    directory: string; approvedServer: string; admission: RecipientResourceAdmission;
    writer: { directory: string; approvedServer: string; renewRead: RecipientRenewalOwner };
    allowLegacyPlaintext?: boolean;
  }>;
  /** The published createInstalledNativeResources factory, never a direct fallback. */
  openResources(admission: RecipientResourceAdmission): Promise<RecipientHostResources>;
  /** The Agent-owned inspector supplies its complete output, including evidence warnings. */
  inspect(source: string, options: RecipientInspection): Promise<{ stdout: string; stderr: string }>;
  /** Original CLI working directory supplied by trusted composition, not a renderer. */
  cwd?: string;
  signal?: AbortSignal;
};
export type RecipientHostReceipt = Readonly<{
  exitCode: 0 | 1 | 2; stdout: string; stderr: string;
  publication?: Readonly<{ state: "committed"; path: string } | { state: "uncertain" }>;
}>;
export type RecipientHostResult = Readonly<{
  receipt: RecipientHostReceipt;
  provider: Readonly<{ state: "not_opened" | "retired" }> | Readonly<{
    /** Composition retains this capability when retirement fails; no automatic retries. */
    state: "retained"; retire(): Promise<void>;
  }>;
}>;
const failed = (category: string, exitCode: 1 | 2 = 1): RecipientHostReceipt => ({ exitCode, stdout: "", stderr: `${category}\n` });
/** Project an already-validated command failure, retaining effects despite later receipt failure. */
export function recipientFailureReceipt(result: Extract<RecipientCliResult, { ok: false }>): RecipientHostReceipt {
  const publication = result.publication ? Object.freeze({ ...result.publication })
    : result.error.category === "publication_uncertain" ? { state: "uncertain" as const } : undefined;
  return { ...failed(result.error.category), ...(publication ? { publication } : {}) };
}
const absolute = (value: string) => typeof value === "string" && path.isAbsolute(value) && !value.includes("\0")
  && !value.split(path.sep).includes("..") && path.normalize(value) === value;
function hostTarget(): Target | undefined {
  if (process.arch !== "x64") return;
  return process.platform === "linux" ? "linux-x64" : process.platform === "darwin" ? "macos-x64" : undefined;
}

function parse(args: readonly string[], cwd: string) {
  const argv = [...args], remaining: string[] = [], experiments: string[] = [];
  let json = false, marks = false, positional = false;
  if (argv[0] !== "inspect") return { command: parseRecipientCommand(argv), inspect: Object.freeze({ json, marks, experiments: Object.freeze(experiments), cwd }) };
  for (let i = 1; i < argv.length; i++) {
    const arg = argv[i]!;
    if (!positional && arg === "--") { positional = true; remaining.push(arg); continue; }
    if (!positional && (arg === "--json" || arg === "--marks")) {
      if (arg === "--json") { if (json) throw Error(); json = true; }
      else { if (marks) throw Error(); marks = true; }
    } else if (!positional && (arg === "--experiment" || arg.startsWith("--experiment="))) {
      const inline = arg.startsWith("--experiment=");
      const value = inline ? arg.slice("--experiment=".length) : argv[++i];
      if (value === undefined || !inline && value.length > 1 && value.startsWith("-")) throw Error();
      experiments.push(value);
    } else remaining.push(arg);
  }
  return { command: parseRecipientCommand(["inspect", ...remaining]), inspect: Object.freeze({ json, marks, experiments: Object.freeze(experiments), cwd }) };
}

/** One-shot Node host composition, not an installed locator, binary or route switch.
 * Remote commands require an externally admitted resource tuple and matching sole
 * Rust writer port. No TypeScript credential/journal writer is imported here.
 * The host owns provider lifetime through publication and inspection; failed
 * retirement returns ownership to composition without retrying any operation.
 */
export async function runRecipientHost(args: readonly string[], options: RecipientHostOptions): Promise<RecipientHostResult> {
  let resources: RecipientHostResources | undefined, receipt: RecipientHostReceipt = failed("operation_failed");
  let publication: RecipientHostReceipt["publication"];
  let inspecting = false;
  let signal: AbortSignal | undefined;
  try {
    const { resolveRemote, openResources, inspect } = options, cwd = options.cwd ?? process.cwd(); signal = options.signal;
    if (!absolute(cwd)) throw Error();
    let parsed: ReturnType<typeof parse>;
    try { parsed = parse(args, cwd); } catch { return { receipt: failed("invalid_arguments", 2), provider: { state: "not_opened" } }; }
    if (signal?.aborted) return { receipt: failed("cancelled"), provider: { state: "not_opened" } };
    const inspectSource = async (source: string) => {
      inspecting = true;
      const output = await inspect(source, Object.freeze({ ...parsed.inspect, signal }));
      if (signal?.aborted) throw Error();
      if (typeof output?.stdout !== "string" || typeof output.stderr !== "string") throw Error();
      return { exitCode: 0 as const, stdout: output.stdout, stderr: output.stderr, ...(publication ? { publication } : {}) };
    };
    if (parsed.command.kind === "inspect-local") receipt = await inspectSource(parsed.command.source);
    else {
      if (parsed.command.out.includes("\0") || parsed.command.out.split(path.sep).includes("..")) return { receipt: failed("invalid_arguments", 2), provider: { state: "not_opened" } };
      const command = { ...parsed.command, out: path.resolve(cwd, parsed.command.out) };
      const value = await resolveRemote();
      const directory = value.directory, approvedServer = value.approvedServer, writer = { ...value.writer };
      const admission = Object.freeze({ artifactDirectory: value.admission.artifactDirectory,
        trustedManifestSha256: value.admission.trustedManifestSha256, expectedSourceSha: value.admission.expectedSourceSha,
        target: value.admission.target }), allowLegacyPlaintext = value.allowLegacyPlaintext;
      if (!absolute(directory) || writer.directory !== directory || writer.approvedServer !== approvedServer || typeof writer.renewRead !== "function"
        || allowLegacyPlaintext !== undefined && typeof allowLegacyPlaintext !== "boolean") throw Error();
      normalizeRecipientServer(approvedServer);
      if (!absolute(admission.artifactDirectory) || typeof admission.trustedManifestSha256 !== "string" || !/^[a-f0-9]{64}$/.test(admission.trustedManifestSha256)
        || typeof admission.expectedSourceSha !== "string" || !/^[a-f0-9]{40}$/.test(admission.expectedSourceSha) || !hostTarget() || admission.target !== hostTarget()) throw Error();
      if (signal?.aborted) return { receipt: failed("cancelled"), provider: { state: "not_opened" } };
      resources = await openResources(admission);
      if (resources.protocol !== 1 || resources.target !== admission.target || resources.sourceSha !== admission.expectedSourceSha
        || resources.manifestSha256 !== admission.trustedManifestSha256 || typeof resources.close !== "function") throw Error();
      const result: RecipientCliResult = await runRecipientCommandFromState(command, { directory, approvedServer, resources,
        inspectLocal: async source => source, signal, renewRead: writer.renewRead, allowLegacyPlaintext });
      if (!result.ok) {
        receipt = recipientFailureReceipt(result); publication = receipt.publication;
      } else {
        publication = Object.freeze({ state: "committed", path: result.path });
        receipt = command.kind === "fetch" ? { exitCode: 0, stdout: `${result.path}\n`, stderr: "", publication } : await inspectSource(result.path);
      }
    }
  } catch {
    receipt = { ...failed(signal?.aborted ? "cancelled" : inspecting ? "inspection_failed" : "state_unavailable"), ...(publication ? { publication } : {}) };
  }
  if (!resources) return { receipt, provider: { state: "not_opened" } };
  try { await resources.close(); return { receipt, provider: { state: "retired" } }; }
  catch {
    return { receipt: { ...receipt, exitCode: 1, stderr: `${receipt.stderr}provider_retirement_failed\n` },
      provider: { state: "retained", retire: () => resources!.close() } };
  }
}
