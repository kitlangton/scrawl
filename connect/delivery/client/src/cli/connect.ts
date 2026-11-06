import { CliArgumentError } from "./recipient.ts";

export type ConnectCommand =
  | { kind: "login"; server?: string; scope: "read" | "upload"; consentUpload: boolean; browser: "default" | "open" | "print-only" }
  | { kind: "status"; account: boolean }
  | { kind: "disable" }
  | { kind: "logout" }
  | { kind: "retry" }
  | { kind: "upload"; bundle: string }
  | { kind: "shares-list" }
  | { kind: "shares-revoke"; id: string };

function invalid(message: string): never { throw new CliArgumentError(message); }

/** Parse the existing Connect/Cloud command shape. This is only an argument grammar: callers
 * retain responsibility for CLI presentation and for all separate consent/approval checks. */
export function parseConnectCommand(args: readonly string[]): ConnectCommand {
  const argv = [...args];
  if (argv[0] === "connect" || argv[0] === "cloud") argv.shift();
  if (!argv.length) invalid("expected a Connect command");
  const [verb, ...rest] = argv;
  if (verb === "login") {
    let server: string | undefined, scope: "read" | "upload" = "read";
    let seenScope = false, consentUpload = false, browser: "default" | "open" | "print-only" = "default";
    let seenBrowser = false;
    for (let i = 0; i < rest.length; i++) {
      const arg = rest[i]!;
      if (arg === "--") { if (i + 1 < rest.length) invalid("unexpected login argument"); break; }
      if (arg === "--consent-upload") { if (consentUpload) invalid("duplicate --consent-upload"); consentUpload = true; continue; }
      if (arg === "--browser" || arg === "--no-browser") {
        if (seenBrowser) invalid("conflicting or duplicate browser option");
        seenBrowser = true; browser = arg === "--browser" ? "open" : "print-only"; continue;
      }
      const option = valueOption(arg, "--server", "--scope");
      if (option) {
        const { name, inline } = option;
        const value = inline ?? rest[++i];
        if (value === undefined || (inline === undefined && value.length > 1 && value.startsWith("-"))) invalid(`missing value for ${name}`);
        if (name === "--server") { if (server !== undefined) invalid("duplicate --server"); server = value; }
        else {
          if (seenScope) invalid("duplicate --scope"); seenScope = true;
          if (value !== "read" && value !== "upload") invalid("invalid --scope"); scope = value;
        }
        continue;
      }
      invalid("unexpected login argument");
    }
    return Object.freeze({ kind: "login", ...(server === undefined ? {} : { server }), scope, consentUpload, browser });
  }
  if (verb === "status") {
    const options = withoutTerminalSeparator(rest);
    if (!options.length) return Object.freeze({ kind: "status", account: false });
    if (options.length === 1 && options[0] === "--account") return Object.freeze({ kind: "status", account: true });
    invalid(rest[0] === "--account" ? "duplicate or unexpected status argument" : "unexpected status argument");
  }
  if (verb === "disable" || verb === "logout" || verb === "retry") {
    if (withoutTerminalSeparator(rest).length) invalid(`unexpected ${verb} argument`);
    return Object.freeze({ kind: verb });
  }
  if (verb === "upload") {
    const positional = onePositional(rest, "upload");
    if (positional === undefined) invalid("missing bundle path");
    return Object.freeze({ kind: "upload", bundle: positional });
  }
  if (verb === "shares") {
    const [action, ...tail] = rest;
    if (action === "list") { if (withoutTerminalSeparator(tail).length) invalid("unexpected shares list argument"); return Object.freeze({ kind: "shares-list" }); }
    if (action === "revoke") {
      const id = onePositional(tail, "shares revoke");
      if (id === undefined) invalid("missing share management ID");
      return Object.freeze({ kind: "shares-revoke", id });
    }
    invalid("expected shares list or revoke");
  }
  invalid("expected login, status, disable, logout, retry, upload, or shares");
}

function valueOption(arg: string, ...names: string[]): { name: string; inline?: string } | undefined {
  for (const name of names) {
    if (arg === name) return { name };
    if (arg.startsWith(`${name}=`)) return { name, inline: arg.slice(name.length + 1) };
  }
  return undefined;
}

function onePositional(args: readonly string[], context: string): string | undefined {
  let value: string | undefined, positionalOnly = false;
  for (const arg of args) {
    if (!positionalOnly && arg === "--") { positionalOnly = true; continue; }
    if (!positionalOnly && arg.length > 1 && arg.startsWith("-")) invalid(`unexpected ${context} option`);
    if (value !== undefined) invalid(`unexpected ${context} argument`);
    value = arg;
  }
  return value;
}

function withoutTerminalSeparator(args: readonly string[]): readonly string[] {
  return args.at(-1) === "--" ? args.slice(0, -1) : args;
}
