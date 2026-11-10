export type LocationSnapshot = {
  savedPath: string | null; effectivePath: string; displayPath: string; custom: boolean;
  source: 'environment' | 'configured' | 'builtin' | 'fallback';
};
export type NativeLocationFacts = {
  saved: { kind: 'absent' } | { kind: 'string'; value: string } | { kind: 'null' } | { kind: 'other' };
  environment: { present: boolean; value: string | null };
  homePath: string; builtinPath: string; revision: string;
};
