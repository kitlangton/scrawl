import {
  absolutePath,
  exact,
  fail,
  id,
  optionalExact,
  parseDetached,
  record,
  safeInt,
  string,
} from "./common.ts";
import type {
  CandidateFacts,
  ContextFacts,
  InteractionFacts,
  MaterialsDocument,
  SealedFile,
  TranscriptFacts,
} from "./types.ts";
import { withinClock } from "./materials.ts";
import { validateStrokeFacts } from "./materials.ts";
import { isDeepStrictEqual } from "node:util";

function identity(v: unknown, materials: MaterialsDocument, label: string) {
  const x = record(v, label);
  exact(x, ["captureId", "materialsId"], label);
  if (
    x.captureId !== materials.payload.identity.captureId ||
    x.materialsId !== materials.payload.identity.materialsId
  )
    fail(`${label} is not bound to materials`);
}
function observed(
  v: unknown,
  label: string,
  validate: (value: unknown) => void,
) {
  const x = record(v, label);
  if (x.state === "unavailable") {
    exact(x, ["state", "reason"], label);
    string(x.reason, `${label}.reason`);
  } else if (x.state === "collected") {
    exact(x, ["state", "value"], label);
    validate(x.value);
  } else fail(`${label} must be explicitly collected or unavailable`);
}
function clock(t: unknown, m: MaterialsDocument, label: string) {
  withinClock(t, m.payload.clock.durationMs, label);
}
export function validateConfirmedSource(v: unknown, m: MaterialsDocument) {
  const x = record(v, "confirmedSource");
  exact(
    x,
    [
      "id",
      "generation",
      "confirmedAtMs",
      "kind",
      "displayId",
      "rectangle",
      "confirmation",
    ],
    "confirmedSource",
  );
  string(x.id, "source id");
  id(x.generation, "source generation");
  clock(x.confirmedAtMs, m, "source confirmation time");
  if (
    (x.kind !== "display" && x.kind !== "area") ||
    !Number.isSafeInteger(x.displayId) ||
    (x.displayId as number) < 0
  )
    fail("invalid display/area source facts");
  const r = record(x.rectangle, "source rectangle");
  exact(r, ["x", "y", "width", "height", "scale"], "source rectangle");
  for (const k of ["x", "y", "width", "height"])
    if (!Number.isFinite(r[k])) fail(`invalid rectangle ${k}`);
  if (
    (r.width as number) <= 0 ||
    (r.height as number) <= 0 ||
    typeof r.scale !== "number" ||
    !Number.isFinite(r.scale) ||
    r.scale <= 0
  )
    fail("invalid source geometry");
  const c = record(x.confirmation, "source confirmation");
  exact(c, ["kind", "id"], "source confirmation");
  if (!["portal", "native-picker", "explicit-display"].includes(String(c.kind)))
    fail("invalid source confirmation kind");
  id(c.id, "source confirmation id");
}
export function validateInvocation(v: unknown, m: MaterialsDocument) {
  const x = record(v, "invocation");
  if (x.kind === "explicit")
    exact(x, ["kind", "invocationId", "observedAtMs"], "invocation");
  else if (x.kind === "picker")
    exact(
      x,
      ["kind", "invocationId", "observedAtMs", "confirmationId"],
      "invocation",
    );
  else if (x.kind === "shortcut")
    optionalExact(
      x,
      [
        "kind",
        "invocationId",
        "observedAtMs",
        "hostEventId",
        "consumedReservationId",
      ],
      ["kind", "invocationId", "observedAtMs", "hostEventId"],
      "invocation",
    );
  else fail("invalid invocation kind");
  id(x.invocationId, "invocationId");
  clock(x.observedAtMs, m, "invocation time");
  for (const k of ["confirmationId", "hostEventId", "consumedReservationId"])
    if (x[k] !== undefined) id(x[k], `invocation.${k}`);
}
export function validateContextFacts(
  value: unknown,
  m: MaterialsDocument,
): asserts value is ContextFacts {
  const x = record(value, "context facts");
  exact(
    x,
    [
      "schema",
      "identity",
      "source",
      "invocation",
      "context",
      "session",
      "windows",
      "cursor",
    ],
    "context facts",
  );
  if (x.schema !== "apparent.capture-context/1")
    fail("unsupported context facts schema");
  identity(x.identity, m, "context identity");
  validateConfirmedSource(x.source, m);
  validateInvocation(x.invocation, m);
  if (
    !isDeepStrictEqual(x.source, m.payload.confirmedSource) ||
    !isDeepStrictEqual(x.invocation, m.payload.invocation)
  )
    fail("context source/invocation do not match sealed materials facts");
  observed(x.context, "context observation", (v) => {
    const c = record(v, "context values");
    exact(c, ["values", "changes"], "context values");
    const values = record(c.values, "context values map");
    for (const [k, val] of Object.entries(values))
      if (typeof val !== "string") fail("context values must be text");
    if (!Array.isArray(c.changes)) fail("context changes must be an array");
    c.changes.forEach((item, i) => {
      const a = record(item, `context change ${i}`);
      exact(a, ["timeMs", "key", "value"], `context change ${i}`);
      clock(a.timeMs, m, "context change time");
      string(a.key, "context change key");
      if (a.value !== null && typeof a.value !== "string")
        fail("context change value must be text or null");
    });
  });
  observed(x.session, "session observation", (v) => {
    const s = record(v, "session");
    optionalExact(s, ["id", "directory", "url"], ["id"], "session");
    string(s.id, "session id");
    if (s.directory !== undefined)
      absolutePath(s.directory, "session directory");
    if (s.url !== undefined) string(s.url, "session URL");
  });
  observed(x.windows, "window observation", (v) => {
    if (!Array.isArray(v)) fail("windows must be an array");
    v.forEach((batch, i) => {
      const b = record(batch, `window sample ${i}`);
      exact(b, ["timeMs", "entries"], "window sample");
      clock(b.timeMs, m, "window sample time");
      if (!Array.isArray(b.entries)) fail("window entries must be an array");
      b.entries.forEach((entry) => {
        const w = record(entry, "window fact");
        optionalExact(
          w,
          [
            "id",
            "app",
            "bundleId",
            "title",
            "regular",
            "bounds",
            "layer",
            "frontmost",
          ],
          ["id", "app", "bounds", "layer"],
          "window fact",
        );
        safeInt(w.id, "window id");
        string(w.app, "window app");
        if (w.bundleId !== undefined) string(w.bundleId, "window bundle id");
        if (w.title !== undefined) string(w.title, "window title");
        if (w.regular !== undefined && typeof w.regular !== "boolean")
          fail("window regular must be boolean");
        if (w.frontmost !== undefined && typeof w.frontmost !== "boolean")
          fail("window frontmost must be boolean");
        if (
          !Array.isArray(w.bounds) ||
          w.bounds.length !== 4 ||
          w.bounds.some((n) => typeof n !== "number" || !Number.isFinite(n))
        )
          fail("window bounds must have four finite coordinates");
        if (!Number.isSafeInteger(w.layer))
          fail("window layer must be an integer");
      });
    });
  });
  observed(x.cursor, "cursor observation", (v) => {
    if (!Array.isArray(v)) fail("cursor samples must be an array");
    v.forEach((sample, i) => {
      if (
        !Array.isArray(sample) ||
        sample.length !== 3 ||
        sample.some((n) => typeof n !== "number" || !Number.isFinite(n))
      )
        fail(`cursor sample ${i} is invalid`);
      clock(sample[0], m, "cursor time");
      if (sample[1] < 0 || sample[1] > 1 || sample[2] < 0 || sample[2] > 1)
        fail("cursor coordinates must be normalized");
    });
  });
}
const identitySchema = (
  v: unknown,
  m: MaterialsDocument,
  label: string,
  schema: string,
) => {
  const x = record(v, label);
  if (x.schema !== schema) fail(`unsupported ${label} schema`);
  identity(x.identity, m, `${label} identity`);
};
export function validateCandidateFacts(
  v: unknown,
  m: MaterialsDocument,
): asserts v is CandidateFacts {
  const x = record(v, "candidate facts");
  exact(x, ["schema", "identity", "candidates"], "candidate facts");
  identitySchema(x, m, "candidate facts", "apparent.capture-candidates/1");
  if (!Array.isArray(x.candidates) || x.candidates.length > 10000)
    fail("candidate inventory exceeds bounds");
  const files = new Set(m.payload.files.map((f) => f.relativePath));
  x.candidates.forEach((item, i) => {
    const c = record(item, `candidate ${i}`);
    optionalExact(
      c,
      ["candidateId", "timeMs", "file", "width", "height", "visualChange"],
      ["candidateId", "timeMs", "file", "width", "height"],
      "candidate",
    );
    id(c.candidateId, "candidateId");
    clock(c.timeMs, m, "candidate time");
    if (typeof c.file !== "string" || !files.has(c.file))
      fail("candidate must reference a sealed material");
    safeInt(c.width, "candidate width", 1);
    safeInt(c.height, "candidate height", 1);
    if (
      c.visualChange !== undefined &&
      (typeof c.visualChange !== "number" || !Number.isFinite(c.visualChange))
    )
      fail("invalid visual change fact");
  });
}
export function validateTranscriptFacts(
  v: unknown,
  m: MaterialsDocument,
): asserts v is TranscriptFacts {
  const x = record(v, "transcript facts");
  exact(
    x,
    ["schema", "identity", "provider", "model", "text", "segments"],
    "transcript facts",
  );
  identitySchema(x, m, "transcript facts", "apparent.capture-transcript/1");
  string(x.provider, "transcript provider");
  string(x.model, "transcript model");
  if (
    typeof x.text !== "string" ||
    !Array.isArray(x.segments) ||
    x.segments.length > 100000
  )
    fail("invalid transcript facts");
  let prev = -1;
  x.segments.forEach((item, i) => {
    const s = record(item, `transcript segment ${i}`);
    optionalExact(
      s,
      ["startMs", "endMs", "text"],
      ["startMs", "text"],
      "transcript segment",
    );
    clock(s.startMs, m, "transcript start");
    if ((s.startMs as number) < prev)
      fail("transcript segments are not ordered");
    prev = s.startMs as number;
    if (s.endMs !== undefined) {
      clock(s.endMs, m, "transcript end");
      if ((s.endMs as number) < (s.startMs as number))
        fail("transcript segment ends before start");
    }
    if (typeof s.text !== "string") fail("transcript segment text is invalid");
  });
}
export function validateInteractionFacts(
  v: unknown,
  m: MaterialsDocument,
): asserts v is InteractionFacts {
  const x = record(v, "interaction facts");
  exact(x, ["schema", "identity", "events"], "interaction facts");
  identitySchema(x, m, "interaction facts", "apparent.capture-interactions/1");
  if (!Array.isArray(x.events) || x.events.length > 100000)
    fail("interaction inventory exceeds bounds");
  x.events.forEach((event, i) => {
    const e = record(event, `interaction ${i}`);
    const kinds: Record<string, string[]> = {
      note: ["kind", "t", "pin", "x", "y", "text"],
      click: ["kind", "t", "x", "y", "button", "label", "role"],
      drag: [
        "kind",
        "t",
        "t1",
        "x0",
        "y0",
        "x1",
        "y1",
        "label",
        "button",
        "path",
      ],
      scroll: ["kind", "t", "x", "y", "dx", "dy", "label"],
      point: ["kind", "t", "x", "y", "label"],
      key: ["kind", "t", "shortcut"],
      app_change: ["kind", "t", "app", "window_title"],
      app_event: ["kind", "t", "label"],
    };
    const allowed = kinds[String(e.kind)];
    if (!allowed) fail("unknown interaction kind");
    const required =
      e.kind === "note"
        ? allowed
        : e.kind === "click"
          ? ["kind", "t", "x", "y"]
          : e.kind === "drag"
            ? ["kind", "t", "t1", "x0", "y0", "x1", "y1"]
            : e.kind === "scroll"
              ? ["kind", "t", "x", "y", "dx", "dy"]
              : e.kind === "point"
                ? ["kind", "t", "x", "y"]
                : e.kind === "key"
                  ? ["kind", "t", "shortcut"]
                  : e.kind === "app_change"
                    ? ["kind", "t", "app"]
                    : ["kind", "t", "label"];
    optionalExact(e, allowed, required, `interaction ${i}`);
    clock(e.t, m, "interaction time");
    if (e.t1 !== undefined) {
      clock(e.t1, m, "drag end");
      if ((e.t1 as number) < (e.t as number))
        fail("drag ends before it starts");
    }
    if (e.pin !== undefined) safeInt(e.pin, "interaction pin");
    if (e.kind === "scroll" && [e.dx, e.dy].some(value => typeof value !== "number" || !Number.isFinite(value))) fail("scroll deltas must be finite numbers");
    for (const k of ["x", "y", "x0", "y0", "x1", "y1"])
      if (
        e[k] !== undefined &&
        (typeof e[k] !== "number" ||
          !Number.isFinite(e[k]) ||
          (e[k] as number) < 0 ||
          (e[k] as number) > 1)
      )
        fail(`invalid normalized interaction ${k}`);
    for (const k of [
      "text",
      "button",
      "label",
      "role",
      "shortcut",
      "app",
      "window_title",
    ])
      if (e[k] !== undefined && typeof e[k] !== "string")
        fail(`invalid interaction ${k}`);
    if (
      e.path !== undefined &&
      (!Array.isArray(e.path) ||
        e.path.some(
          (p) =>
            !Array.isArray(p) ||
            p.length !== 2 ||
            p.some(
              (n) =>
                typeof n !== "number" || !Number.isFinite(n) || n < 0 || n > 1,
            ),
        ))
    )
      fail("invalid normalized drag path");
  });
}

export function validateFactFile(
  file: SealedFile,
  bytes: Uint8Array,
  materials: MaterialsDocument,
  kind:
    | "context"
    | "interactions"
    | "candidates"
    | "transcript"
    | "acceptedStrokes",
): unknown {
  const reference = materials.payload.facts[kind];
  if (
    materials.payload.collection[kind] !== "collected" ||
    !reference ||
    reference.file !== file.relativePath
  )
    fail("fact file is not a collected materials reference");
  const sealed = materials.payload.files.find(
    (f) => f.relativePath === file.relativePath,
  );
  if (
    !sealed ||
    sealed.sha256 !== file.sha256 ||
    !isDeepStrictEqual(sealed.identity, file.identity) ||
    file.identity.size !== bytes.byteLength
  )
    fail("fact file differs from materials sealed reference");
  const parsed = parseDetached<unknown>(bytes, file.sha256, "capture fact");
  if (kind === "context") validateContextFacts(parsed, materials);
  else if (kind === "interactions") validateInteractionFacts(parsed, materials);
  else if (kind === "candidates") validateCandidateFacts(parsed, materials);
  else if (kind === "transcript") validateTranscriptFacts(parsed, materials);
  else validateStrokeFacts(file, bytes, materials);
  return parsed;
}
