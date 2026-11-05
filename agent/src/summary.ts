import { markEntries, markLine } from './inspection.js';

type Data = Record<string, any>;
const clock = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')}`;
const clean = (value: unknown) => typeof value === 'string' ? value.replace(/\s+/gu, ' ').trim() : '';
const value = (raw: unknown) => {
  const text = clean(raw);
  return [...text].length > 60 ? `${[...text].slice(0, 59).join('').trimEnd()}…` : text;
};
const quote = (text: string) => text.includes('“') || text.startsWith('<') ? text : `“${text}”`;
function screenAt(bundle: Data, t: number): string | undefined {
  return [...(bundle.screens ?? [])].reverse().find((span: Data) => span.t0 <= t && (t < span.t1 || span.t1 >= bundle.durationMs))?.name;
}
function targetName(bundle: Data): string {
  const target = bundle.target;
  let name = clean(target.app) || 'Display';
  if (target.windowTitle && target.windowTitle !== target.app) name += ` ${quote(target.windowTitle)}`;
  const device = bundle.device;
  if (device) {
    name = clean(device.name) || 'Device';
    if (device.app) name += ` · ${clean(device.app)}`;
    const short = clean(device.appShortVersion), build = clean(device.appVersion);
    if (short && build && short !== build) name += ` ${short} (${build})`;
    else if (short) name += ` ${short}`;
    else if (build) name += ` (${build})`;
  }
  return name;
}

function eventText(event: Data): string {
  const label = clean(event.label);
  switch (event.kind) {
    case 'click': return `clicked${event.role ? ` ${clean(event.role)}` : ''}${label ? ` ${quote(label)}` : ''}`;
    case 'drag': return `dragged${label ? ` ${quote(label)}` : ''}`;
    case 'scroll': return `scrolled${label ? ` ${quote(label)}` : ''}`;
    case 'point': return label ? `pointed at ${quote(label)}` : 'pointed';
    case 'key': return `pressed ${clean(event.shortcut)}`;
    case 'note': return `Typed note · pin ${event.pin ?? 1}:\n${String(event.text ?? '').split(/\r?\n/u).map((line: string) => `    ${line}`).join('\n')}`;
    case 'appEvent': return `app: ${clean(event.label)}`;
    case 'appChange': return `switched to ${clean(event.app)}${event.windowTitle ? ` — ${quote(clean(event.windowTitle))}` : ''}`;
    default: return label ? `${clean(event.kind)}: ${label}` : clean(event.kind);
  }
}

function semanticEvents(bundle: Data, agentContext: boolean): Array<{ t: number; text: string; rank: number }> {
  const out: Array<{ t: number; text: string; rank: number }> = [];
  if (!bundle.keyframes.length) for (const segment of bundle.transcript.segments) out.push({ t: segment.t0, text: `“${segment.text}”`, rank: 0 });
  if (!bundle.keyframes.length && bundle.transcript.segments.length === 0 && clean(bundle.transcript.text))
    out.push({ t: 0, text: `“${clean(bundle.transcript.text)}”`, rank: 0 });
  if (agentContext) for (const mark of markEntries(bundle)) out.push({ t: mark.t0, text: markLine(mark), rank: 1 });
  for (const event of bundle.events) {
    if (!agentContext && !['note', 'appEvent'].includes(event.kind)) continue;
    out.push({ t: event.t ?? 0, text: eventText(event), rank: 2 });
  }
  for (const change of bundle.contextChanges ?? []) out.push({
    t: change.t, text: `${value(change.key)} ${change.value == null ? 'cleared' : `→ ${value(change.value)}`}`, rank: 3,
  });
  out.sort((a, b) => a.t - b.t || a.rank - b.rank);
  if (!agentContext) return out;
  // In the interleaved mode each event remains adjacent to its timestamped speech/action.
  return out;
}

