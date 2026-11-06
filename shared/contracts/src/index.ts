import type { ComponentType } from 'react';
import type { StreamHostDependencies } from './host.js';

export const APP_SHELL_API = 1 as const;
export const REACT_VERSION = '19.2.0' as const;

export interface StreamManifest {
  schemaVersion: 1;
  id: string;
  version: string;
  label: string;
  shellApi: 1;
  reactVersion: typeof REACT_VERSION;
  entry: 'index.js';
  sha256: string;
}

export interface StreamModule {
  id: string;
  Screen: ComponentType<{ hostDependencies?: StreamHostDependencies }>;
  Settings: ComponentType<{ hostDependencies?: StreamHostDependencies }>;
}

const manifestFields = new Set([
  'schemaVersion', 'id', 'version', 'label', 'shellApi', 'reactVersion', 'entry', 'sha256',
]);
const slugPattern = /^[a-z][a-z0-9-]{0,63}$/;
const versionPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const sha256Pattern = /^[a-f0-9]{64}$/;

export function parseStreamManifest(value: unknown): StreamManifest {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new TypeError('Stream manifest must be an object');
  }

  const record = value as Record<string, unknown>;
  const keys = Object.keys(record);
  if (keys.length !== manifestFields.size || keys.some((key) => !manifestFields.has(key))) {
    throw new TypeError('Stream manifest has missing or unknown fields');
  }
  if (record.schemaVersion !== 1 || record.shellApi !== APP_SHELL_API || record.reactVersion !== REACT_VERSION) {
    throw new TypeError('Unsupported stream manifest or runtime version');
  }
  if (typeof record.id !== 'string' || !slugPattern.test(record.id)) {
    throw new TypeError('Invalid stream id');
  }
  if (typeof record.version !== 'string' || !versionPattern.test(record.version)) {
    throw new TypeError('Invalid stream version');
  }
  if (typeof record.label !== 'string' || record.label.trim().length === 0 || record.label.length > 120) {
    throw new TypeError('Invalid stream label');
  }
  if (record.entry !== 'index.js') {
    throw new TypeError('Stream entry must be index.js');
  }
  if (typeof record.sha256 !== 'string' || !sha256Pattern.test(record.sha256)) {
    throw new TypeError('Invalid stream SHA-256');
  }

  return {
    schemaVersion: 1,
    id: record.id,
    version: record.version,
    label: record.label,
    shellApi: APP_SHELL_API,
    reactVersion: REACT_VERSION,
    entry: 'index.js',
    sha256: record.sha256,
  };
}
