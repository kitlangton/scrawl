import type { LocationSnapshot, RetentionPolicy, RetentionPreset, RetentionView } from '../bridge';

export type RetentionField = { present: boolean; value: number | null };
export type StoredRetention = { mediaDays: RetentionField; bundleDays: RetentionField };

export const RETENTION_OPTIONS: readonly { id: RetentionPreset; label: string }[] = [
  { id: 'week', label: 'Video for 7 Days' },
  { id: 'month', label: 'Video for 30 Days' },
  { id: 'forever', label: 'Forever' },
];

const presets: Record<RetentionPreset, RetentionPolicy> = {
  week: { mediaDays: 7, bundleDays: 30 },
  month: { mediaDays: 30, bundleDays: 90 },
  forever: { mediaDays: null, bundleDays: null },
};

function validField(field: unknown): field is RetentionField {
  return !!field && typeof field === 'object' &&
    Object.keys(field).length === 2 && Object.hasOwn(field, 'present') && Object.hasOwn(field, 'value') &&
    typeof (field as RetentionField).present === 'boolean' &&
    ((field as RetentionField).value === null ||
      (Number.isInteger((field as RetentionField).value) &&
        (field as RetentionField).value! >= 0 && (field as RetentionField).value! <= 0xffff_ffff)) &&
    ((field as RetentionField).present || (field as RetentionField).value === null);
}

export function isStoredRetention(value: unknown): value is StoredRetention {
  if (!value || typeof value !== 'object') return false;
  const policy = value as StoredRetention;
  return Object.keys(policy).length === 2 && Object.hasOwn(policy, 'mediaDays') && Object.hasOwn(policy, 'bundleDays') &&
    validField(policy.mediaDays) && validField(policy.bundleDays);
}

export function retentionForPreset(preset: RetentionPreset): RetentionPolicy {
  return { ...presets[preset] };
}

export function isRetentionPolicy(value: unknown): value is RetentionPolicy {
  if (!value || typeof value !== 'object') return false;
  const policy = value as RetentionPolicy;
  const valid = (days: unknown) => days === null ||
    (Number.isInteger(days) && (days as number) >= 0 && (days as number) <= 0xffff_ffff);
  return Object.keys(policy).length === 2 && valid(policy.mediaDays) && valid(policy.bundleDays);
}

function effective(field: RetentionField, fallback: number | null): number | null {
  return field.present ? field.value : fallback;
}

function days(value: number): string { return `${value} ${value === 1 ? 'day' : 'days'}`; }

export function retentionView(stored: StoredRetention, customOverride?: boolean): { custom: boolean; retention: RetentionView } {
  const mediaDays = effective(stored.mediaDays, 7);
  const bundleDays = effective(stored.bundleDays, 30);
  const preset = (Object.keys(presets) as RetentionPreset[]).find(id =>
    presets[id].mediaDays === mediaDays && presets[id].bundleDays === bundleDays) ?? null;
  const bin = 'Trash';
  const note = mediaDays === null && bundleDays === null ? '' :
    mediaDays !== null && bundleDays !== null
      ? `Video and audio go to the ${bin} after ${days(mediaDays)}, the rest of each capture after ${days(bundleDays)}.`
      : mediaDays !== null
        ? `Video and audio go to the ${bin} after ${days(mediaDays)}.`
        : `Captures go to the ${bin} after ${days(bundleDays!)}.`;
  return {
    custom: customOverride ?? (stored.mediaDays.present || stored.bundleDays.present),
    retention: { preset, options: RETENTION_OPTIONS.map(option => ({ ...option })), note },
  };
}

export function isLocationSnapshot(value: unknown): value is LocationSnapshot {
  if (!value || typeof value !== 'object') return false;
  const snapshot = value as LocationSnapshot;
  const pathValue = (item: unknown) => typeof item === 'string' && item.length <= 4096 && !item.includes('\0');
  if (!(snapshot.savedPath === null || pathValue(snapshot.savedPath)) ||
      !pathValue(snapshot.effectivePath) || !pathValue(snapshot.displayPath) ||
      typeof snapshot.custom !== 'boolean' || snapshot.custom !== (snapshot.savedPath !== null) ||
      !['environment', 'configured', 'builtin', 'fallback'].includes(snapshot.source)) return false;
  if (snapshot.source === 'configured' && snapshot.savedPath === null) return false;
  if (snapshot.source === 'builtin' && snapshot.savedPath !== null) return false;
  if (snapshot.source === 'fallback' && snapshot.savedPath === null) return false;
  return true;
}
