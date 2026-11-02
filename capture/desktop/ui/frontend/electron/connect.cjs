// One Rust controller per owned stream. EOF cancels login; never retry a command.
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
  if (v.presentation !== undefined) {
    const p = v.presentation;
    if (!p || !['neutral','success','progress','warning','failure'].includes(p.tone) ||
        !['headline','detail','note'].every(k => text(p[k])) ||
        !['artwork','compact','accountMenu'].every(k => typeof p[k] === 'boolean') ||
        !Array.isArray(p.buttons) || p.buttons.length > actions.size ||
        !p.buttons.every(b => actions.has(b.command) && text(b.title) && typeof b.enabled === 'boolean') ||
        new Set(p.buttons.map(b => b.command)).size !== p.buttons.length ||
        !Array.isArray(p.menuActions) || !p.menuActions.every(a => p.buttons.some(b => b.command === a)) ||
        !(p.sharedLinksUrl == null || linksUri(p.sharedLinksUrl))) return failed();
    presentation = { tone:p.tone, headline:p.headline, detail:p.detail, note:p.note,
      artwork:p.artwork, compact:p.compact, accountMenu:p.accountMenu,
      buttons:p.buttons.map(({command,title,enabled}) => ({command,title,enabled})), menuActions:[...p.menuActions], sharedLinksUrl:p.sharedLinksUrl ?? null };
  }
  if (value.openUri !== undefined && !(
      action === 'open-browser' && value.openUri === v.verificationUri && browserUri(value.openUri, v.userCode) ||
      action === 'open-shared-links' && value.openUri === presentation?.sharedLinksUrl && linksUri(value.openUri))) return failed();
  return { status: 'connected', view: { title: v.title, status: v.status, detail: v.detail, note: v.note,
    ...(presentation === undefined ? {} : { presentation }),
    server: v.server, verificationUri: v.verificationUri, userCode: v.userCode,
    buttons: v.buttons.map(({ command, title, enabled }) => ({ command, title, enabled })) },
    ...(value.openUri === undefined ? {} : { openUri: value.openUri }) };
}
class ConnectSettings {
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
module.exports = { ConnectSettings, actions, browserUri };
