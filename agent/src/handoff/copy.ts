import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { lstat } from 'node:fs/promises';
import { formatLocalHandoff } from '@apparent/record';
import type { ProcessingInput, ProcessingWarning } from '@apparent/record/processing';
import type { SemanticFrameText } from '@apparent/record';
import { createNativeHostPort } from '@apparent/native-host';
import { createNativeClipboardPort, MAX_UTF8_TEXT_BYTES, type NativeClipboardPort } from '@apparent/native-host/clipboard';
import type { NativeHostPort } from '@apparent/app-contracts/native';
import { resolveBundle, readManifest, checkImageReference, hasMicrophoneSamples, configuredExperiment } from '../bundle.js';
import { keyframeTexts } from '../semantic/context.js';
import { readCanonicalRecordOutput } from '@apparent/record/processing/bundle';

type Ports = { host: Pick<NativeHostPort, 'readSnapshot'>; clipboard: NativeClipboardPort };
export type CopyResult = { status: 'applied'; digest: string; byteLength: number } |
  { status: 'refused'; reason: string } | { status: 'outcome-unknown' };

/** No retry: a missing or invalid reply may follow an actual clipboard mutation. */
export async function writeCopyText(text: string, ports: () => Ports = () => ({ host: createNativeHostPort(), clipboard: createNativeClipboardPort() })): Promise<CopyResult> {
  if (/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(text) || text.includes('\0')) return { status: 'refused', reason: 'invalid-text' };
  const bytes = Buffer.from(text, 'utf8');
  if (bytes.length > MAX_UTF8_TEXT_BYTES) return { status: 'refused', reason: 'too-large' };
  const expected = { digest: createHash('sha256').update(bytes).digest('hex'), byteLength: bytes.length };
  let bound: Ports;
  let observed: Awaited<ReturnType<NativeHostPort['readSnapshot']>>;
  try { bound = ports(); observed = await bound.host.readSnapshot(); }
  catch { return { status: 'refused', reason: 'unavailable' }; }
  try {
    const result = await bound.clipboard.writeText(text, { processEpoch: observed.processEpoch, revision: observed.revision });
    if (result.status === 'applied') return result.digest === expected.digest && result.byteLength === expected.byteLength
      ? { status: 'applied', ...expected } : { status: 'outcome-unknown' };
    return result;
  } catch { return { status: 'outcome-unknown' }; }
}

