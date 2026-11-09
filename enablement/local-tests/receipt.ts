// Disposable, same-user local evidence. No caller-selected receipt paths.
import {createHash, createHmac, randomBytes} from 'node:crypto';
import {closeSync, constants, fstatSync, fsyncSync, lstatSync, mkdirSync, mkdtempSync,
  openSync, readFileSync, readdirSync, realpathSync, renameSync, rmSync, writeFileSync} from 'node:fs';
import {join} from 'node:path';

export const digest = (value: string | Buffer) => createHash('sha256').update(value).digest('hex');
export type Snapshot = {key: string; stable: string; head: string};
const lifetime = 6 * 60 * 60 * 1000;

function privateEntry(path: string, directory = false) {
  const info = lstatSync(path);
  if (info.uid !== process.getuid!() || (info.mode & 0o777) !== (directory ? 0o700 : 0o600)
    || !(directory ? info.isDirectory() : info.isFile()) || (!directory && info.nlink !== 1)) {
    throw Error('untrusted receipt storage');
  }
  return `${info.dev}:${info.ino}`;
}

function privateRead(path: string): Buffer {
  privateEntry(path);
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const info = fstatSync(fd);
    if (info.uid !== process.getuid!() || (info.mode & 0o777) !== 0o600 || info.nlink !== 1 || !info.isFile()) throw Error('untrusted receipt');
    return readFileSync(fd);
  } finally { closeSync(fd); }
}

// The namespace is worktree-local and bounded to one latest success. Removing
// its private /tmp job simply loses the optimization. Never repair unsafe state.
export function receiptStore(cwd: string): string {
  const prefix = `apparent-dev-local-reuse-${digest(realpathSync(cwd)).slice(0, 20)}-`;
  const existing = readdirSync('/tmp').filter(name => name.startsWith(prefix)).sort();
  if (existing.length) {
    const path = join('/tmp', existing[0]);
    privateEntry(path, true);
    return path;
  }
  const path = mkdtempSync(join('/tmp', prefix));
  writeFileSync(join(path, 'key'), randomBytes(32), {flag: 'wx', mode: 0o600});
  return path;
}

