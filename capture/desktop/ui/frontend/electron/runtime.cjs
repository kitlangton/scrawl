// Bounded same-user Rust socket only. Never launches a backend.
const { lstat } = require('node:fs/promises');
const net = require('node:net');
const path = require('node:path');

const phases = new Set(['idle', 'selecting', 'starting', 'recording', 'processing', 'completed']);
const reportingCodes = new Set(['invalid_request', 'config_read_failed', 'config_write_failed', 'app_busy', 'admission_expired']);
// Only bounded classifications cross IPC; never return native payloads/errors or paths.
const runtimeFailure = (code) => ({ status: 'error', code,
  message: `The local app state read failed (${code}). Refresh when the app is responsive.` });
const codes = new Set(['invalid_request', 'invalid_path', 'conflict', 'admission_expired', 'outcome_unknown', 'config_write_failed',
  'config_read_failed', 'app_busy', 'app_unresponsive', 'feature_disabled', 'usage_unavailable']);
const text = (value) => typeof value === 'string' && Buffer.byteLength(value) <= 4096 && !value.includes('\0');

async function exchange(home, request, decode, maxBytes, timeoutMs, timeoutResult, failed = () => decode(null)) {
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
          !entry.isSocket() || entry.uid !== owner || (entry.mode & 0o777) !== 0o600) return failed('unsafe_endpoint');
      if (expired) return timeoutResult;
      return await new Promise((resolve) => {
        const chunks = [];
        let length = 0;
        const finish = (result) => { socket.destroy(); resolve(result); };
        socket = net.createConnection(address);
        socket.once('connect', () => socket.write(request));
        socket.on('data', (chunk) => {
          length += chunk.length;
          if (length > maxBytes) return finish(failed('response_too_large'));
          chunks.push(chunk);
          const reply = Buffer.concat(chunks);
          const newline = reply.indexOf(10);
          if (newline < 0) return;
          if (newline !== reply.length - 1) return finish(failed('invalid_frame'));
          try { finish(decode(JSON.parse(reply.subarray(0, newline).toString('utf8')))); }
          catch { finish(failed('invalid_json')); }
        });
        socket.once('error', (error) => finish(
          ['ENOENT', 'ECONNREFUSED'].includes(error.code) ? { status: 'disconnected' } : failed('transport_error'),
        ));
        socket.once('end', () => finish(failed('incomplete_response')));
      });
    } catch (error) {
      return error.code === 'ENOENT' ? { status: 'disconnected' } : failed('transport_error');
    }
  };
  try { return await Promise.race([read(), deadline]); }
  finally { clearTimeout(timer); }
}

