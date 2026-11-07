function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

export function makeLauncher(nodePath: string, cliPath: string, entryPath: string, runtimePath: string,
  captureEntry?: string, media2Entry?: string): string {
  const lines = [
    '#!/bin/sh',
    `export APPARENT_ENGINE_FRAMES_EXECUTABLE=${shellQuote(entryPath)}`,
    `export APPARENT_ENGINE_RUNTIME=${shellQuote(runtimePath)}`,
  ];
  if (captureEntry) lines.push(`export APPARENT_ENGINE_CAPTURE_EXECUTABLE=${shellQuote(captureEntry)}`);
  if (media2Entry) lines.push(`export APPARENT_ENGINE_MEDIA2_BIN=${shellQuote(media2Entry)}`);
  lines.push(`exec ${shellQuote(nodePath)} ${shellQuote(cliPath)} "$@"`, '');
  return lines.join('\n');
}
