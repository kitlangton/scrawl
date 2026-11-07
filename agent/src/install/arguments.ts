import { isAbsolute, resolve } from 'node:path';
import type { InstallerInput } from './types.js';

const SHA256 = /^[a-f0-9]{64}$/;
const OPTION_TO_FIELD = new Map<string, keyof InstallerInput>([
  ['--agent', 'agentPath'],
  ['--agent-sha256', 'agentSha256'],
  ['--native-root', 'nativeRoot'],
  ['--manifest-sha256', 'manifestSha256'],
  ['--prefix', 'prefix'],
  ['--capture-root', 'captureRoot'],
  ['--capture-manifest-sha256', 'captureManifestSha256'],
]);

export function parseInstallArguments(args: string[]): InstallerInput {
  const fields = new Map<keyof InstallerInput, string>();
  for (let index = 0; index < args.length; index += 2) {
    const option = args[index];
    const field = OPTION_TO_FIELD.get(option);
    if (!field) throw new Error(`unknown or malformed option: ${option ?? '<missing>'}`);
    if (fields.has(field)) throw new Error(`duplicate option: ${option}`);
    const value = args[index + 1];
    if (value === undefined || value.startsWith('--')) throw new Error(`missing value for ${option}`);
    fields.set(field, value);
  }
  for (const field of ['agentPath', 'agentSha256', 'nativeRoot', 'manifestSha256', 'prefix'] as const) {
    if (!fields.has(field)) throw new Error(`missing required option: ${field}`);
  }

  const input = Object.fromEntries(fields) as InstallerInput;
  if (!SHA256.test(input.agentSha256)) throw new Error('invalid Agent SHA-256');
  if (!SHA256.test(input.manifestSha256)) throw new Error('invalid manifest SHA-256');
  if ((input.captureRoot === undefined) !== (input.captureManifestSha256 === undefined)) {
    throw new Error('--capture-root and --capture-manifest-sha256 must be supplied together');
  }
  if (input.captureManifestSha256 !== undefined && !SHA256.test(input.captureManifestSha256)) {
    throw new Error('invalid capture manifest SHA-256');
  }
  if (!isAbsolute(input.prefix) || resolve(input.prefix) !== input.prefix || input.prefix === '/') {
    throw new Error('prefix must be a fresh canonical absolute path');
  }
  if (!isAbsolute(input.agentPath) || !isAbsolute(input.nativeRoot) || (input.captureRoot !== undefined && !isAbsolute(input.captureRoot))) {
    throw new Error('Agent archive and native root paths must be absolute');
  }
  return input;
}
