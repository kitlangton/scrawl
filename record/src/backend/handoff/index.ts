import { assembleProcessingOutput, type ProcessingInput } from "../processing/index.js";

const CLI_INSTALL = "Once per agent session, install or update to the latest Apparent CLI: https://github.com/tryapparent/apparent#agents";
const VIDEO_PROOF_REQUEST = "Use make-it-apparent for complex visible changes; share verified video proof or explain the blocker.";

/** Already verified saved bundle facts and semantic processing facts. This formatter performs no IO. */
export interface LocalHandoffInput {
  /** Caller-attested result from an existing publication producer; this formatter does not verify it. */
  savedBundle: { status: "saved"; bundlePath: string };
  processing: ProcessingInput;
  /** Mirrors the existing agentDemoVideo opt-out exactly; caller supplies the effective value. */
  requestVideoProof: boolean;
}

export interface LocalHandoffOutput {
  text: string;
}

function shellQuote(path: string): string {
  return `'${path.replaceAll("'", "'\\''")}'`;
}

function validate(input: LocalHandoffInput): void {
  if (!input || typeof input.requestVideoProof !== "boolean")
    throw new TypeError("requestVideoProof must be supplied explicitly");
  if (input.savedBundle?.status !== "saved" || input.savedBundle.bundlePath !== input.processing.bundlePath)
    throw new TypeError("savedBundle must identify the processed bundle with saved status");
}

/**
 * Format the existing local copy/paste prompt from verified semantic facts.
 * It does not read/authenticate a bundle, publish it, or perform clipboard/paste effects.
 */
export function formatLocalHandoff(input: LocalHandoffInput): LocalHandoffOutput {
  validate(input);
  const semantic = assembleProcessingOutput(input.processing, "local-clipboard").summaryMarkdown;
  const lines = [
    `apparent inspect ${shellQuote(input.processing.bundlePath)}`,
    "",
    CLI_INSTALL,
    "",
    "Inspect contact-sheet.png and relevant frames/ in that folder before making changes.",
  ];
  const fullFramesDirectory = input.processing.autocrop?.fullFramesDirectory;
  if (fullFramesDirectory)
    lines[lines.length - 1] += ` Uncropped images are in ${fullFramesDirectory}/.`;
  if (input.requestVideoProof) lines[lines.length - 1] += ` ${VIDEO_PROOF_REQUEST}`;
  lines.push("");
  lines.push(semantic.trimEnd());
  return { text: `${lines.join("\n")}\n` };
}
