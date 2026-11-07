import { keyframeTexts } from './semantic/context.js';

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
    name = clean(device.name);
    if (device.app !== undefined && device.app !== null) name += ` · ${clean(device.app)}`;
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
    case 'app_event': return `app: ${clean(event.label)}`;
    case 'app_change': return `switched to ${clean(event.app)}`;
    default: return label ? `${clean(event.kind)}: ${label}` : clean(event.kind);
  }
}
function typedNoteText(bundle: Data, event: Data): string {
  const frame = bundle.marks.find((mark: Data) => mark.pin === event.pin)?.frame;
  const body = String(event.text ?? '').split(/\r?\n/u).map((line: string) => `    ${line}`).join('\n');
  return `Typed note · pin ${event.pin ?? 1}${frame ? ` · ${frame}` : ''}:\n${body}`;
}

function semanticEvents(bundle: Data): Array<{ t: number; text: string }> {
  const out: Array<{ t: number; text: string }> = [];
  for (const event of bundle.events) {
    if (!['note', 'app_event'].includes(event.kind)) continue;
    out.push({ t: event.t ?? 0, text: event.kind === 'note' ? typedNoteText(bundle, event) : eventText(event) });
  }
  for (const change of bundle.contextChanges ?? []) out.push({
    t: change.t, text: `${value(change.key)} ${change.value == null ? 'cleared' : `→ ${value(change.value)}`}`,
  });
  if (!bundle.keyframes.length) {
    for (const segment of bundle.transcript.segments) out.push({t:segment.t0,text:`“${segment.text}”`});
    if (!bundle.transcript.segments.length && clean(bundle.transcript.text)) out.push({t:0,text:`“${clean(bundle.transcript.text)}”`});
  }
  out.sort((a, b) => a.t - b.t);
  return out;
}

/** Generate inspect text from the manifest only. No media is opened or decoded. */
export function semanticSummary(bundle: Data, bundleDir: string, agentContext = false, evidenceOnly = false, hasAudio = false): string {
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
  for (const [key, item] of Object.entries(bundle.context ?? {}).sort(([a],[b])=>a<b?-1:a>b?1:0)) context.push(`${value(key)}: ${value(item)}`);
  if (context.length) lines.push(context.join(' · '));
  if (clean(bundle.note)) lines.push(`Note: ${String(bundle.note).trim()}`);
  if (bundle.interactionTelemetry === 'videoOnly') lines.push('Interaction context: video only; clicks and Shift ink were not collected. A visible click in the video will not appear in events, marks or click keyframes.');
  if (bundle.imageQualityWarning) lines.push(`Image quality: ${clean(bundle.imageQualityWarning)}`);
  if (bundle.autocrop) lines.push(`Cropped to: ${clean(bundle.autocrop.label)} (full frames in ${clean(bundle.autocrop.fullFrames)}/)`);
  if (bundle.source !== 'iphone' && bundle.context?.microphoneCapture !== 'off') {
    const deferredAudio = !!bundle.audio && evidenceOnly;
    if (!hasAudio && !deferredAudio) lines.push('No microphone audio was saved. Check the microphone and try again.');
    else if (!clean(bundle.transcript.text) && bundle.transcript.missing)
      lines.push(`No transcript: ${clean(bundle.transcript.missing.reason)}`);
    else if (!clean(bundle.transcript.text) && bundle.transcript.source === 'none')
      lines.push('No transcript: Recognition did not run. The audio is saved.');
    else if (!clean(bundle.transcript.text)) lines.push('No words transcribed. Your audio was saved—check it or try again closer to the microphone.');
  }

  if (bundle.keyframes.length) {
    const richFrames = agentContext ? keyframeTexts(bundle) : undefined;
    const timeline = semanticEvents(bundle);
    let shownScreen = startScreen;
    if (!agentContext) for (const item of timeline.filter((event) => event.t < (bundle.keyframes[0].t ?? 0)))
      lines.push(`  · ${clock(item.t)} ${item.text}`);
    for (let i = 0; i < bundle.keyframes.length; i++) {
      const frame = bundle.keyframes[i];
      const said = clean(frame.said);
      const isFlag = bundle.marks.some((mark: Data) => mark.id === frame.markId && mark.shape === 'flag');
      const index = frame.index ?? i + 1, t = frame.t ?? 0;
      const screen = screenAt(bundle, t);
      const screenMarker = screen && screen !== shownScreen ? ` ⟨${value(screen)}⟩` : '';
      shownScreen = screen;
      const actionMarker = isFlag ? ' ⚑' : !agentContext && said && frame.reason === 'click' ? ` [${clean(frame.event).split(' (')[0]}]` : '';
      if (agentContext && richFrames) {
        const rich = richFrames[i];
        lines.push(`- #${index} (${clock(t)})${screenMarker}${actionMarker}${rich.head ? ` ${rich.head}` : ''}`);
        for (const line of rich.lines) lines.push(`  ${line}`);
      } else lines.push(`- #${index} (${clock(t)})${screenMarker}${actionMarker}${said ? ` “${said}”` : ` ${clean(frame.event)}`}`);
      if (!agentContext) {
        const next = bundle.keyframes[i + 1]?.t ?? Number.POSITIVE_INFINITY;
        for (const item of timeline.filter((event) => event.t >= t && event.t < next))
          lines.push(`  · ${clock(item.t)} ${item.text}`);
      }
    }
  } else {
    const timeline = semanticEvents(bundle);
    if (!timeline.length && clean(bundle.transcript.text)) lines.push(`“${clean(bundle.transcript.text)}”`);
    else for (const item of timeline) lines.push(`· ${clock(item.t)} ${item.text}`);
  }
  return `${lines.join('\n')}\n`;
}
