#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { access, realpath } from 'node:fs/promises';
import { isAbsolute, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export type Options = { sourceRoot: string; inputs?: string; output?: string; plan: boolean };
const defaultSourceRoot = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const usage = `Build a complete Linux x64 development desktop bundle using the repository's existing builder.

Usage:
  node --experimental-strip-types enablement/desktop/build-dev-app.ts [options]

Options:
  --source-root DIR   Checkout containing composition/app/scripts/build-omarchy.ts (default: this checkout)
  --inputs FILE       Explicit admitted-native-inputs.json (required to build)
  --output DIR        Fresh output directory (required to build)
  --plan              Show builder and prerequisites without starting a build
  -h, --help          Show this help

Prerequisites: Linux x64; Node.js 22.18+ with the source checkout's dependencies installed;
the existing build-omarchy.ts builder and its accepted app dependencies; admitted native
artifact input JSON. The builder rejects dirty source and refuses an existing output.
The result is a real packaged desktop bundle only when the input includes the pinned
GUI closure. This command never substitutes a synthetic shell. It does not install or launch.
`;

export function parseArgs(argv: string[], cwd = process.cwd()): Options | 'help' {
  const options: Options = { sourceRoot: defaultSourceRoot, plan: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '-h' || arg === '--help') return 'help';
    if (arg === '--plan') { options.plan = true; continue; }
    if (arg === '--source-root' || arg === '--inputs' || arg === '--output') {
      const value = argv[++i];
      if (!value || value.startsWith('--')) throw new Error(`${arg} requires a value`);
      if (arg === '--source-root') options.sourceRoot = resolve(cwd, value);
      else if (arg === '--inputs') options.inputs = resolve(cwd, value);
      else options.output = resolve(cwd, value);
      continue;
    }
    throw new Error(`Unknown option: ${arg}\n\n${usage}`);
  }
  return options;
}

export async function run(options: Options, runProcess = spawn) {
  const sourceRoot = await realpath(options.sourceRoot);
  const builder = resolve(sourceRoot, 'composition/app/scripts/build-omarchy.ts');
  try { await access(builder); }
  catch { throw new Error(`Full desktop builder not found at ${builder}. Use --source-root for a checkout that contains the accepted build-omarchy.ts; this command will not fall back to a synthetic shell.`); }
  if (process.platform !== 'linux' || process.arch !== 'x64') throw new Error('The complete Omarchy desktop builder currently requires Linux x64.');
  if (options.plan) {
    return { builder, sourceRoot, inputs: options.inputs ?? '(required admitted native input JSON)', output: options.output ?? '(required fresh output directory)', requires: ['clean committed source checkout', 'Linux x64', 'Node.js 22.18+ and source dependencies', 'admitted Capture/Media/config inputs', 'pinned GUI AppImage closure with source/executable hashes'] };
  }
  if (!options.inputs || !options.output) throw new Error(`--inputs and --output are required. See --help for prerequisites.`);
  if (!isAbsolute(options.inputs) || !isAbsolute(options.output)) throw new Error('Inputs and output paths must resolve to absolute paths.');
  const child = runProcess(process.execPath, ['--experimental-strip-types', builder, options.inputs, options.output], { cwd: sourceRoot, stdio: 'inherit', env: process.env });
  return await new Promise<void>((resolvePromise, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0 ? resolvePromise() : reject(new Error(`Builder failed${signal ? ` with signal ${signal}` : ` with exit code ${code}`}. Check the builder output above; preserve its source and artifact evidence.`)));
  });
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    const parsed = parseArgs(process.argv.slice(2));
    if (parsed === 'help') console.log(usage);
    else {
      const result = await run(parsed);
      if (result) console.log(JSON.stringify(result, null, 2));
    }
  } catch (error) { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; }
}