export function reuse(cwd: string, snapshot: () => Snapshot, run: () => void,
  report: (message: string) => void = console.error): void {
  let directory: string, identity: string, key: Buffer;
  let trusted: (() => void) | undefined;
  const invalidate = () => {
    trusted!();
    const temporary = join(directory, `epoch-${randomBytes(12).toString('hex')}`);
    try {
      writeFileSync(temporary, randomBytes(32).toString('hex'), {flag: 'wx', mode: 0o600});
      trusted!(); renameSync(temporary, join(directory, 'epoch'));
    } finally { try { trusted!(); rmSync(temporary, {force: true}); } catch { /* disposable */ } }
  };
  try {
    directory = receiptStore(cwd);
    identity = privateEntry(directory, true);
    key = privateRead(join(directory, 'key'));
    if (key.length !== 32) throw Error('invalid receipt key');
    trusted = () => {
      if (privateEntry(directory, true) !== identity || !privateRead(join(directory, 'key')).equals(key)) throw Error('receipt storage replaced');
    };
    mkdirSync(join(directory, 'lock'), {mode: 0o700});
  } catch {
    report('local-reuse: fresh (storage unavailable, untrusted or busy)');
    if (!trusted) { run(); return; }
    // A non-holder must invalidate both prior evidence and anything the holder
    // might publish while this fresh command runs. Interrupted markers block
    // reuse until disposable storage is removed; never guess that a run ended.
    let marker: string | undefined;
    try {
      trusted(); marker = join(directory!, `busy-${randomBytes(12).toString('hex')}`);
      writeFileSync(marker, '', {flag: 'wx', mode: 0o600});
      invalidate();
    } catch { /* Storage errors lose the optimization, not fresh execution. */ }
    try { run(); } finally {
      try { trusted(); invalidate(); if (marker) rmSync(marker, {force: true}); } catch { /* Keep interrupted marker. */ }
    }
    return;
  }
  const epoch = () => {
    trusted!();
    if (readdirSync(directory).some(name => name.startsWith('busy-'))) throw Error('concurrent fresh command');
    let value: string;
    try { value = privateRead(join(directory, 'epoch')).toString(); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return ''; throw error; }
    if (!/^[a-f0-9]{64}$/.test(value)) throw Error('invalid evidence epoch');
    return value;
  };
  const receipt = join(directory, 'success.json');
  const seal = (payload: string) => createHmac('sha256', key).update(payload).digest('hex');
  let temporary: string | undefined;
  try {
    let before: Snapshot;
    try { before = snapshot(); } catch {
      // Discard any earlier success before attempting an unqualified command.
      try { trusted(); rmSync(receipt, {force: true}); } catch { /* no reusable evidence */ }
      report('local-reuse: fresh (inputs unqualified)');
      run(); return;
    }
    try {
      trusted();
      const generation = epoch();
      const saved = JSON.parse(privateRead(receipt).toString());
      if (typeof saved.payload !== 'string' || saved.seal !== seal(saved.payload)) throw Error('invalid seal');
      const data = JSON.parse(saved.payload);
      const age = Date.now() - data.finished;
      if (data.schema !== 2 || data.epoch !== generation || data.key !== before.key || !Number.isFinite(age) || age < 0 || age > lifetime
        || !/^[a-f0-9]{40}$/.test(data.head) || !Number.isFinite(data.elapsedMs) || data.elapsedMs < 0) throw Error('incompatible receipt');
      const after = snapshot();
      if (JSON.stringify(before) !== JSON.stringify(after)) throw Error('inputs changed during lookup');
      trusted();
      if (epoch() !== generation) throw Error('evidence invalidated during lookup');
      report(`local-reuse: reused service tests from ${data.head} (${Math.round(data.elapsedMs)} ms), receipt ${receipt}`);
      return;
    } catch { /* Missing, malformed, expired or mutated evidence executes fresh. */ }
    try {
      trusted();
      rmSync(receipt, {force: true}); // Failures/signals cannot leave an old success.
    } catch {
      report('local-reuse: fresh (storage changed)');
      run(); return;
    }
    report('local-reuse: fresh (no compatible success)');
    let generation: string | undefined;
    try { generation = epoch(); } catch { /* Fresh command runs without reusable evidence. */ }
    const started = performance.now();
    run(); // Deliberately outside the storage-error catch: propagate test failure.
    const elapsedMs = performance.now() - started;
    try {
      const after = snapshot();
      if (JSON.stringify(before) !== JSON.stringify(after)) throw Error('inputs changed during tests');
      trusted();
      if (generation === undefined || epoch() !== generation) throw Error('concurrent execution invalidated evidence');
      const payload = JSON.stringify({schema: 2, epoch: generation, key: before.key, head: before.head, finished: Date.now(), elapsedMs});
      temporary = join(directory, `receipt-${randomBytes(12).toString('hex')}`);
      const fd = openSync(temporary, 'wx', 0o600);
      try { writeFileSync(fd, JSON.stringify({payload, seal: seal(payload)})); fsyncSync(fd); } finally { closeSync(fd); }
      trusted();
      renameSync(temporary, receipt);
      if (epoch() !== generation) { rmSync(receipt, {force: true}); throw Error('evidence invalidated during publication'); }
      report(`local-reuse: saved service tests (${Math.round(elapsedMs)} ms), receipt ${receipt}`);
    } catch { report('local-reuse: success not saved (inputs or storage changed)'); }
  } finally {
    // Cleanup must neither hide a test failure nor touch replacement storage.
    try { trusted(); if (temporary) rmSync(temporary, {force: true}); rmSync(join(directory, 'lock'), {recursive: true}); } catch { /* disposable */ }
  }
}
