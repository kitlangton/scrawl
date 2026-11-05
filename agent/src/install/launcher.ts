function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

export function makeLauncher(nodePath: string, cliPath: string, entryPath: string, runtimePath: string): string {
  return [
    '#!/bin/sh',
    `export APPARENT_ENGINE_FRAMES_EXECUTABLE=${shellQuote(entryPath)}`,
    `export APPARENT_ENGINE_RUNTIME=${shellQuote(runtimePath)}`,
    `exec ${shellQuote(nodePath)} ${shellQuote(cliPath)} "$@"`,
    '',
  ].join('\n');
}
