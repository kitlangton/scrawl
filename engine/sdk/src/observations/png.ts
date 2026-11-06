import { crc32 } from "node:zlib";

const SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const MAX_CHUNKS = 16_384;
const MAX_PROFILE_BYTES = 74_772;
function bad(message: string): never { throw new Error(`frame observations: ${message}`); }

/** Bounded structural/CRC inspection only: no decompression or pixel decoding.
 * A native completion authenticates that the measured plane is the encoded plane. */
export function readProfile(png: Buffer, width: number, height: number) {
  if (!png.subarray(0, 8).equals(SIGNATURE)) bad("invalid PNG signature");
  let at = 8, chunks = 0, imageBytes = 0;
  let header = false, ended = false, dataStarted = false, dataEnded = false;
  let rgba8 = false, palette = false, colourType = -1;
  let profile: Buffer | undefined;
  while (at < png.length) {
    if (++chunks > MAX_CHUNKS || png.length - at < 12) bad("PNG chunk bounds exceeded");
    const length = png.readUInt32BE(at);
    if (length > 0x7fffffff || length > png.length - at - 12) bad("truncated or oversized PNG chunk");
    const typeBytes = png.subarray(at + 4, at + 8);
    const type = typeBytes.toString("ascii");
    if (![...typeBytes].every(c => c >= 65 && c <= 90 || c >= 97 && c <= 122) || (typeBytes[2] & 32) !== 0) bad("invalid PNG chunk type");
    const body = png.subarray(at + 8, at + 8 + length);
    if (crc32(body, crc32(typeBytes)) !== png.readUInt32BE(at + 8 + length)) bad("PNG CRC mismatch");
    if (!header && type !== "IHDR") bad("PNG IHDR must be first");
    if (type === "IHDR") {
      if (header || length !== 13) bad("duplicate or malformed PNG IHDR");
      if (body.readUInt32BE(0) !== width || body.readUInt32BE(4) !== height) bad("PNG IHDR differs from derivative dimensions");
      const depth = body[8], colour = body[9];
      colourType = colour;
      const allowed = colour === 0 ? [1, 2, 4, 8, 16] : colour === 2 || colour === 4 || colour === 6 ? [8, 16] : colour === 3 ? [1, 2, 4, 8] : [];
      if (!allowed.includes(depth) || body[10] !== 0 || body[11] !== 0 || body[12] > 1) bad("invalid PNG IHDR encoding");
      rgba8 = depth === 8 && colour === 6 && body[12] === 0;
      header = true;
    } else if (type === "IDAT") {
      if (dataEnded) bad("nonconsecutive PNG IDAT chunks");
      if (colourType === 3 && !palette) bad("indexed PNG is missing palette");
      dataStarted = true; imageBytes += length;
    } else {
      if (dataStarted) dataEnded = true;
      if (type === "apOB") {
        if (!dataStarted || imageBytes === 0) bad("observation profile must follow after image data");
        if (profile !== undefined) bad("duplicate observation profile");
        if (length > MAX_PROFILE_BYTES) bad("observation profile exceeds bounds");
        profile = body;
      } else if (type === "IEND") {
        if (length !== 0 || !dataStarted || imageBytes === 0) bad("invalid PNG end/image data");
        ended = true;
      } else if (type === "PLTE") {
        if (palette || colourType === 0 || colourType === 4 || dataStarted || length === 0 || length > 768 || length % 3 !== 0) bad("invalid PNG palette");
        palette = true;
      } else if ((typeBytes[0] & 32) === 0) bad("unsupported critical PNG chunk");
    }
    at += length + 12;
    if (ended) break;
  }
  if (!ended || at !== png.length) bad("missing PNG IEND or trailing bytes");
  if (profile === undefined) return { state: "unavailable" as const, reason: "observation-profile-absent" as const };
  const magic = profile.subarray(0, 8).toString("latin1");
  if (!/^APOBS[0-9]{3}$/.test(magic)) bad("malformed observation profile identifier");
  if (magic !== "APOBS001") return { state: "unavailable" as const, reason: "observation-profile-unsupported" as const };
  if (!rgba8 || profile.length < 20) bad("profile requires noninterlaced RGBA8 PNG");
  if (profile.readUInt32LE(8) !== width || profile.readUInt32LE(12) !== height) bad("profile dimensions differ from PNG/derivative");
  const columns = profile.readUInt16LE(16), rows = profile.readUInt16LE(18);
  if (columns !== Math.min(width, 128) || rows !== Math.min(height, 128)) bad("invalid observation region dimensions");
  const count = 64 * 36 + columns * rows;
  if (profile.length !== 20 + count * 4) bad("invalid observation payload length");
  const cells: number[] = [];
  for (let i = 0; i < count; i++) {
    const value = profile.readFloatLE(20 + i * 4);
    if (!Number.isFinite(value) || value < 0 || value > 1) bad("observation cell outside finite unit range");
    cells.push(value);
  }
  return { state: "measured" as const, columns, rows, thumbnail: cells.slice(0, 64 * 36), regions: cells.slice(64 * 36) };
}
