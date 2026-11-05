type Data = Record<string, any>;
type Item = {
  t: number;
  mark?: Data;
  number?: number;
  text?: string;
  note?: boolean;
};
type FrameText = { head: string; lines: string[] };
const clock = (ms: number) =>
  `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
const compact = (s: unknown) =>
  typeof s === "string" ? s.split(/\s+/u).filter(Boolean).join(" ") : "";
const short = (s: unknown) => {
  const text = compact(s);
  return [...text].length > 60
    ? `${[...text].slice(0, 59).join("").trimEnd()}…`
    : text;
};
const q = (s: string) => (s.trim() ? `“${s.trim()}”` : "");
const quoteLabel = (s: string) =>
  s.includes("“") || s.startsWith("<") ? s : `“${s}”`;
const circs = [
  "①",
  "②",
  "③",
  "④",
  "⑤",
  "⑥",
  "⑦",
  "⑧",
  "⑨",
  "⑩",
  "⑪",
  "⑫",
  "⑬",
  "⑭",
  "⑮",
  "⑯",
  "⑰",
  "⑱",
  "⑲",
  "⑳",
];
const num = (n: number) => circs[n - 1] ?? `(${n})`;
const markOn = (m: Data, k: Data) =>
  k.markId === m.id || (!!k.file && m.frame === k.file);
const marksOn = (k: Data, marks: Data[]) =>
  marks
    .filter((m) => m.strokes?.length && markOn(m, k))
    .sort((a, b) => a.t0 - b.t0);
const norm = (s: string) =>
  [...s]
    .filter((c) => /[\p{L}\p{N}]/u.test(c))
    .join("")
    .toLowerCase();
function wordsPerFrame(bundle: Data): Data[][] | undefined {
  const words = [...(bundle.transcript.words ?? [])]
    .filter((w: Data) => compact(w.text))
    .sort((a: Data, b: Data) => a.t0 - b.t0);
  if (!words.length) return;
  let at = 0;
  const result: Data[][] = [];
  for (const frame of bundle.keyframes) {
    const wanted = norm(frame.said ?? "");
    let got = "",
      part: Data[] = [];
    while (got.length < wanted.length && at < words.length) {
      got += norm(words[at].text);
      part.push(words[at++]);
    }
    if (got !== wanted) return;
    result.push(part);
  }
  return result;
}
function joinWords(words: Data[]): string {
  let out = "";
  for (const w of words) {
    const unit = String(w.text).trim();
    const attached = /^[,.!?;:’')\]”]/u.test(unit),
      open = /[(\[“]$/u.test(out);
    if (out && !attached && !open) out += " ";
    out += unit;
  }
  return out;
}
function clauses(words: Data[]): number[] {
  const starts: number[] = [];
  let count = 0;
  for (let i = 0; i < words.length; i++) {
    const prev = String(words[i - 1]?.text ?? "")
      .trimEnd()
      .replace(/["'”’\])]+$/u, "");
    const t = String(words[i].text)
      .trimEnd()
      .replace(/["'”’\])]+$/u, "");
    const gap = i > 0 ? words[i].t0 - words[i - 1].t1 : 0;
    const start =
      i === 0 ||
      /[.!?;:]$/u.test(prev) ||
      (count >= 3 && prev.endsWith(",")) ||
      gap >= 1500;
    if (start) {
      starts.push(i);
      count = 0;
    }
    count++;
    void t;
  }
  return starts;
}
function splitAt(t: number, words: Data[], starts: number[]): number {
  const inside = words.findIndex((w) => w.t0 >= t);
  if (inside < 0) return t;
  if (starts.includes(inside)) return Math.min(t, words[inside].t0);
  const ci = [...starts].reverse().find((i) => i < inside) ?? 0;
  if (t - words[ci].t0 <= 3000) return words[ci].t0;
  const end = starts.find((i) => i > inside) ?? words.length;
  if (words[end - 1].t1 - t <= 1200)
    return words[end]?.t0 ?? words[end - 1].t1 + 1;
  return t;
}
function describeMark(m: Data, n?: number): string {
  const on = compact(m.on),
    base =
      (
        {
          circle: "circled",
          underline: "underlined",
          line: "drew a line",
          arrow: "drew an arrow",
          pin: `pin ${m.pin ?? 1}`,
          tap: "tapped",
          mark: "marked",
          flag: "flagged this moment",
        } as Record<string, string>
      )[m.shape] ?? "marked";
  let text = `✎${n === undefined ? "" : num(n)} ${clock(m.t0)} ${base}`;
  if (on)
    text +=
      (
        { arrow: " pointing to ", pin: " on ", line: " on " } as Record<
          string,
          string
        >
      )[m.shape] ?? " ";
  if (on) text += quoteLabel(on);
  if (m.strokes?.length > 1) text += ` (${m.strokes.length} strokes)`;
  return text;
}
function at([x, y]: number[]): string {
  return `x ${x.toFixed(2)}, y ${y.toFixed(2)}`;
}
function direction(dx: number, dy: number, threshold: number): string {
  const h = Math.abs(dx) > threshold ? (dx > 0 ? "right" : "left") : "";
  const v = Math.abs(dy) > threshold ? (dy > 0 ? "down" : "up") : "";
  const parts =
    Math.abs(dx) > 2 * Math.abs(dy)
      ? [h]
      : Math.abs(dy) > 2 * Math.abs(dx)
        ? [v]
        : [v, h];
  return parts.filter(Boolean).join(" and ") || "a little";
}
function describe(e: Data): string | undefined {
  switch (e.kind) {
    case "click": {
      let s = e.button === "touch" ? "tapped" : "clicked";
      if (e.role) s += ` ${e.role}`;
      if (e.label) s += ` ${quoteLabel(e.label)}`;
      if (
        process.platform === "linux" &&
        e.button !== "touch" &&
        !e.role &&
        !e.label
      )
        s += " (element unavailable)";
      return s;
    }
    case "drag": {
      const l = compact(e.label);
      if (l === "long press") return `long-pressed (${at([e.x0, e.y0])})`;
      if (l.startsWith("long press:"))
        return `long-pressed “${l.slice(11).trim()}” (${at([e.x0, e.y0])})`;
      return `${e.button === "touch" ? "swiped" : "dragged"}${l ? ` ${quoteLabel(l)}` : ""} ${direction(e.x1 - e.x0, e.y1 - e.y0, 0.04)} (${at([e.x0, e.y0])} → ${at([e.x1, e.y1])})`;
    }
    case "scroll":
      return `scrolled${e.label ? ` ${quoteLabel(e.label)}` : ""} ${direction(e.dx, e.dy, 0)}`;
    case "point":
      return e.label
        ? `pointed at ${quoteLabel(e.label)} (${at([e.x, e.y])})`
        : `pointed at (${at([e.x, e.y])})`;
    case "key":
      return `pressed ${e.shortcut}`;
    case "app_event":
      return e.label;
    case "app_change":
      return `switched to ${e.app}`;
    default:
      return undefined;
  }
}
function settled(events: Data[]): Data[] {
  const sorted = [...events].sort((a, b) => a.t - b.t),
    out: Data[] = [];
  for (const e of sorted) {
    let prior = -1;
    for (let i = out.length - 1; i >= 0; i--) {
      const p = out[i];
      if (
        (e.kind === "click" &&
          p.kind === "click" &&
          p.button === e.button &&
          p.label === e.label &&
          e.t - p.t <= 650 &&
          Math.hypot(e.x - p.x, e.y - p.y) <= 0.03) ||
        (e.kind === "scroll" &&
          p.kind === "scroll" &&
          p.label === e.label &&
          e.t - p.t < 1500) ||
        (e.kind === "point" &&
          p.kind === "point" &&
          p.label === e.label &&
          e.t - p.t < 1000)
      ) {
        prior = i;
        break;
      }
    }
    if (prior >= 0) {
      if (e.kind === "scroll") {
        e.dx += out[prior].dx;
        e.dy += out[prior].dy;
      }
      out.splice(prior, 1);
    }
    out.push(e);
  }
  return out;
}
function eventLines(bundle: Data): Array<[number, string]> {
  const events = settled(bundle.events),
    scrubs = events
      .filter(
        (e) =>
          e.kind === "app_event" &&
          !bundle.device &&
          String(e.label).includes(" scrubbed to "),
      )
      .map((e) => e.t);
  const out: Array<[number, string]> = [];
  let drag:
    | { t: number; end: number; verb: string; name: string; dirs: string[] }
    | undefined;
  const flush = () => {
    if (!drag) return;
    let text = drag.verb + (drag.name ? ` ${quoteLabel(drag.name)}` : "");
    const d = drag.dirs[0];
    if (drag.verb !== "scrubbed" && drag.verb !== "long-pressed") {
      if (drag.dirs.every((x) => x === d) && d !== "a little") text += ` ${d}`;
      else if (!drag.dirs.every((x) => x === d)) text += " back and forth";
    }
    if (drag.dirs.length > 1 && drag.verb !== "scrubbed")
      text += ` ×${drag.dirs.length}`;
    out.push([drag.t, text]);
    drag = undefined;
  };
  for (const e of events) {
    if (e.kind === "drag") {
      const l = compact(e.label),
        touch = e.button === "touch";
      const long = l.startsWith("long press"),
        scrub = [
          "slider",
          "scroll bar",
          "scrubber",
          "timeline",
          "progress",
        ].some((x) => l.toLowerCase().includes(x));
      const dist = Math.hypot(e.x1 - e.x0, e.y1 - e.y0);
      const text = long
        ? {
            verb: "long-pressed",
            name: l === "long press" ? "" : l.slice(11).trim(),
            dir: "",
          }
        : l || dist >= 0.15
          ? {
              verb: long
                ? "long-pressed"
                : scrub
                  ? "scrubbed"
                  : touch
                    ? "swiped"
                    : "dragged",
              name: long ? l.slice(11).trim() : l,
              dir: direction(e.x1 - e.x0, e.y1 - e.y0, 0.04),
            }
          : undefined;
      if (
        text &&
        !scrubs.some((s: number) => s + 1500 >= e.t && s <= e.t1 + 1500)
      ) {
        if (
          drag &&
          drag.verb === text.verb &&
          drag.name === text.name &&
          e.t <= drag.end + 3000
        ) {
          drag.end = Math.max(drag.end, e.t1);
          drag.dirs.push(text.dir);
        } else {
          flush();
          drag = {
            t: e.t,
            end: e.t1,
            verb: text.verb,
            name: text.name,
            dirs: [text.dir],
          };
        }
      }
      continue;
    }
    let text = describe(e);
    if (
      e.kind === "note" ||
      (e.kind === "point" && !e.label) ||
      (e.kind === "app_event" && bundle.device)
    )
      continue;
    if (text) out.push([e.t, text]);
  }
  flush();
  for (const change of bundle.contextChanges ?? [])
    out.push([
      change.t,
      `${short(change.key)} ${change.value == null ? "cleared" : `→ ${short(change.value)}`}`,
    ]);
  const windows = bundle.windows ?? [];
  const normal = (s: Data) =>
    s.windows.filter(
      (w: Data) => w.layer === 0 && w.regular !== false && w.app !== "Apparent",
    );
  for (let i = 1; i < windows.length; i++) {
    const before = normal(windows[i - 1]),
      after = normal(windows[i]),
      opened = after.filter(
        (w: Data) => !before.some((b: Data) => b.id === w.id),
      ),
      closed = before.filter(
        (w: Data) => !after.some((a: Data) => a.id === w.id),
      );
    if (opened.length + closed.length > 4)
      out.push([
        windows[i].t,
        `on-screen windows changed (${opened.length} opened, ${closed.length} closed)`,
      ]);
    else {
      for (const w of closed)
        out.push([windows[i].t, `${w.app} window closed`]);
      for (const w of opened)
        out.push([windows[i].t, `${w.app} window opened`]);
    }
  }
  flush();
  out.sort((a, b) => a[0] - b[0]);
  const folded: Array<[number, string, number, number]> = [];
  for (const [t, text] of out) {
    const p = folded.at(-1);
    if (p && p[1] === text && t - p[2] < 3000) {
      p[2] = t;
      p[3]++;
    } else folded.push([t, text, t, 1]);
  }
  return folded.map(([t, s, , n]) => [t, n > 1 ? `${s} ×${n}` : s]);
}
function frameStarts(bundle: Data, words: Data[][] | undefined): number[] {
  const starts: number[] = [];
  for (let i = 0; i < bundle.keyframes.length; i++) {
    const f = bundle.keyframes[i],
      frameMarks = marksOn(f, bundle.marks),
      first = words?.[i]?.[0]?.t0,
      mark = frameMarks[0]?.t0;
    const click =
      f.reason === "click"
        ? bundle.events
            .filter((e: Data) => ["click", "drag"].includes(e.kind))
            .map((e: Data) => e.t)
            .filter((t: number) => t <= f.t + 200 && f.t - t <= 1500)
            .sort((a: number, b: number) => b - a)[0]
        : undefined;
    const all = [f.t, first, mark, click].filter(
      (n: any) => n !== undefined,
    ) as number[];
    starts.push(Math.max(i ? starts[i - 1] : 0, Math.min(...all)));
  }
  return starts;
}
export function keyframeTexts(bundle: Data): FrameText[] {
  const frames = bundle.keyframes;
  if (!frames.length) return [];
  const words = wordsPerFrame(bundle),
    starts = frameStarts(bundle, words),
    allEvents = eventLines(bundle),
    items: Item[][] = frames.map(() => []);
  const owner = (t: number) => {
    let found = 0;
    for (let i = 0; i < starts.length; i++) if (starts[i] <= t) found = i;
    return found;
  };
  frames.forEach((f: Data, i: number) => {
    const ms = marksOn(f, bundle.marks),
      numbered = ms.length >= 2;
    ms.forEach((m: Data, j: number) =>
      items[i].push({ t: m.t0, mark: m, number: numbered ? j + 1 : undefined }),
    );
  });
  for (const e of bundle.events.filter((e: Data) => e.kind === "note")) {
    const frame = bundle.marks.find((m: Data) => m.pin === e.pin)?.frame;
    const body = String(e.text ?? "").split(/\r?\n/u);
    if (body.at(-1) === "") body.pop();
    items[owner(e.t)].push({
      t: e.t,
      text: `Typed note · pin ${e.pin ?? 1}${frame ? ` · ${frame}` : ""}:\n${body.map((s) => `    ${s}`).join("\n")}`,
      note: true,
    });
  }
  for (const [t, text] of allEvents) items[owner(t)].push({ t, text });
  return frames.map((f: Data, i: number) => {
    const sorted = items[i].sort((a, b) => a.t - b.t),
      frameWords = words?.[i],
      splits = sorted.map((it) =>
        it.note
          ? it.t
          : frameWords
            ? splitAt(it.t, frameWords, clauses(frameWords))
            : it.t,
      );
    for (let j = 1; j < splits.length; j++)
      splits[j] = Math.max(splits[j], splits[j - 1]);
    const said = (from: number | undefined, to: number | undefined) =>
      q(
        joinWords(
          (frameWords ?? []).filter(
            (w) =>
              (from === undefined || w.t0 >= from) &&
              (to === undefined || w.t0 < to),
          ),
        ),
      );
    const head = frameWords ? said(undefined, splits[0]) : q(f.said ?? "");
    const lines: string[] = [];
    let open: string | undefined;
    for (let j = 0; j < sorted.length; j++) {
      const item = sorted[j],
        isMark = !!item.mark,
        label = item.mark
          ? describeMark(item.mark, item.number)
          : `· ${clock(item.t)} ${item.text}`;
      if (item.note) {
        if (open) lines.push(open);
        open = undefined;
        lines.push(label);
        const speech = frameWords ? said(splits[j], splits[j + 1]) : "";
        if (speech) lines.push(speech);
        continue;
      }
      const line = open && !isMark ? `${open} ${label}` : label;
      if (open && isMark) lines.push(open);
      open = undefined;
      const speech = frameWords ? said(splits[j], splits[j + 1]) : "";
      if (!speech && !isMark) open = line;
      else lines.push(speech ? `${line} ${speech}` : line);
    }
    if (open) lines.push(open);
    let changed = "";
    if (f.reason === "scene_change" && bundle.autocrop?.changes?.[i]) {
      const boxes = [...bundle.autocrop.changes[i]].map((b: number[]) => [
        ...b,
      ]);
      let joined = true;
      while (joined) {
        joined = false;
        outer: for (let a = 0; a < boxes.length; a++)
          for (let b = a + 1; b < boxes.length; b++) {
            const x = boxes[a],
              y = boxes[b];
            if (
              x[0] - 0.02 <= y[2] &&
              y[0] - 0.02 <= x[2] &&
              x[1] - 0.02 <= y[3] &&
              y[1] - 0.02 <= x[3]
            ) {
              boxes[a] = [
                Math.min(x[0], y[0]),
                Math.min(x[1], y[1]),
                Math.max(x[2], y[2]),
                Math.max(x[3], y[3]),
              ];
              boxes.splice(b, 1);
              joined = true;
              break outer;
            }
          }
      }
      if (
        boxes.some(
          (b: number[]) => Math.abs((b[2] - b[0]) * (b[3] - b[1])) >= 0.9,
        )
      )
        changed = "the whole screen";
    }
    let finalHead = head;
    if (changed)
      finalHead = `[changed: ${changed}]${finalHead ? ` ${finalHead}` : ""}`;
    if (!finalHead && !lines.length)
      finalHead = String(f.event ?? "").replace(/ \(x [^)]*\)/gu, "");
    return { head: finalHead, lines };
  });
}
