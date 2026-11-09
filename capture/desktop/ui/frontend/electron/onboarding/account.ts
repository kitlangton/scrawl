export type AccountState = {
  status: 'loading' | 'signed-out' | 'starting' | 'approval' | 'signed-in' | 'failed';
  error: string | null;
  userCode: string | null;
  verificationUri: string | null;
};
const errors = new Set(['cancelled','offline','auth-required','rejected','expired','local-state','invalid-server','unavailable','browser']);
export function projectAccount(value: unknown): AccountState | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const v = value as Record<string, unknown>;
  if (typeof v.status !== 'string' || !['loading','signed-out','starting','approval','signed-in','failed'].includes(v.status)) return null;
  let userCode = null, verificationUri = null;
  if (v.status === 'approval') {
    if (typeof v.userCode !== 'string' || !/^[A-Za-z0-9-]{1,128}$/.test(v.userCode) || typeof v.verificationUri !== 'string' || v.verificationUri.length > 4096 || /[\s\\]/.test(v.verificationUri)) return null;
    try {
      const url = new URL(v.verificationUri);
      if (!(url.protocol === 'https:' || url.protocol === 'http:' && ['127.0.0.1','localhost','[::1]'].includes(url.hostname)) || url.username || url.password || url.hash) return null;
    } catch { return null; }
    userCode = v.userCode; verificationUri = v.verificationUri;
  }
  return {status: v.status as AccountState['status'], error: typeof v.error === 'string' && errors.has(v.error) ? v.error : null, userCode, verificationUri};
}
