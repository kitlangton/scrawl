// Bounded same-user Rust socket only. Never launches a backend.
const { lstat } = require('node:fs/promises');
const net = require('node:net');
const path = require('node:path');

const phases = new Set(['idle', 'selecting', 'starting', 'recording', 'processing', 'completed']);
const failure = { status: 'error', message: 'The Rust app did not return a valid state. Retry when it is responsive.' };
const codes = new Set(['invalid_request', 'invalid_path', 'conflict', 'admission_expired', 'config_write_failed',
  'config_read_failed', 'app_busy', 'app_unresponsive', 'feature_disabled']);
const text = (value) => typeof value === 'string' && Buffer.byteLength(value) <= 4096 && !value.includes('\0');

async function exchange(home, request, decode, maxBytes, timeoutMs, timeoutResult) {
  let socket;
  let timer;
  let expired = false;
  const deadline = new Promise((resolve) => {
    timer = setTimeout(() => { expired = true; socket?.destroy(); resolve(timeoutResult); }, timeoutMs);
  });
  const read = async () => {
    try {
      const directory = path.join(home, '.config/apparent/agent-control');
      const address = path.join(directory, 'capture.sock');
      const owner = process.geteuid();
      const dir = await lstat(directory);
      const entry = await lstat(address);
      if (!dir.isDirectory() || dir.uid !== owner || (dir.mode & 0o077) !== 0 ||
          !entry.isSocket() || entry.uid !== owner || (entry.mode & 0o777) !== 0o600) return decode(null);
      if (expired) return timeoutResult;
      return await new Promise((resolve) => {
        const chunks = [];
        let length = 0;
        const finish = (result) => { socket.destroy(); resolve(result); };
        socket = net.createConnection(address);
        socket.once('connect', () => socket.write(request));
        socket.on('data', (chunk) => {
          length += chunk.length;
          if (length > maxBytes) return finish(decode(null));
          chunks.push(chunk);
          const reply = Buffer.concat(chunks);
          const newline = reply.indexOf(10);
          if (newline < 0) return;
          if (newline !== reply.length - 1) return finish(decode(null));
          try { finish(decode(JSON.parse(reply.subarray(0, newline).toString('utf8')))); }
          catch { finish(decode(null)); }
        });
        socket.once('error', (error) => finish(
          ['ENOENT', 'ECONNREFUSED'].includes(error.code) ? { status: 'disconnected' } : decode(null),
        ));
        socket.once('end', () => finish(decode(null)));
      });
    } catch (error) {
      return error.code === 'ENOENT' ? { status: 'disconnected' } : decode(null);
    }
  };
  try { return await Promise.race([read(), deadline]); }
  finally { clearTimeout(timer); }
}

function readRuntimeState(home, timeoutMs = 3500) {
  return exchange(home, 'state\n', (value) => !value?.error && phases.has(value?.state?.phase)
    ? { status: 'connected', phase: value.state.phase } : failure, 4096, timeoutMs, failure);
}

function decodeStorage(value) {
  if (codes.has(value?.error)) {
    if (value.error === 'feature_disabled') return { status: 'disabled' };
    if (value.error === 'invalid_request') return { status: 'incompatible' };
    return { status: 'error', code: value.error };
  }
  const s = value?.storage;
  if (!s || !(s.savedPath === null || text(s.savedPath)) || !text(s.effectivePath) || !text(s.displayPath) ||
      typeof s.custom !== 'boolean' || s.custom !== (s.savedPath !== null) ||
      !['environment', 'configured', 'builtin', 'fallback'].includes(s.source) ||
      typeof s.revision !== 'string' || !/^[a-f0-9]{16}$/.test(s.revision)) return { status: 'error', code: 'invalid_response' };
  // Explicit projection: neither raw errors/config nor unexpected snapshot fields escape main.
  return { status: 'connected', storage: { savedPath: s.savedPath, effectivePath: s.effectivePath,
    displayPath: s.displayPath, custom: s.custom, source: s.source, revision: s.revision } };
}

function captureLocation(home, operation = 'read', fields = {}, timeoutMs = 3500) {
  if (!['read', 'set', 'reset'].includes(operation)) throw new Error('Invalid storage operation');
  const request = { storage: operation, deadline: Number(process.hrtime.bigint() / 1000000n) + Math.min(timeoutMs, 3000) };
  if (operation !== 'read') {
    if (!/^[a-f0-9]{16}$/.test(fields.expected)) throw new Error('Invalid revision');
    request.expected = fields.expected;
  }
  if (operation === 'set') {
    if (!text(fields.path) || !path.isAbsolute(fields.path)) throw new Error('Invalid selected folder');
    request.path = fields.path;
  }
  return exchange(home, JSON.stringify(request) + '\n', decodeStorage, 131072, timeoutMs,
    { status: 'error', code: operation === 'read' ? 'timeout' : 'outcome_unknown' });
}

module.exports = { readRuntimeState, captureLocation };
