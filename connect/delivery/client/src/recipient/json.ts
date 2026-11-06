/** Strict, bounded JSON decoding for untrusted recipient payloads. */

const ABSOLUTE_MAXIMUM_BYTES = 2_000_000;
const MAXIMUM_DEPTH = 64;
const MAXIMUM_NODES = 100_000;

export type StrictJsonFailure =
  | "invalid_json"
  | "invalid_utf8"
  | "byte_limit_exceeded"
  | "complexity_limit_exceeded";

/** Errors intentionally contain only an allowlisted category, never input data. */
export class StrictJsonError extends Error {
  readonly category: StrictJsonFailure;
  constructor(category: StrictJsonFailure) {
    super(category);
    this.name = "StrictJsonError";
    this.category = category;
  }
}

function fail(category: StrictJsonFailure): never {
  throw new StrictJsonError(category);
}

/** Decode one complete JSON value, rejecting ambiguous and excessively complex input. */
export function decodeStrictJson(bytes: Uint8Array, maximumBytes = 128 * 1024): unknown {
  if (!Number.isSafeInteger(maximumBytes) || maximumBytes < 0 || maximumBytes > ABSOLUTE_MAXIMUM_BYTES) {
    fail("byte_limit_exceeded");
  }
  if (bytes.byteLength > maximumBytes) fail("byte_limit_exceeded");

  let source: string;
  try {
    source = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    fail("invalid_utf8");
  }
  if (source!.charCodeAt(0) === 0xfeff) fail("invalid_json");

  let offset = 0;
  let nodes = 0;
  const whitespace = () => {
    while (offset < source!.length && (source![offset] === " " || source![offset] === "\t" || source![offset] === "\n" || source![offset] === "\r")) offset++;
  };
  const node = () => {
    nodes++;
    if (nodes > MAXIMUM_NODES) fail("complexity_limit_exceeded");
  };
  const string = (): string => {
    const start = offset;
    if (source![offset] !== '"') fail("invalid_json");
    offset++;
    while (offset < source!.length) {
      const code = source!.charCodeAt(offset);
      if (code === 0x22) {
        offset++;
        let value: string;
        try { value = JSON.parse(source!.slice(start, offset)) as string; } catch { fail("invalid_json"); }
        for (let i = 0; i < value!.length; i++) {
          const c = value!.charCodeAt(i);
          if (c >= 0xd800 && c <= 0xdbff) {
            const next = value!.charCodeAt(i + 1);
            if (!(next >= 0xdc00 && next <= 0xdfff)) fail("invalid_json");
            i++;
          } else if (c >= 0xdc00 && c <= 0xdfff) fail("invalid_json");
        }
        return value!;
      }
      if (code < 0x20) fail("invalid_json");
      if (code === 0x5c) {
        offset++;
        if (offset >= source!.length) fail("invalid_json");
        const escape = source![offset]!;
        if (escape === "u") {
          if (!/^[0-9a-fA-F]{4}$/.test(source!.slice(offset + 1, offset + 5))) fail("invalid_json");
          offset += 5;
          continue;
        }
        if (!'"\\/bfnrt'.includes(escape)) fail("invalid_json");
      }
      offset++;
    }
    fail("invalid_json");
  };

  const value = (depth: number): unknown => {
    if (depth > MAXIMUM_DEPTH) fail("complexity_limit_exceeded");
    whitespace();
    node();
    const c = source![offset];
    if (c === '"') return string();
    if (c === "{") {
      offset++;
      whitespace();
      const result: Record<string, unknown> = {};
      const keys = new Set<string>();
      if (source![offset] === "}") { offset++; return result; }
      while (true) {
        whitespace();
        if (source![offset] !== '"') fail("invalid_json");
        const key = string();
        if (keys.has(key)) fail("invalid_json");
        keys.add(key);
        whitespace();
        if (source![offset] !== ":") fail("invalid_json");
        offset++;
        Object.defineProperty(result, key, { value: value(depth + 1), enumerable: true, configurable: true, writable: true });
        whitespace();
        if (source![offset] === "}") { offset++; return result; }
        if (source![offset] !== ",") fail("invalid_json");
        offset++;
      }
    }
    if (c === "[") {
      offset++;
      whitespace();
      const result: unknown[] = [];
      if (source![offset] === "]") { offset++; return result; }
      while (true) {
        result.push(value(depth + 1));
        whitespace();
        if (source![offset] === "]") { offset++; return result; }
        if (source![offset] !== ",") fail("invalid_json");
        offset++;
      }
    }
    if (c === "t" && source!.slice(offset, offset + 4) === "true") { offset += 4; return true; }
    if (c === "f" && source!.slice(offset, offset + 5) === "false") { offset += 5; return false; }
    if (c === "n" && source!.slice(offset, offset + 4) === "null") { offset += 4; return null; }
    const match = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(source!.slice(offset));
    if (!match) fail("invalid_json");
    offset += match[0].length;
    const number = Number(match[0]);
    if (!Number.isFinite(number)) fail("invalid_json");
    return number;
  };

  const result = value(0);
  whitespace();
  if (offset !== source!.length) fail("invalid_json");
  return result;
}
