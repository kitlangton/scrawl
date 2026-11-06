import type { PenColor } from "../../../capabilities/index.js";
import { PEN_COLOR_RGB } from "../../../capabilities/helpers.js";
import type { RecordPenConfigPort } from "../../capabilities/provider.js";

type LegacyConfigValue = { present: boolean; value: unknown };
type ConfigResult = { status: "saved" } | { status: "conflict"; current: LegacyConfigValue } | { status: "unknown"; message: string };
export interface RecordPenColorConfigInput {
  read(): Promise<LegacyConfigValue>;
  compareAndSwap(expected: LegacyConfigValue, next: LegacyConfigValue): Promise<ConfigResult>;
}
export interface RecordNativePenInput {
  readStrokeColor(): Promise<{ api: 1; processEpoch: string; scope: "running-desktop-process"; rgb: [number, number, number] | null }>;
  applyStrokeColor(rgb: [number, number, number] | null, expectedPenColor: LegacyConfigValue,
    expected: { processEpoch: string; revision: number }): Promise<
      | { status: "applied"; rgb: [number, number, number] | null }
      | { status: "conflict"; current: LegacyConfigValue }
      | { status: "unavailable" | "outcome-unknown" }
    >;
}
export interface RecordNativeHostInput {
  readSnapshot(): Promise<{ api: 1; processEpoch: string; revision: number; scope: "running-desktop-process" }>;
}

const colors = Object.keys(PEN_COLOR_RGB) as PenColor[];
const colorForRgb = (rgb: readonly number[] | null): PenColor | null => {
  if (!rgb || rgb.length !== 3) return null;
  return colors.find(color => PEN_COLOR_RGB[color].every((channel, index) => channel === rgb[index])) ?? null;
};
const validLegacy = (value: unknown): value is LegacyConfigValue => !!value && typeof value === "object"
  && !Array.isArray(value) && Object.keys(value).length === 2 && Object.hasOwn(value, "present")
  && Object.hasOwn(value, "value") && typeof (value as LegacyConfigValue).present === "boolean"
  && ((value as LegacyConfigValue).present || (value as LegacyConfigValue).value === null);
const sameLegacy = (a: LegacyConfigValue, b: LegacyConfigValue) => a.present === b.present && Object.is(a.value, b.value);
function persistedColor(value: LegacyConfigValue): PenColor | null {
  if (!validLegacy(value)) throw new Error("Invalid pen config snapshot");
  if (!value.present || value.value === null) return null;
  if (typeof value.value === "string" && colors.includes(value.value as PenColor)) return value.value as PenColor;
  throw new Error("Unsupported stored pen color");
}
function expectedColor(color: PenColor): LegacyConfigValue { return { present: true, value: color }; }
function validState(value: { api: number; processEpoch: string; revision: number; scope: string }) {
  return value.api === 1 && value.scope === "running-desktop-process"
    && typeof value.processEpoch === "string" && /^[a-zA-Z0-9-]{1,64}$/.test(value.processEpoch)
    && Number.isSafeInteger(value.revision) && value.revision >= 0;
}

/** Consumer-neutral Record adapter over the single config writer and guarded native pen port. */
export function createRecordPenConfigPort(input: {
  config: RecordPenColorConfigInput;
  nativePen: RecordNativePenInput;
  nativeHost: RecordNativeHostInput;
}): RecordPenConfigPort {
  const { config, nativePen, nativeHost } = input;
  let queue = Promise.resolve();
  const serialize = <T>(operation: () => Promise<T>): Promise<T> => {
    const work = queue.then(operation);
    queue = work.then(() => undefined, () => undefined);
    return work;
  };
  const readCurrent = async () => {
    const [config, admission, actual] = await Promise.all([
      input.config.read(), nativeHost.readSnapshot(), nativePen.readStrokeColor(),
    ]);
    const persisted = persistedColor(config);
    if (!validState(admission) || actual.api !== 1 || actual.scope !== "running-desktop-process"
      || typeof actual.processEpoch !== "string" || actual.processEpoch !== admission.processEpoch) {
      throw new Error("Pen observations do not belong to the same Desktop process");
    }
    return { persisted, effective: colorForRgb(actual.rgb) };
  };
  return {
    read: () => serialize(readCurrent),
    writeApplyAndReadBack(color) {
      const requested = color;
      return serialize(async () => {
        if (!colors.includes(requested)) return { status: "unknown" };
        let before: LegacyConfigValue;
        try {
          before = await config.read();
          persistedColor(before); // Unsupported existing values are preserved and never overwritten.
        } catch { return { status: "unknown" }; }
        let saved: ConfigResult;
        try { saved = await config.compareAndSwap(before, expectedColor(requested)); }
        catch { return { status: "unknown" }; }
        if (saved.status !== "saved") return { status: "unknown" };

        try {
          const admission = await nativeHost.readSnapshot();
          if (!validState(admission)) return { status: "unknown" };
          const result = await nativePen.applyStrokeColor([...PEN_COLOR_RGB[requested]], expectedColor(requested), {
            processEpoch: admission.processEpoch, revision: admission.revision,
          });
          if (result.status !== "applied" || colorForRgb(result.rgb) !== requested) return { status: "unknown" };
          const [current, actual] = await Promise.all([config.read(), nativePen.readStrokeColor()]);
          if (!sameLegacy(current, expectedColor(requested)) || actual.api !== 1
            || actual.scope !== "running-desktop-process" || actual.processEpoch !== admission.processEpoch
            || colorForRgb(actual.rgb) !== requested) return { status: "unknown" };
          return { status: "saved", effective: requested };
        } catch { return { status: "unknown" }; }
      });
    },
  };
}
