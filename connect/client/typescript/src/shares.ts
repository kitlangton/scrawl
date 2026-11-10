import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';

export type Share = {
  [key: string]: unknown;
  id: string;
  captureId: string;
  url: string | null;
  status: string;
  createdAt: string;
  expiresAt: string;
  title: string;
};

export type SharesPage = { shares: Share[]; nextCursor: string | null };
export type SharesResult = { shares: Share[]; nextCursor: null };

export interface PagePort {
  (cursor: string | null): Promise<unknown>;
}

const ID = /^[a-f0-9]{32}$/;
const HASH = /^[a-f0-9]{64}$/;
const INACTIVE = new Set(['revoked', 'expired', 'deleted']);
const MAX_RESULT_BYTES = 64 * 1024 * 1024;

function invalid(message: string): never { throw new Error(message); }

function parsePage(value: unknown, origin: string): SharesPage {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return invalid('Invalid shares response');
  const page = value as Record<string, unknown>;
  if (!Array.isArray(page.shares) || page.shares.length > 100) return invalid('Invalid shares response');
  if (!(page.nextCursor === null || (typeof page.nextCursor === 'string' && page.nextCursor.length > 0 && Buffer.byteLength(page.nextCursor, 'utf8') <= 4096))) return invalid('Invalid shares cursor');
  const shares = page.shares.map((value): Share => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return invalid('Invalid share response');
    const row = value as Record<string, unknown>;
    if (typeof row.id !== 'string' || row.id.length !== 32 || !ID.test(row.id) || typeof row.captureId !== 'string' || row.captureId.length !== 32 || !ID.test(row.captureId)) return invalid('Invalid share identity');
    if (typeof row.status !== 'string' || typeof row.title !== 'string' || typeof row.createdAt !== 'string' || typeof row.expiresAt !== 'string') return invalid('Invalid share response');
    if (INACTIVE.has(row.status)) {
      if (row.url !== null) return invalid('Inactive share exposed a URL');
    } else {
      const prefix = `${origin}/s/`;
      const capability = typeof row.url === 'string' && row.url.startsWith(prefix) ? row.url.slice(prefix.length) : '';
      if (!row.url || capability.length !== 64 || !HASH.test(capability)) return invalid('Invalid share URL');
    }
    return { ...row, id: row.id, captureId: row.captureId, url: row.url as string | null, status: row.status, createdAt: row.createdAt, expiresAt: row.expiresAt, title: row.title };
  });
  return { shares, nextCursor: page.nextCursor as string | null };
}

export function encodeCursor(cursor: string): string {
  return Array.from(Buffer.from(cursor, 'utf8'), byte =>
    (byte >= 0x41 && byte <= 0x5a) || (byte >= 0x61 && byte <= 0x7a) || (byte >= 0x30 && byte <= 0x39) || '-._~'.includes(String.fromCharCode(byte))
      ? String.fromCharCode(byte) : `%${byte.toString(16).toUpperCase().padStart(2, '0')}`,
  ).join('');
}

export async function listShares(origin: string, fetchPage: PagePort): Promise<SharesResult> {
  let cursor: string | null = null;
  const seen = new Set<string>();
  const shares: Share[] = [];
  let bytes = 0;
  for (;;) {
    const page = parsePage(await fetchPage(cursor), origin);
    // Bound work before retaining the page, including empty unique-cursor pages.
    bytes += Buffer.byteLength(JSON.stringify(page), 'utf8');
    if (bytes > MAX_RESULT_BYTES) return invalid('Shares list exceeds the 64 MiB output limit');
    shares.push(...page.shares);
    cursor = page.nextCursor;
    if (cursor === null) {
      const result = { shares, nextCursor: null };
      if (Buffer.byteLength(JSON.stringify(result), 'utf8') > MAX_RESULT_BYTES) return invalid('Shares list exceeds the 64 MiB output limit');
      return result;
    }
    if (seen.has(cursor)) return invalid('Repeated shares cursor');
    seen.add(cursor);
  }
}

