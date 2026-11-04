import type { CaptureLocation, NativeRetentionResult, RetentionPreset, RetentionResult } from '../bridge';
import { isLocationSnapshot, isStoredRetention, retentionForPreset, retentionView } from './policy';

function projectLocation(result: CaptureLocation): CaptureLocation {
  if (result.status !== 'connected') return result;
  return isLocationSnapshot(result.storage) ? result : { status: 'error', code: 'invalid_response' };
}

export async function readCaptureLocation(): Promise<CaptureLocation> {
  const result = await window.apparent?.readCaptureLocation();
  return result ? projectLocation(result) : { status: 'disabled' };
}

export async function changeCaptureLocation(): Promise<CaptureLocation> {
  const result = await window.apparent?.changeCaptureLocation();
  return result ? projectLocation(result) : { status: 'disabled' };
}

export async function resetCaptureLocation(): Promise<CaptureLocation> {
  const result = await window.apparent?.resetCaptureLocation();
  return result ? projectLocation(result) : { status: 'disabled' };
}

function project(result: NativeRetentionResult): RetentionResult {
  if (result.status !== 'connected') return result;
  if (!isStoredRetention(result.policy)) return { status: 'error', code: 'invalid_response' };
  return { status: 'connected', ...retentionView(result.policy, result.custom) };
}

export async function readRetention(): Promise<RetentionResult> {
  const result = await window.apparent?.readRetention();
  return result ? project(result) : { status: 'disabled' };
}

export async function saveRetention(preset: RetentionPreset): Promise<RetentionResult> {
  const result = await window.apparent?.saveRetention(retentionForPreset(preset));
  return result ? project(result) : { status: 'disabled' };
}

export async function resetRetention(): Promise<RetentionResult> {
  const result = await window.apparent?.resetRetention();
  return result ? project(result) : { status: 'disabled' };
}
