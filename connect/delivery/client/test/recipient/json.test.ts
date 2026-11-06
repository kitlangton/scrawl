import assert from "node:assert/strict";
import test from "node:test";
import { decodeStrictJson, StrictJsonError } from "../../src/recipient/json.ts";

const encode = (text: string) => new TextEncoder().encode(text);
function rejects(text: string, category?: string, maximumBytes?: number): void {
  assert.throws(() => decodeStrictJson(encode(text), maximumBytes), (error: unknown) => {
    assert.ok(error instanceof StrictJsonError);
    if (category) assert.equal(error.category, category);
    assert.equal(error.message.includes(text), false);
    return true;
  });
}

test("decodes strict JSON values and preserves ordinary semantics", () => {
  const result = decodeStrictJson(encode('{"a":[1,true,null,"é"],"n":-1.25e2}')) as Record<string, unknown>;
  assert.equal(Object.getPrototypeOf(result), Object.prototype);
  assert.deepEqual(result.a, [1, true, null, "é"]);
  assert.equal(result.n, -125);
  assert.equal(decodeStrictJson(encode('"😀"')), "😀");
});

test("rejects malformed encodings, BOM, and non-single JSON documents", () => {
  for (const text of ["\uFEFF{}", "{} {}", "{} //x", "[1,]", '{"x":1,}', "01", "+1", ".1", "1.", "1e", "NaN", "Infinity", '"\\uD800"', '"\\uDC00"']) rejects(text, "invalid_json");
  assert.throws(() => decodeStrictJson(new Uint8Array([0x22, 0xc3, 0x28, 0x22])), (e: unknown) => e instanceof StrictJsonError && e.category === "invalid_utf8");
});

test("rejects duplicate decoded keys, including escaped aliases at every depth", () => {
  rejects('{"x":1,"x":2}');
  rejects('{"x":1,"\\u0078":2}');
  rejects('{"nested":[{"\\u0061":1,"a":2}]}');
  assert.equal((decodeStrictJson(encode('{"__proto__":{"polluted":true}}')) as Record<string, unknown>)["__proto__"] !== undefined, true);
  assert.equal(({} as Record<string, unknown>).polluted, undefined);
});

test("checks byte limits before decoding and accepts exact multibyte boundary", () => {
  const bytes = encode('"é"');
  assert.equal(decodeStrictJson(bytes, bytes.byteLength), "é");
  assert.throws(() => decodeStrictJson(bytes, bytes.byteLength - 1), (e: unknown) => e instanceof StrictJsonError && e.category === "byte_limit_exceeded");
  assert.throws(() => decodeStrictJson(new Uint8Array(3), 2), (e: unknown) => e instanceof StrictJsonError && e.category === "byte_limit_exceeded");
  assert.throws(() => decodeStrictJson(bytes, 2_000_001), (e: unknown) => e instanceof StrictJsonError && e.category === "byte_limit_exceeded");
});

test("bounds nesting and value count", () => {
  rejects(`${"[".repeat(66)}0${"]".repeat(66)}`, "complexity_limit_exceeded");
  const many = `[${Array.from({ length: 100_001 }, () => "0").join(",")}]`;
  assert.throws(() => decodeStrictJson(encode(many), 1_000_000), (e: unknown) => e instanceof StrictJsonError && e.category === "complexity_limit_exceeded");
});