/** Generate inspect text from the manifest only. No media is opened or decoded. */
export function semanticSummary(bundle: Data, bundleDir: string, agentContext = false, evidenceOnly = false): string {
  const lines = [`[apparent ${clock(bundle.durationMs)} · ${targetName(bundle)} · ${bundleDir}]`];
  if (bundle.destination) {
    const destination = bundle.destination;
    lines.push(`For session: ${clean(destination.title) || clean(destination.session)} (${clean(destination.session)})${destination.url ? ` · page ${clean(destination.url)}` : ''}`);
  }
  if (bundle.keyframes.length) lines.push(bundle.events.some((event: Data) => ['click', 'note'].includes(event.kind))
    ? 'Open contact-sheet.png and frames/ in that folder before changing anything: the marks and words only make sense on the images, which can show more than the text does.'
    : 'Visuals are saved in this bundle, but are optional for this text-first handoff. Use apparent zoom if the words need screen context; contact-sheet.png and frames/ are also available there.');
  const context: string[] = [];
  const startScreen = screenAt(bundle, 0);
  if (startScreen) context.push(`Screen: ${value(startScreen)}`);
  for (const [key, item] of Object.entries(bundle.context ?? {})) context.push(`${value(key)}: ${value(item)}`);
  if (context.length) lines.push(context.join(' · '));
  if (clean(bundle.note)) lines.push(`Note: ${String(bundle.note).trim()}`);
  if (bundle.interactionTelemetry === 'videoOnly') lines.push('Interaction context: timed CLI video only; clicks and Shift ink were not collected. A visible click in the video will not appear in events, marks or click keyframes. Use the app\'s recording shortcut for interaction context.');
  if (bundle.imageQualityWarning) lines.push(`Image quality: ${clean(bundle.imageQualityWarning)}`);
  if (bundle.autocrop) lines.push(`Cropped to: ${clean(bundle.autocrop.label)} (full frames in ${clean(bundle.autocrop.fullFrames)}/)`);
  if (bundle.source !== 'iphone' && bundle.context?.microphoneCapture !== 'off') {
    const deferredAudio = !!bundle.audio && evidenceOnly;
    if (!bundle.audio && !deferredAudio) lines.push('No microphone audio was saved. Check the microphone and try again.');
    else if (!clean(bundle.transcript.text) && bundle.transcript.missing)
      lines.push(`No transcript: ${clean(bundle.transcript.missing.reason)}`);
    else if (!clean(bundle.transcript.text) && bundle.transcript.source === 'none')
      lines.push('No transcript: Recognition did not run. The audio is saved.');
    else if (!clean(bundle.transcript.text)) lines.push('No words transcribed. Your audio was saved—check it or try again closer to the microphone.');
  }

  if (bundle.keyframes.length) {
    const timeline = semanticEvents(bundle, agentContext);
    let shownScreen = startScreen;
    for (const item of timeline.filter((event) => event.t < (bundle.keyframes[0].t ?? 0)))
      lines.push(`  · ${clock(item.t)} ${item.text}`);
    for (let i = 0; i < bundle.keyframes.length; i++) {
      const frame = bundle.keyframes[i];
      const said = clean(frame.said);
      const isFlag = bundle.marks.some((mark: Data) => mark.id === frame.markId && mark.shape === 'flag');
      const index = frame.index ?? i + 1, t = frame.t ?? 0;
      const screen = screenAt(bundle, t);
      const screenMarker = screen && screen !== shownScreen ? ` ⟨${value(screen)}⟩` : '';
      shownScreen = screen;
      const actionMarker = isFlag ? ' ⚑' : !agentContext && frame.reason === 'click' ? ` [${clean(frame.event).split(' (')[0]}]` : '';
      lines.push(`- #${index} (${clock(t)})${screenMarker}${actionMarker}${said ? ` “${said}”` : ` ${clean(frame.event)}`}`);
      const next = bundle.keyframes[i + 1]?.t ?? Number.POSITIVE_INFINITY;
      for (const item of timeline.filter((event) => event.t >= t && event.t < next))
        lines.push(`  · ${clock(item.t)} ${item.text}`);
    }
  } else {
    const timeline = semanticEvents(bundle, agentContext);
    if (!timeline.length && clean(bundle.transcript.text)) lines.push(`“${clean(bundle.transcript.text)}”`);
    else for (const item of timeline) lines.push(`· ${clock(item.t)} ${item.text}`);
  }
  return `${lines.join('\n')}\n`;
}