// Private stdio protocol: stdout is protocol-only; diagnostics contain no payloads.
// The native host performs only fixed GET /v1/shares page operations and owns credentials.
export async function runSharesStdio(origin: string): Promise<void> {
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
  const pending: string[] = [];
  let wake: (() => void) | undefined;
  let closed = false;
  input.on('line', line => { pending.push(line); wake?.(); });
  input.on('close', () => { closed = true; wake?.(); });
  const readReply = async (): Promise<unknown> => {
    while (pending.length === 0) {
      if (closed) return invalid('Shares transport closed');
      await new Promise<void>(resolve => { wake = resolve; });
      wake = undefined;
    }
    const line = pending.shift()!;
    if (Buffer.byteLength(line, 'utf8') > 2_000_032) return invalid('Invalid shares transport response');
    try { return JSON.parse(line); } catch { return invalid('Invalid shares transport response'); }
  };
  let result: SharesResult;
  try {
    result = await listShares(origin, async cursor => {
      const request = cursor === null ? { operation: 'page', before: null, query: null } : { operation: 'page', before: cursor, query: encodeCursor(cursor) };
      process.stdout.write(`${JSON.stringify(request)}\n`);
      const reply = await readReply();
      if (!reply || typeof reply !== 'object' || Array.isArray(reply) || (reply as Record<string, unknown>).ok !== true) {
        const code = reply && typeof reply === 'object' ? (reply as Record<string, unknown>).error : undefined;
        if (code === 'auth_required') return invalid('auth_required: approve this device again with apparent connect login');
        if (code === 'temporarily_unavailable') return invalid('Hosted API temporarily unavailable; retry later');
        if (code === 'transport_failed') return invalid('Hosted transport failed (network, timeout or response limit)');
        if (code === 'response_limit') return invalid('Hosted response exceeds limit');
        if (code === 'invalid_json') return invalid('Invalid hosted JSON response');
        const safeCodes = new Set(['file_too_large', 'manifest_conflict', 'beta_required', 'slow_down', 'quota_exceeded', 'unauthorized', 'forbidden', 'expired', 'rejected', 'not_found', 'conflict', 'authorization_pending', 'device_expired', 'device_rejected', 'reauthorization_required']);
        if (typeof code === 'string' && safeCodes.has(code)) return invalid(`Hosted API: ${code}`);
        return invalid(code === 'unauthorized' || code === 'forbidden' ? 'Hosted API: unauthorized' : 'Hosted API request failed');
      }
      return (reply as Record<string, unknown>).body;
    });
  } catch (error) {
    const known = new Set([
      'Invalid shares response', 'Invalid share response', 'Invalid shares cursor',
      'Invalid share identity', 'Inactive share exposed a URL', 'Invalid share URL',
      'Repeated shares cursor', 'Shares list exceeds the 64 MiB output limit',
      'Invalid shares transport response', 'Shares transport closed',
      'Hosted API temporarily unavailable; retry later',
      'Hosted transport failed (network, timeout or response limit)',
      'Hosted response exceeds limit', 'Invalid hosted JSON response',
      'Hosted API: file_too_large', 'Hosted API: manifest_conflict', 'Hosted API: beta_required',
      'Hosted API: slow_down', 'Hosted API: quota_exceeded', 'Hosted API: unauthorized',
      'Hosted API: forbidden', 'Hosted API: expired', 'Hosted API: rejected', 'Hosted API: not_found',
      'Hosted API: conflict', 'Hosted API: authorization_pending', 'Hosted API: device_expired',
      'Hosted API: device_rejected', 'Hosted API: reauthorization_required',
      'Hosted API: unauthorized', 'auth_required: approve this device again with apparent connect login',
      'Hosted API request failed',
    ]);
    const message = error instanceof Error && known.has(error.message) ? error.message : 'Hosted API request failed';
    process.stdout.write(`${JSON.stringify({ operation: 'error', error: message })}\n`);
    process.exitCode = 1;
    return;
  } finally {
    input.close();
  }
  process.stdout.write(`${JSON.stringify({ operation: 'complete', result })}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const origin = process.argv[2];
  if (!origin || !/^https?:\/\/[^/?#]+$/.test(origin)) process.exitCode = 2;
  else runSharesStdio(origin).catch(error => {
    const message = error instanceof Error ? error.message : 'Shares listing failed';
    process.stderr.write(`${message}\n`);
    process.exitCode = 1;
  });
}
