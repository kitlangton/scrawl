#!/usr/bin/env node
import { pathToFileURL } from 'node:url';
import { parseInstallArguments } from './install/arguments.js';
import { installLocal } from './install/core.js';
import { systemInstallerDependencies } from './install/runtime.js';

export { installLocal, parseInstallArguments };

export async function runInstallerCli(args = process.argv.slice(2)): Promise<void> {
  const input = parseInstallArguments(args);
  await installLocal(input, systemInstallerDependencies);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  runInstallerCli().catch((error: unknown) => {
    process.stderr.write(`Apparent local installation failed: ${(error as Error).message}\n`);
    process.exitCode = 1;
  });
}
