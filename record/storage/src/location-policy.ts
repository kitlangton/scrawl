import type { LocationSnapshot, NativeLocationFacts } from './location-contract.js';

export type LocationCandidate = {
  revision: string;
  snapshot?: LocationSnapshot;
  probePath?: string;
};

function boundedPath(value: unknown): value is string {
  return typeof value === 'string' && new TextEncoder().encode(value).length <= 4096 && !value.includes('\0');
}

function absolute(value: string): boolean { return value.startsWith('/'); }

function configuredPath(value: string, homePath: string): { savedPath: string; effectivePath: string } | undefined {
  const savedPath = value.trim();
  if (!boundedPath(savedPath) || savedPath.length === 0) return undefined;
  const tilde = savedPath[0] === '~' && (savedPath.length === 1 || savedPath[1] === '/' || savedPath[1] === '\\');
  const rest = tilde ? savedPath.slice(1).replace(/^[/\\]+/, '') : undefined;
  // Match PathBuf::join's spelling, retaining dot segments that native capture
  // resolution also retains (normalizing them breaks usage-path identity checks).
  const effectivePath = tilde
    ? rest ? `${homePath.endsWith('/') ? homePath : homePath + '/'}${rest}` : homePath
    : savedPath;
  if (!absolute(effectivePath) || !boundedPath(effectivePath)) return undefined;
  return { savedPath, effectivePath };
}

function displayPath(value: string, homePath: string): string {
  if (value === homePath) return '~';
  const prefix = homePath.endsWith('/') ? homePath : homePath + '/';
  if (!value.startsWith(prefix)) return value;
  const relative = value.slice(homePath.length).replace(/^\/+/, '');
  return relative ? `~/${relative}` : '~';
}

function snapshot(savedPath: string | null, effectivePath: string, source: LocationSnapshot['source'], homePath: string): LocationSnapshot {
  if (!boundedPath(effectivePath) || !boundedPath(homePath)) throw new Error('invalid_location_facts');
  const display = displayPath(effectivePath, homePath);
  if (!boundedPath(display)) throw new Error('invalid_location_facts');
  return { savedPath, effectivePath, displayPath: display, custom: savedPath !== null, source };
}

/** Selects product intent from native facts; only the OS adapter decides availability. */
export function locationCandidate(facts: NativeLocationFacts): LocationCandidate {
  if (!/^[a-f0-9]{16}$/.test(facts.revision) || !boundedPath(facts.homePath) ||
      !boundedPath(facts.builtinPath) || !absolute(facts.homePath) || !absolute(facts.builtinPath) ||
      !facts.environment || typeof facts.environment.present !== 'boolean' ||
      !(facts.environment.value === null || boundedPath(facts.environment.value)) ||
      facts.environment.present !== (facts.environment.value !== null)) throw new Error('invalid_location_facts');

  const saved = facts.saved?.kind === 'string' ? configuredPath(facts.saved.value, facts.homePath) : undefined;
  if (facts.environment.present) {
    return { revision: facts.revision,
      snapshot: snapshot(saved?.savedPath ?? null, facts.environment.value!, 'environment', facts.homePath) };
  }
  if (saved) return { revision: facts.revision, probePath: saved.effectivePath };
  return { revision: facts.revision, snapshot: snapshot(null, facts.builtinPath, 'builtin', facts.homePath) };
}

export function resolveLocationCandidate(facts: NativeLocationFacts, available?: boolean): LocationSnapshot {
  const candidate = locationCandidate(facts);
  if (candidate.snapshot) return candidate.snapshot;
  if (typeof available !== 'boolean') throw new Error('location_probe_required');
  const saved = configuredPath((facts.saved as { kind: 'string'; value: string }).value, facts.homePath)!;
  return available
    ? snapshot(saved.savedPath, saved.effectivePath, 'configured', facts.homePath)
    : snapshot(saved.savedPath, facts.builtinPath, 'fallback', facts.homePath);
}

export function normalizeSelectedPath(value: unknown): string | undefined {
  if (!boundedPath(value)) return undefined;
  const normalized = value.trim();
  return normalized.length > 0 && boundedPath(normalized) && absolute(normalized) ? normalized : undefined;
}