/** Translate validated legacy manifest facts; Record retains handoff text policy. */
export async function localCopyText(input = 'latest'): Promise<string> {
  const bundlePath = await resolveBundle(input);
  for (const marker of ['unfinished.json', '.unsafe-video', '.audio-recording', '.recovering', '.media-pending']) {
    if (await lstat(join(bundlePath, marker)).catch((error: NodeJS.ErrnoException) => { if (error.code !== 'ENOENT') throw error; return null; }))
      throw new Error('Bundle is unfinished or media is pending; no clipboard handoff was attempted');
  }
  const { value: bundle } = await readManifest(bundlePath);
  const output = readCanonicalRecordOutput(bundle);
  const references = [bundle.contactSheet, ...bundle.keyframes.map((frame: any) => frame.file), ...bundle.marks.map((mark: any) => mark.frame).filter(Boolean), ...(bundle.autocrop ? [bundle.autocrop.fullFrames, bundle.autocrop.fullContactSheet] : [])];
  for (const reference of references) await checkImageReference(bundlePath, reference);
  const warnings: ProcessingWarning[] = [];
  warnings.push(...(output?.warnings ?? []));
  if (bundle.interactionTelemetry === 'videoOnly') warnings.push({ kind: 'videoOnlyInteractionTelemetry' });
  if (bundle.imageQualityWarning) warnings.push({ kind: 'imageQuality', message: bundle.imageQualityWarning });
  if (bundle.source !== 'iphone' && bundle.context?.microphoneCapture !== 'off') {
    if (!await hasMicrophoneSamples(bundlePath, bundle.audio)) warnings.push({ kind: 'noMicrophoneAudio' });
    else if (!output && !bundle.transcript.text.trim()) {
      if (bundle.transcript.missing) warnings.push({ kind: 'noTranscript', message: bundle.transcript.missing.reason });
      else if (bundle.transcript.source === 'none') warnings.push({ kind: 'noTranscript', message: 'Recognition did not run. The audio is saved.' });
      else warnings.push({ kind: 'noWordsTranscribed' });
    }
  }
  const frames: ProcessingInput['frames'] = bundle.keyframes.map((frame: any, i: number) => ({ index: frame.index ?? i + 1, timeMs: frame.t ?? 0, absolutePath: join(bundlePath, frame.file), text: frame.said ?? undefined,
    event: frame.event ?? undefined, reason: ['click', 'flag'].includes(frame.reason) ? frame.reason : 'other', markId: frame.markId ?? undefined,
    screenName: bundle.screens?.find((screen: any) => screen.t0 <= (frame.t ?? 0) && (frame.t ?? 0) < screen.t1)?.name }));
  const semanticFrameText: SemanticFrameText[] | undefined = await configuredExperiment('agentContext') && frames.length
    ? keyframeTexts(bundle).map((chunk, i) => ({ frameIndex: frames[i].index, head: chunk.head, lines: chunk.lines }))
    : undefined;
  if (semanticFrameText && semanticFrameText.length !== frames.length) throw new Error('Agent semantic text does not cover every bundle keyframe');
  const processing: ProcessingInput = {
    bundlePath, durationMs: bundle.durationMs, target: bundle.target, device: bundle.device,
    context: bundle.context, contextChanges: bundle.contextChanges?.map((event: any) => ({ timeMs: event.t, key: event.key, value: event.value ?? undefined })),
    screenAtStart: bundle.screens?.find((screen: any) => screen.t0 === 0)?.name,
    transcript: { text: bundle.transcript.text, segments: bundle.transcript.segments.map((segment: any) => ({ startMs: segment.t0, text: segment.text })) },
    transcriptStatus: output?.transcriptStatus,
    acceptedStrokeDocument: output?.acceptedStrokeDocument ? { path: join(bundlePath, output.acceptedStrokeDocument.path), strokeCount: output.acceptedStrokeDocument.strokeCount } : output?.acceptedStrokeDocument,
    frames, semanticFrameText: semanticFrameText ? { mode: 'rich-context', frames: semanticFrameText } : undefined,
    marks: bundle.marks.map((mark: any) => ({ id: mark.id, shape: mark.shape, pin: mark.pin ?? undefined, framePath: mark.frame ? join(bundlePath, mark.frame) : undefined })),
    events: bundle.events.filter((event: any) => ['click', 'app_event', 'note'].includes(event.kind)).map((event: any) => event.kind === 'app_event'
      ? { kind: 'appEvent', timeMs: event.t, label: event.label } : event.kind === 'note' ? { kind: 'note', timeMs: event.t, pin: event.pin ?? 1, text: event.text } : { kind: 'click', timeMs: event.t }),
    note: bundle.note ?? undefined, warnings,
    autocrop: bundle.autocrop ? { label: bundle.autocrop.label, fullFramesDirectory: join(bundlePath, bundle.autocrop.fullFrames) } : undefined,
  };
  // This is a completed local artifact supplied by the caller, not native publication authentication.
  const text = formatLocalHandoff({ savedBundle: { status: 'saved', bundlePath }, processing, requestVideoProof: await configuredExperiment('agentDemoVideo') }).text;
  const prefix = 'apparent inspect ';
  if (!text.startsWith(prefix)) throw new Error('Record handoff command contract changed');
  // Bind the public formatter's legacy executable token to the independently installed Agent CLI.
  return `apparent-agent inspect ${text.slice(prefix.length)}`;
}
