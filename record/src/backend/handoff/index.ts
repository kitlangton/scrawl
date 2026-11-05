import { isAbsolute, resolve } from "node:path";
import { assembleProcessingOutput, type ProcessingInput } from "../processing/index.js";

const CLI_INSTALL = "Once per agent session, install or update to the latest Apparent CLI: https://github.com/tryapparent/apparent#agents";
const VIDEO_PROOF_REQUEST = "Use make-it-apparent for complex visible changes; share verified video proof or explain the blocker.";

/** Already verified saved bundle facts and semantic processing facts. This formatter performs no IO. */
export interface LocalHandoffInput {
  /** Caller-attested result from an existing publication producer; this formatter does not verify it. */
  savedBundle: { status: "saved"; bundlePath: string };
  processing: ProcessingInput;
  /** Optional Engine publication warnings, kept verbatim and in caller order. */
  warnings?: string[];
  /** Absolute directory containing preserved full frames when the delivered frames are cropped. */
  uncroppedFramesPath?: string;
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
  if (input.uncroppedFramesPath !== undefined &&
    (!isAbsolute(input.uncroppedFramesPath) || resolve(input.uncroppedFramesPath) !== input.uncroppedFramesPath))
    throw new TypeError("uncroppedFramesPath must be absolute and normalized");
  for (const warning of input.warnings ?? []) {
    if (typeof warning !== "string" || !warning.trim()) throw new TypeError("warnings must be non-empty strings");
  }
}

/**
 * Format the existing local copy/paste prompt from verified semantic facts.
 * It does not read/authenticate a bundle, publish it, or perform clipboard/paste effects.
 */
export function formatLocalHandoff(input: LocalHandoffInput): LocalHandoffOutput {
  validate(input);
  const frameText = new Set(input.processing.frames.map((frame) => frame.text?.trim()).filter(Boolean));
  const processing = {
    ...input.processing,
    transcript: {
      ...input.processing.transcript,
      segments: input.processing.transcript.segments.filter((segment) => !frameText.has(segment.text.trim())),
      text: frameText.has(input.processing.transcript.text.trim()) ? "" : input.processing.transcript.text,
    },
  };
  const semantic = assembleProcessingOutput(processing).summaryMarkdown;
  const lines = [
    `apparent inspect ${shellQuote(input.processing.bundlePath)}`,
    "",
    CLI_INSTALL,
    "",
    "Inspect contact-sheet.png and relevant frames/ in that folder before making changes.",
  ];
  if (input.uncroppedFramesPath)
    lines[lines.length - 1] += ` Uncropped images are in ${input.uncroppedFramesPath}/.`;
  if (input.requestVideoProof) lines[lines.length - 1] += ` ${VIDEO_PROOF_REQUEST}`;
  lines.push("");
  lines.push(...(input.warnings ?? []).map((warning) => `Warning: ${warning.trim()}`));
  if (input.warnings?.length) lines.push("");
  lines.push(semantic.trimEnd());
  return { text: `${lines.join("\n")}\n` };
}
