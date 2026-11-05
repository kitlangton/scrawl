/** Match capture/agent/context.rs mark_entries/mark_text for existing bundle readers. */
export function markEntries(bundle: Record<string, any>) {
  const onFrame = (mark: any, frame: any) => frame.markId === mark.id || (!!frame.file && mark.frame === frame.file);
  return bundle.marks.map((mark: any) => {
    const frame = bundle.keyframes.find((k: any) => onFrame(mark, k));
    const shown = frame ? bundle.marks.filter((m: any) => m.strokes.length && onFrame(m, frame)).sort((a: any, b: any) => a.t0 - b.t0) : [];
    const pos = shown.indexOf(mark);
    const names: Record<string, string> = { circle: 'circled', underline: 'underlined', line: 'drew a line', arrow: 'drew an arrow', tap: 'tapped', mark: 'marked', flag: 'flagged this moment', pin: `pin ${mark.pin ?? 1}` };
    let text = names[mark.shape];
    const label = mark.on?.trim();
    if (label) {
      const quoted = label.includes('“') || label.startsWith('<') ? label : `“${label}”`;
      text += (mark.shape === 'arrow' ? ' pointing to ' : ['pin', 'line'].includes(mark.shape) ? ' on ' : ' ') + quoted;
    }
    if (mark.strokes.length > 1) text += ` (${mark.strokes.length} strokes)`;
    return { keyframe: frame?.index ?? null, number: pos < 0 ? 1 : pos + 1, numbered: pos >= 0 && shown.length >= 2,
      id: mark.id, t0: mark.t0, t1: mark.t1,
      time: `${Math.floor(mark.t0 / 60000)}:${String(Math.floor(mark.t0 / 1000) % 60).padStart(2, '0')}`,
      shape: mark.shape, on: mark.on ?? null, strokes: mark.strokes.length, text, frame: mark.frame ?? null };
  }).sort((a: any, b: any) => a.t0 - b.t0);
}
export function markLine(mark: ReturnType<typeof markEntries>[number]): string {
  const digits = '①②③④⑤⑥⑦⑧⑨⑩⑪⑫⑬⑭⑮⑯⑰⑱⑲⑳';
  const number = mark.numbered ? digits[mark.number - 1] ?? `(${mark.number})` : '';
  return `#${mark.keyframe ?? '-'} ✎${number} ${mark.time} ${mark.text} · ${mark.id}`;
}
