import { useEffect, useRef, useState } from 'react';
import { Button } from '@apparent/ui/button';
import {RetentionController, type RetentionState, type RetentionResult, type UsageResult} from '@apparent/record-storage';
import {retentionPorts} from './storage/transport';

function problem(result: RetentionResult | UsageResult | undefined) {
  if (!result || result.status === 'connected' || result.status === 'cancelled') return '';
  if (result.status === 'error') {
    if (result.code === 'outcome_unknown') return 'The save outcome is unknown. Refresh retention before trying again.';
    if (result.code === 'config_write_failed') return 'Retention could not be saved. Refresh to read the unchanged preference.';
    if (result.code === 'conflict') return 'Storage changed during this request. Refresh before trying again.';
    if (result.code === 'usage_unavailable') return 'Folder usage is unavailable or the scan exceeded its limit. Refresh usage to try again.';
    return 'Storage is unavailable. Refresh when the Rust app is responsive.';
  }
  if (result.status === 'incompatible') return 'This Rust backend does not support retention and usage yet.';
  if (result.status === 'disabled') return 'Experimental Storage is disabled in Rust.';
  return 'Not connected to Rust. Open Apparent and refresh.';
}

export function Retention({ effectivePath, settings = false }: { effectivePath: string; settings?: boolean }) {
  const [state, setState] = useState<RetentionState>({preset: '', busy: true, scanning: false});
  const controller = useRef<RetentionController | undefined>(undefined);
  const {result, usage, preset, busy, scanning} = state;
  const request = (operation: 'read' | 'save' | 'reset') => controller.current?.request(operation);
  const measure = () => controller.current?.measure();
  useEffect(() => {
    const current = new RetentionController(retentionPorts, effectivePath);
    controller.current = current;
    const unsubscribe = current.subscribe(setState);
    void current.request('read');
    return () => { unsubscribe(); current.dispose(); if (controller.current === current) controller.current = undefined; };
  }, [effectivePath]);
  const snapshot = result?.status === 'connected' ? result : undefined;
  return <div className="space-y-4 border-t border-border pt-4">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <h3 className="font-semibold">Retention</h3>
      <Button variant="outline" disabled={busy || scanning} onClick={() => void request('read')}>Refresh retention</Button>
    </div>
    <div role="status" aria-live="polite" className="space-y-2 text-sm">
      {busy && <p>Waiting for retention…</p>}
      {problem(result) && <p>{problem(result)}</p>}
      {snapshot && <p>{snapshot.retention.note || 'Recordings and captures are kept forever.'}</p>}
    </div>
    {snapshot && <div className="flex flex-wrap items-end gap-3">
      <label className="grid gap-2 text-sm" htmlFor="retention-preset">Keep recordings
        <select id="retention-preset" className="h-9 rounded-md border border-border bg-surface px-3 text-sm focus-visible:outline-2 focus-visible:outline-primary"
          disabled={busy || scanning} value={preset} onChange={event => controller.current?.select(event.target.value)}>
          <option value="" disabled>Custom policy (config file)</option>
          {snapshot.retention.options.map(option => <option key={option.id} value={option.id}>{option.label}</option>)}
        </select>
      </label>
      <Button disabled={busy || scanning || !preset || preset === snapshot.retention.preset} onClick={() => void request('save')}>Save retention</Button>
      <Button variant="outline" disabled={busy || scanning || !snapshot.custom} onClick={() => void request('reset')}>Reset retention</Button>
    </div>}
    <p className="text-sm text-muted-foreground">{settings
      ? 'Changes apply to cleanup shortly after saving. Cleanup waits until recording finishes. After changing the capture folder, restart Apparent so cleanup uses the new folder.'
      : 'Saving schedules the existing cleanup worker in about 5 seconds; it waits while recording. File edits apply on the next pass (normally hourly). A running worker keeps its startup folder until Apparent restarts. Saving does not move captures or run cleanup in this window.'}</p>
    <div className="flex flex-wrap items-center justify-between gap-3">
      <h3 className="font-semibold">Capture-folder usage</h3>
      <Button variant="outline" disabled={busy || scanning} onClick={() => void measure()}>Refresh usage</Button>
    </div>
    <div role="status" aria-live="polite" className="space-y-2 text-sm">
      {scanning && <p>Calculating folder usage…</p>}
      {problem(usage) && <p>{problem(usage)}</p>}
      {usage?.status === 'connected' && <><p className="font-medium">{usage.usage.formatted} in captures</p>
        <p className="break-all text-muted-foreground">{usage.usage.displayPath}</p></>}
    </div>
    <p className="text-sm text-muted-foreground">Logical file size at the last scan, not free disk space. Refresh after background cleanup; no continuous scanning.</p>
  </div>;
}
