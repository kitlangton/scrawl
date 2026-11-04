// @ts-nocheck
// Bounded compatibility route for installed Rust versions without backend RPC.
// Preserved transport code; retire once all supported runtimes expose that RPC.
const { lstat } = require('node:fs/promises');
const net = require('node:net');
const path = require('node:path');
const actions = new Set(['connect', 'allow', 'cancel', 'check', 'disable', 'disconnect', 'open-browser', 'request-disable', 'request-disconnect', 'open-shared-links']);
const failed = () => ({ status: 'error' });
const text = value => typeof value === 'string' && Buffer.byteLength(value) <= 4096 && !/[\x00-\x1f\x7f]/.test(value);
function browserUri(value, code) {
  if (!text(value) || /[\s%#\\"']/.test(value)) return false;
  try {
    const url = new URL(value);
    return !url.username && !url.password &&
      (url.protocol === 'https:' || (url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname))) &&
      (!url.search || url.search === `?code=${code}`);
  } catch { return false; }
}
function linksUri(value) {
  if (!browserUri(value, null)) return false;
  const url = new URL(value);
  return !url.search && ['/connect/shares', '/hosted/shares'].includes(url.pathname);
}
// Allowlisted semantic facts only; Connect's TypeScript owner renders them.
function presentationState(value) {
  const errors = new Set(['cancelled','offline','auth-required','rejected','expired','local-state','invalid-server','unverified-commit','unavailable']);
  const nullableBoolean = v => v === null || typeof v === 'boolean';
  const error = v => v === null || errors.has(v);
  if (!value || !['consent','approval','logout','closed'].every(k => typeof value[k] === 'boolean') ||
      !nullableBoolean(value.retryDisable) || !nullableBoolean(value.confirmation) ||
      !error(value.error) || !error(value.localError) ||
      ![null,'online','offline','auth-required'].includes(value.health) ||
      ![null,'login','probe','disable'].includes(value.operation) ||
      !(value.sharedLinksUrl === null || linksUri(value.sharedLinksUrl))) return;
  let snapshot = null;
  if (value.snapshot !== null) {
    const s = value.snapshot;
    if (!s || !['connected','uploadsEnabled','renewalRequired','reauthorizationRequired'].every(k => typeof s[k] === 'boolean') ||
        !(s.accountInitial === null || typeof s.accountInitial === 'string' && /^[\p{L}\p{N}]{1,3}$/u.test(s.accountInitial))) return;
    snapshot = {connected:s.connected, uploadsEnabled:s.uploadsEnabled, renewalRequired:s.renewalRequired,
      reauthorizationRequired:s.reauthorizationRequired, accountInitial:s.accountInitial};
  }
  return { snapshot, health:value.health, error:value.error, localError:value.localError,
    consent:value.consent, approval:value.approval, operation:value.operation, logout:value.logout,
    retryDisable:value.retryDisable, confirmation:value.confirmation, closed:value.closed, sharedLinksUrl:value.sharedLinksUrl };
}
function decode(value, action) {
  if (value?.error === 'feature_disabled') return { status: 'disabled' };
  const v = value?.connect;
  if (!v || !['title', 'status', 'detail', 'note'].every(key => text(v[key])) ||
      !(v.server === null || text(v.server)) ||
      !(v.userCode === null || (typeof v.userCode === 'string' && /^[a-zA-Z0-9-]{1,128}$/.test(v.userCode))) ||
      !(v.verificationUri === null || browserUri(v.verificationUri, v.userCode)) ||
      !Array.isArray(v.buttons) || v.buttons.length > actions.size ||
      !v.buttons.every(b => actions.has(b.command) && text(b.title) && typeof b.enabled === 'boolean') ||
      new Set(v.buttons.map(b => b.command)).size !== v.buttons.length) return failed();
  let presentation;
  const state = v.presentationState === undefined ? undefined : presentationState(v.presentationState);
  if (v.presentationState !== undefined && state === undefined) return failed();
  if (v.presentation !== undefined) {
    const p = v.presentation;
    if (!p || !['neutral','success','progress','warning','failure'].includes(p.tone) ||
        !['headline','detail','note'].every(k => text(p[k])) ||
        !['artwork','compact','accountMenu'].every(k => typeof p[k] === 'boolean') ||
        !(p.accountInitial === undefined || typeof p.accountInitial === 'string' && /^[\p{L}\p{N}]{1,3}$/u.test(p.accountInitial)) ||
        !Array.isArray(p.buttons) || p.buttons.length > actions.size ||
        !p.buttons.every(b => actions.has(b.command) && text(b.title) && typeof b.enabled === 'boolean') ||
        new Set(p.buttons.map(b => b.command)).size !== p.buttons.length ||
        !Array.isArray(p.menuActions) || !p.menuActions.every(a => p.buttons.some(b => b.command === a)) ||
        !(p.sharedLinksUrl == null || linksUri(p.sharedLinksUrl))) return failed();
    presentation = { tone:p.tone, headline:p.headline, detail:p.detail, note:p.note,
      artwork:p.artwork, compact:p.compact, accountMenu:p.accountMenu,
      ...(p.accountInitial === undefined ? {} : { accountInitial:p.accountInitial }),
      buttons:p.buttons.map(({command,title,enabled}) => ({command,title,enabled})), menuActions:[...p.menuActions], sharedLinksUrl:p.sharedLinksUrl ?? null };
  }
  if (value.openUri !== undefined && !(
      action === 'open-browser' && value.openUri === v.verificationUri && browserUri(value.openUri, v.userCode) ||
      action === 'open-shared-links' && value.openUri === (state !== undefined ? state.sharedLinksUrl : presentation?.sharedLinksUrl) && linksUri(value.openUri))) return failed();
  return { status: 'connected', view: { title: v.title, status: v.status, detail: v.detail, note: v.note,
    ...(presentation === undefined ? {} : { presentation }),
    ...(state === undefined ? {} : { presentationState: state }),
    server: v.server, verificationUri: v.verificationUri, userCode: v.userCode,
    buttons: v.buttons.map(({ command, title, enabled }) => ({ command, title, enabled })) },
    ...(value.openUri === undefined ? {} : { openUri: value.openUri }) };
}
class ConnectSettings {
  [key: string]: any;
  constructor(home, timeout = 3500) {
    this.home = home;
    this.timeout = timeout;
    this.generation = 0;
  }
  close() {
    this.generation += 1;
    this.socket?.destroy();
    this.socket = undefined;
    this.opened = false;
    this.finish?.({ status: 'cancelled' });
  }
  async request(action) {
    let timer;
    try {
      return await Promise.race([this.exchange(action), new Promise(resolve => {
        // Includes filesystem admission, not just the socket round trip. A late
        // filesystem completion observes the invalidated generation before I/O.
        timer = setTimeout(() => { this.close(); resolve({ status: 'cancelled' }); }, this.timeout);
      })]);
    } finally { clearTimeout(timer); }
  }
  async exchange(action) {
    if (action !== undefined && !actions.has(action)) throw new Error('Invalid Connect action');
    if (this.busy) return failed();
    this.busy = true;
    const generation = this.generation;
    try {
      if (!this.socket) {
        // A mutation must follow a successful read of this exact controller.
        if (action !== undefined) return failed();
        const directory = path.join(this.home, '.config/apparent/agent-control');
        const address = path.join(directory, 'capture.sock');
        let dir, entry;
        try { [dir, entry] = await Promise.all([lstat(directory), lstat(address)]); }
        catch (error) { return error.code === 'ENOENT' ? { status: 'disconnected' } : failed(); }
        if (generation !== this.generation) return { status: 'cancelled' };
        if (!dir.isDirectory() || dir.uid !== process.geteuid() || (dir.mode & 0o077) !== 0 ||
            !entry.isSocket() || entry.uid !== process.geteuid() || (entry.mode & 0o777) !== 0o600) return failed();
        const socket = net.createConnection(address);
        this.socket = socket;
        const closed = () => { if (this.socket === socket) this.close(); };
        socket.on('error', closed);
        socket.on('end', closed);
        socket.on('close', closed);
      }
      const socket = this.socket;
      const first = !this.opened;
      const result = await new Promise(resolve => {
        let bytes = Buffer.alloc(0);
        let finished = false;
        const finish = result => {
          if (finished) return;
          finished = true;
          clearTimeout(timer);
          socket.removeListener('data', onData);
          if (this.finish === finish) this.finish = undefined;
          resolve(result);
        };
        const onData = chunk => {
          bytes = Buffer.concat([bytes, chunk]);
          const newline = bytes.indexOf(10);
          if (bytes.length > 65536 || (newline >= 0 && newline !== bytes.length - 1)) { finish(failed()); return; }
          if (newline < 0) return;
          try { finish(decode(JSON.parse(bytes.subarray(0, newline).toString('utf8')), action)); }
          catch { finish(failed()); }
        };
        const timer = setTimeout(() => { finish(failed()); this.close(); }, this.timeout);
        this.finish = finish;
        socket.on('data', onData);
        const request = { connect: first ? 'settings' : action === undefined ? 'read' : 'command',
          ...(action === undefined ? {} : { command: action }),
          deadline: Number(process.hrtime.bigint() / 1000000n) + Math.min(this.timeout, 3000) };
        socket.write(JSON.stringify(request) + '\n');
      });
      if (generation !== this.generation) return { status: 'cancelled' };
      this.opened = result.status === 'connected';
      if (!this.opened) this.close();
      return result;
    } finally { this.busy = false; }
  }
}
export { ConnectSettings as LegacySettings, actions, browserUri, linksUri };
