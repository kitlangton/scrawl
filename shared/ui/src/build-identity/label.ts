export const UNKNOWN_BUILD_IDENTITY = 'build unknown';

/** Reduce the versioned backend label to its commit and tracked-edit marker. */
export function formatBuildIdentity(label: unknown): string {
  if (typeof label !== 'string') return UNKNOWN_BUILD_IDENTITY;
  const match = /^(?:Version\s+)?\S+\s+·\s+([0-9a-f]{7})(\s+\+modified)?$/i.exec(label.trim());
  if (!match) return UNKNOWN_BUILD_IDENTITY;
  return `${match[1].toLowerCase()}${match[2] ? ' +modified' : ''}`;
}
