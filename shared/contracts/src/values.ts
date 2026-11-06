export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
export interface LegacyConfigValue { present: boolean; value: JsonValue }