function readRuntimeState(home, timeoutMs = 3500) {
  return exchange(home, 'state\n', (value) => !value?.error && phases.has(value?.state?.phase)
    ? { status: 'connected', phase: value.state.phase }
    : runtimeFailure(['app_busy', 'app_unresponsive', 'invalid_request'].includes(value?.error)
      ? value.error : 'invalid_response'),
  4096, timeoutMs, runtimeFailure('request_timeout'), runtimeFailure);
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

function decodeLocationFacts(value) {
  if (codes.has(value?.error)) {
    if (value.error === 'feature_disabled') return { status: 'disabled' };
    if (value.error === 'invalid_request') return { status: 'incompatible' };
    return { status: 'error', code: value.error };
  }
  const l = value?.location;
  const saved = l?.saved;
  const environment = l?.environment;
  const validSaved = saved?.kind === 'absent' || saved?.kind === 'null' || saved?.kind === 'other' ||
    (saved?.kind === 'string' && text(saved.value));
  if (!validSaved || !environment || typeof environment.present !== 'boolean' ||
      !(environment.value === null || text(environment.value)) || environment.present !== (environment.value !== null) ||
      !text(l.homePath) || !text(l.builtinPath) || !/^[a-f0-9]{16}$/.test(l.revision)) {
    return { status: 'error', code: 'invalid_response' };
  }
  const projectedSaved = saved.kind === 'string' ? { kind: 'string', value: saved.value } : { kind: saved.kind };
  return { status: 'connected', location: { saved: projectedSaved, environment: { present: environment.present, value: environment.value }, homePath: l.homePath,
    builtinPath: l.builtinPath, revision: l.revision } };
}

function storageLocation(home, operation = 'facts', fields = {}, timeoutMs = 3500) {
  if (!['facts', 'probe', 'set', 'reset'].includes(operation)) throw new Error('Invalid storage operation');
  const wireOperation = { facts: 'locationfacts', probe: 'probelocation', set: 'setlocation', reset: 'resetlocation' }[operation];
  const request = { storage: wireOperation, deadline: Number(process.hrtime.bigint() / 1000000n) + Math.min(timeoutMs, 3000) };
  if (operation !== 'facts') {
    if (!/^[a-f0-9]{16}$/.test(fields.expected)) throw new Error('Invalid revision');
    request.expected = fields.expected;
  }
  if (operation === 'set' || operation === 'probe') {
    if (!text(fields.path) || !path.isAbsolute(fields.path)) throw new Error('Invalid selected folder');
    request.path = fields.path;
  }
  const decode = operation === 'facts' || operation === 'set' || operation === 'reset' ? decodeLocationFacts : (value) => {
    if (codes.has(value?.error)) {
      if (value.error === 'feature_disabled') return { status: 'disabled' };
      if (value.error === 'invalid_request') return { status: 'incompatible' };
      return { status: 'error', code: value.error };
    }
    return typeof value?.probe?.available === 'boolean' ? { status: 'connected', available: value.probe.available }
      : { status: 'error', code: 'invalid_response' };
  };
  return exchange(home, JSON.stringify(request) + '\n', decode, 131072, timeoutMs,
    { status: 'error', code: operation === 'set' || operation === 'reset' ? 'outcome_unknown' : 'timeout' });
}

// Retained for already-installed Settings assets and the capture bridge contract.
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

function errorReporting(home, enabled, timeoutMs = 3500) {
  if (enabled !== undefined && typeof enabled !== 'boolean') throw new Error('Invalid reporting preference');
  const request = { errorReporting: enabled === undefined ? 'read' : 'set', deadline: Number(process.hrtime.bigint() / 1000000n) + Math.min(timeoutMs, 3000) };
  if (enabled !== undefined) request.enabled = enabled;
  return exchange(home, JSON.stringify(request) + '\n', (value) => {
    if (reportingCodes.has(value?.error)) return value.error === 'invalid_request' ? { status: 'incompatible' } : { status: 'error', code: value.error };
    return typeof value?.errorReporting?.enabled === 'boolean'
      ? { status: 'connected', enabled: value.errorReporting.enabled }
      : { status: 'error', code: 'invalid_response' };
  }, 4096, timeoutMs, { status: 'error', code: enabled === undefined ? 'timeout' : 'outcome_unknown' });
}

const referenceCodes = new Set(['conflict', 'no_selection', 'admission_expired', 'feature_disabled',
  'clipboard_failed', 'invalid_capture', 'app_unresponsive']);
const referenceID = (value) => text(value) && value.length > 0 && !value.includes('/');
function captureReference(home, operation = 'read', expected, timeoutMs = 3500) {
  if (!['read', 'copy'].includes(operation)) throw new Error('Invalid reference operation');
  if (operation === 'copy' && !referenceID(expected)) throw new Error('Invalid capture reference');
  const request = { reference: operation, deadline: Number(process.hrtime.bigint() / 1000000n) + Math.min(timeoutMs, 3000) };
  if (operation === 'copy') request.expected = expected;
  return exchange(home, JSON.stringify(request) + '\n', (value) => {
    if (value?.error === 'invalid_request') return { status: 'incompatible' };
    if (value?.error === 'feature_disabled') return { status: 'disabled' };
    if (referenceCodes.has(value?.error)) return { status: 'error', code: value.error };
    const r = value?.reference;
    if (!r || !(r.id === null || referenceID(r.id)) || r.copied !== (operation === 'copy') ||
        (operation === 'copy' && r.id !== expected)) return { status: 'error', code: 'invalid_response' };
    return { status: 'connected', id: r.id, copied: r.copied };
  }, 16384, timeoutMs, { status: 'error', code: operation === 'copy' ? 'outcome_unknown' : 'timeout' });
}

function storagePreference(home, operation, fields = {}, timeoutMs = 3500) {
  if (!['retention', 'saveretention', 'resetretention', 'usage'].includes(operation)) throw new Error('Invalid preference operation');
  const mutation = ['saveretention', 'resetretention'].includes(operation);
  const request = { storage: operation, deadline: Number(process.hrtime.bigint() / 1000000n) + Math.min(timeoutMs, 3000) };
  if (mutation) {
    if (!/^[a-f0-9]{16}$/.test(fields.expected)) throw new Error('Invalid revision');
    request.expected = fields.expected;
  }
  if (operation === 'saveretention') {
    const policy = fields.policy;
    const validDays = value => value === null || (Number.isInteger(value) && value >= 0 && value <= 0xffffffff);
    if (!policy || Object.keys(policy).length !== 2 || !validDays(policy.mediaDays) || !validDays(policy.bundleDays))
      throw new Error('Invalid retention policy');
    request.policy = { mediaDays: policy.mediaDays, bundleDays: policy.bundleDays };
  }
  return exchange(home, JSON.stringify(request) + '\n', (value) => {
    if (value?.error) return decodeStorage(value);
    if (operation === 'usage') {
      const u = value?.usage;
      return u && text(u.effectivePath) && text(u.displayPath) && text(u.formatted)
        ? { status: 'connected', usage: { effectivePath: u.effectivePath, displayPath: u.displayPath, formatted: u.formatted } }
        : { status: 'error', code: 'invalid_response' };
    }
    const r = value?.retention;
    const field = v => v && typeof v.present === 'boolean' &&
      (v.value === null || (Number.isInteger(v.value) && v.value >= 0 && v.value <= 0xffffffff));
    if (!r || !field(r.mediaDays) || !field(r.bundleDays) || typeof value.custom !== 'boolean' ||
        !/^[a-f0-9]{16}$/.test(value.revision)) return { status: 'error', code: 'invalid_response' };
    return { status: 'connected', policy: { mediaDays: { present: r.mediaDays.present, value: r.mediaDays.value },
      bundleDays: { present: r.bundleDays.present, value: r.bundleDays.value } }, custom: value.custom, revision: value.revision };
  }, 16384, timeoutMs, { status: 'error', code: mutation ? 'outcome_unknown' : 'timeout' });
}

module.exports = { readRuntimeState, captureLocation, storageLocation, errorReporting, captureReference, storagePreference };

// Technical failures accept no error text, paths or child-process details.
module.exports.reportException = (home, kind) => {
  if (!['main_exception', 'main_rejection', 'renderer_gone', 'child_gone'].includes(kind)) return Promise.resolve();
  return exchange(home, JSON.stringify({ errorReporting: 'exception', kind,
    deadline: Number(process.hrtime.bigint() / 1000000n) + 500 }) + '\n',
    () => undefined, 1024, 500, undefined);
};

async function publicShare(home, operation, value) {
  const request = { publicShare: operation, deadline: Number(process.hrtime.bigint()/1000000n)+3000 };
  if (operation === 'start') { request.expected = value; request.disclosed = true; }
  else request.operation = value;
  return exchange(home, JSON.stringify(request)+'\n', response => {
    if (['feature_disabled','connect_required','app_busy','invalid_capture','local_state_error','conflict','no_selection','no_operation','not_prepared','cancelled','clipboard_failed','admission_expired','app_unresponsive'].includes(response?.error)) return {status:'error',code:response.error};
    const v=response?.publicShare;
    if (!v || !['preparing','prepared','copied','failed','cancelled','expired'].includes(v.state) ||
        (operation==='start' && !/^[a-f0-9]{32}$/.test(v.operation))) return {status:'error',code:'invalid_response'};
    return {status:'connected',state:v.state,...(operation==='start'?{operation:v.operation}:{})};
  },4096,3500,{status:'error',code:'outcome_unknown'});
}
module.exports.publicShare = publicShare;

function microphoneEnabled(home, enabled, timeoutMs = 3500) {
  if (enabled !== undefined && typeof enabled !== 'boolean') throw new Error('Invalid microphone preference');
  const request = { microphone: enabled === undefined ? 'read' : 'set', deadline: Number(process.hrtime.bigint() / 1000000n) + Math.min(timeoutMs, 3000) };
  if (enabled !== undefined) request.enabled = enabled;
  return exchange(home, JSON.stringify(request) + '\n', value => {
    if (value?.error) return { status: 'error', code: reportingCodes.has(value.error) ? value.error : 'invalid_response' };
    return typeof value?.microphone?.enabled === 'boolean' ? { status: 'connected', enabled: value.microphone.enabled } : { status: 'error', code: 'invalid_response' };
  }, 4096, timeoutMs, { status: 'error', code: enabled === undefined ? 'timeout' : 'outcome_unknown' });
}
module.exports.microphoneEnabled = microphoneEnabled;
