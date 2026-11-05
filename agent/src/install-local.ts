#!/usr/bin/env node
import { pathToFileURL } from 'node:url';
import { realpathSync } from 'node:fs';
import { parseInstallArguments } from './install/arguments.js';
import { installLocal } from './install/core.js';
import { systemInstallerDependencies } from './install/runtime.js';

export { installLocal, parseInstallArguments };

export async function runInstallerCli(args = process.argv.slice(2)): Promise<void> {
  const input = parseInstallArguments(args);
  await installLocal(input, systemInstallerDependencies);
}

function invokedAsCommand(): boolean {
  if (!process.argv[1]) return false;
  try {
    // npm bin entries are symlinks, while import.meta.url identifies the real module.
    return import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href;
  } catch {
    return false;
  }
}

if (invokedAsCommand()) {
  runInstallerCli().catch((error: unknown) => {
    process.stderr.write(`Apparent local installation failed: ${(error as Error).message}\n`);
    process.exitCode = 1;
  });
}
