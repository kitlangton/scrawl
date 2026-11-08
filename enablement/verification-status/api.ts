import {spawnSync} from 'node:child_process';

export type API = (path: string, body?: unknown) => any;
export class ReporterHalt extends Error {
  constructor(message: string, readonly retryAt?: number) { super(message); }
}
export type Response = {status: number; headers: Record<string, string>; data: any};
export function decodeResponse(raw: string): Response {
  // gh --include emits response headers followed by JSON. Redirect header blocks
  // may precede the final response; never print request headers or credentials.
  let rest = raw;
  let status = 0;
  let headers: Record<string, string> = {};
  while (rest.startsWith('HTTP/')) {
    const boundary = /\r?\n\r?\n/.exec(rest);
    if (!boundary) throw Error('Incomplete API response headers');
    const lines = rest.slice(0, boundary.index).split(/\r?\n/);
    status = Number(/^HTTP\/\S+\s+(\d+)/.exec(lines.shift()!)?.[1]);
    headers = {};
    for (const line of lines) {
      const colon = line.indexOf(':');
      if (colon > 0) headers[line.slice(0, colon).toLowerCase()] = line.slice(colon + 1).trim();
    }
    rest = rest.slice(boundary.index + boundary[0].length);
  }
  if (!status) throw Error('Missing API response status');
  try { return {status, headers, data: rest.trim() ? JSON.parse(rest) : null}; }
  catch { throw Error('Invalid GitHub API JSON response'); }
}
export function request(path: string, body?: unknown): Response {
  const result = spawnSync('gh', ['api', '--include', path, ...(body ? ['--method', 'POST', '--input', '-'] : [])], {
    input: body ? JSON.stringify(body) : undefined, encoding: 'utf8', timeout: 20_000,
    maxBuffer: 8 * 1024 * 1024,
  });
  if (result.error) throw Error('GitHub API transport failed');
  // gh also returns the HTTP response on nonzero exits. Retain quota headers,
  // but do not propagate its process/env/response dump into Actions logs.
  if (!result.stdout.startsWith('HTTP/')) throw Error('GitHub API response unavailable');
  return decodeResponse(result.stdout);
}
export function createAPI(options: {maximum?: number; transport?: typeof request; now?: () => number; pause?: (ms: number) => void} = {}) {
  const maximum = options.maximum ?? 60;
  const transport = options.transport ?? request;
  const now = options.now ?? Date.now;
  const pause = options.pause ?? (ms => {Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);});
  let lastWrite: number | undefined;
  const immutable = new Map<string, any>();
  let calls = 0;
  let remaining: number | undefined;
  let resetAt: number | undefined;
  let halt: ReporterHalt | undefined;
  const stop = (reason: string, retryAt = resetAt): never => {
    const recovery = retryAt ? ` Retry no earlier than ${new Date(retryAt).toISOString()}, then preflight this Actions credential again.` : ' No automatic retry; inspect the same Actions credential quota before one targeted recovery.';
    halt = new ReporterHalt(reason + recovery, retryAt);
    throw halt;
  };
  const fetch: API = (path, body) => {
    if (halt) throw halt;
    if (calls >= maximum) stop(`Reporter request budget exhausted (${maximum}).`);
    if (remaining !== undefined && remaining <= 5) stop('Reporter stopped with five API requests reserved.');
    const cacheable = !body && /\/contents\/.+\?ref=[a-f0-9]{40}$/.test(path);
    if (cacheable && immutable.has(path)) return immutable.get(path);
    calls++;
    if (body && lastWrite !== undefined) {
      const delay = Math.max(0, lastWrite + 1000 - now());
      if (delay) pause(delay);
    }
    if (body) lastWrite = now();
    const response = transport(path, body);
    const {headers, status, data} = response;
    if (headers['x-ratelimit-remaining'] !== undefined) {
      remaining = Number(headers['x-ratelimit-remaining']);
      if (!Number.isInteger(remaining) || remaining < 0) stop('Invalid API quota headers.');
    }
    if (headers['x-ratelimit-reset']) {
      const reset = Number(headers['x-ratelimit-reset']);
      if (!Number.isInteger(reset) || reset <= 0 || reset > 9_999_999_999) stop('Invalid API reset headers.');
      resetAt = reset * 1000;
    }
    const retryAfter = Number(headers['retry-after']);
    if (status === 429 || (status === 403 && (remaining === 0 || /rate limit/i.test(data?.message ?? '')))) {
      const retryAt = Math.max(resetAt || 0, retryAfter > 0 ? now() + retryAfter * 1000 : 0);
      stop(`GitHub API rate limit (HTTP ${status}; request ${headers['x-github-request-id'] || 'unknown'}).`, retryAt || now() + 60_000);
    }
    if (status < 200 || status >= 300) throw Error(`GitHub API HTTP ${status}; request ${headers['x-github-request-id'] || 'unknown'}`);
    if (cacheable) immutable.set(path, data);
    return data;
  };
  return Object.assign(fetch, {
    preflight() {
      const quota = fetch('rate_limit').resources?.core;
      if (!Number.isInteger(quota?.remaining) || !Number.isInteger(quota?.reset)) stop('Invalid Actions API quota response.');
      remaining = quota.remaining;
      resetAt = quota.reset * 1000;
      if (remaining! < maximum + 10) stop(`Insufficient Actions API quota: ${remaining} remaining, ${maximum + 10} required.`);
      return {remaining, resetAt, maximum};
    },
    count: () => calls,
  });
}
