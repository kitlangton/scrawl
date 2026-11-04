/** Packaging-only transactions. Callers own validation, signing and exclusive installation locks. */
import assert from 'node:assert/strict';
import { chmodSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync, writeSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

/** Call only after validating all candidate bytes. Private staging ancestors remain 0700. */
export function shippingDirectory(candidate: string) {
  const stat = lstatSync(candidate);
  assert(stat.isDirectory() && !stat.isSymbolicLink(), 'Shipping payload must be a regular directory');
  chmodSync(candidate, 0o755);
  assert.equal(lstatSync(candidate).mode & 0o777, 0o755, 'Shipping directory must be traversable');
}

/** States: validated candidate -> previous reserved -> published -> committed.
 * On failure after reservation/publication, restore the previous directory.
 * Without an explicit previous-owner validator, existing destinations are refused.
 */
export function publishValidatedDirectory(candidate: string, destination: string,
  previousOwner?: (path: string) => void, afterPublish: () => void = () => {}) {
  const output = resolve(destination);
  if (existsSync(output)) {
    assert(previousOwner, 'Destination already exists');
    previousOwner(output);
  }
  shippingDirectory(candidate);
  if (!previousOwner) {
    // Reserve the destination exclusively. rename(candidate, output) can replace
    // a concurrently created empty directory on both supported platforms.
    // Callers keep assembly private until this operation returns successfully.
    mkdirSync(output, { mode: 0o700 });
    const moved: string[] = [];
    let candidateRemoved = false;
    try {
      for (const entry of readdirSync(candidate)) {
        renameSync(join(candidate, entry), join(output, entry));
        moved.push(entry);
      }
      shippingDirectory(output);
      rmSync(candidate, { recursive: true });
      candidateRemoved = true;
      afterPublish();
    } catch (error) {
      try {
        if (candidateRemoved && !existsSync(candidate)) mkdirSync(candidate, { mode: 0o700 });
        for (const entry of moved.reverse()) renameSync(join(output, entry), join(candidate, entry));
        rmSync(output, { recursive: true });
      } catch (rollback) {
        throw new AggregateError([error, rollback], `Publication failed; reserved payload retained at ${output}`);
      }
      throw error;
    }
    return;
  }
  const transaction = mkdtempSync(join(dirname(output), '.package-publication-'));
  chmodSync(transaction, 0o700);
  const previous = join(transaction, 'previous');
  let state: 'validated' | 'reserved' | 'published' | 'committed' | 'rolled-back' | 'rollback-failed' = 'validated';
  try {
    if (existsSync(output)) { renameSync(output, previous); state = 'reserved'; }
    renameSync(candidate, output); state = 'published';
    afterPublish(); state = 'committed';
  } catch (error) {
    try {
      if (state === 'published') {
        assert(!existsSync(candidate), 'Candidate rollback destination already exists');
        renameSync(output, candidate);
      }
      if (existsSync(previous)) renameSync(previous, output);
      state = 'rolled-back';
    } catch (rollback) {
      state = 'rollback-failed';
      throw new AggregateError([error, rollback], `Publication failed; previous application retained at ${previous}`);
    }
    throw error;
  } finally {
    if (state !== 'rollback-failed') {
      try { rmSync(transaction, { recursive: true }); }
      catch {
        // Cleanup cannot change a committed installation into a failed one, or
        // obscure the original failure after a successful rollback.
        // Do not emit a process event: application warning listeners must not
        // participate in the transaction. Even diagnostic I/O is best effort.
        try { writeSync(2, `Publication ${state}; private cleanup retained at ${transaction}\n`); }
        catch { /* The transaction result takes precedence over reporting. */ }
      }
    }
  }
}

function application(path: string) {
  assert(lstatSync(path).isDirectory() && !lstatSync(path).isSymbolicLink(), 'Unsafe application destination');
  for (const name of ['apparent', 'apparent-helper']) {
    const stat = lstatSync(join(path, 'Contents/MacOS', name));
    assert(stat.isFile() && !stat.isSymbolicLink(), 'Not an owned Apparent application');
  }
}
function appTransaction(candidate: string, destination?: string) {
  const path = resolve(candidate);
  const directory = dirname(path);
  const stat = lstatSync(directory);
  assert(stat.isDirectory() && !stat.isSymbolicLink() && (stat.mode & 0o777) === 0o700 &&
    stat.uid === process.getuid?.(), 'Unsafe application transaction');
  const record = JSON.parse(readFileSync(join(directory, 'transaction.json'), 'utf8'));
  assert(record.schema === 1 && record.candidate === path && basename(path) === 'candidate.app', 'Unowned application transaction');
  if (destination) assert.equal(record.destination, resolve(destination), 'Application destination changed');
  return { directory, record };
}
export function prepareApplication(destination: string) {
  const output = resolve(destination);
  if (existsSync(output)) application(output);
  mkdirSync(dirname(output), { recursive: true });
  const directory = mkdtempSync(join(dirname(output), '.apparent-install-'));
  chmodSync(directory, 0o700);
  const candidate = join(directory, 'candidate.app');
  mkdirSync(candidate, { mode: 0o755 });
  writeFileSync(join(directory, 'transaction.json'), JSON.stringify({ schema: 1, candidate, destination: output }), { mode: 0o600, flag: 'wx' });
  return candidate;
}
export function publishApplication(candidate: string, destination: string) {
  appTransaction(candidate, destination);
  publishPreparedApplication(candidate, destination);
}
export function publishPreparedApplication(candidate: string, destination: string) {
  application(candidate); // Native signature verification is performed by the installer first.
  publishValidatedDirectory(candidate, destination, application);
}
export function discardApplication(candidate: string) {
  const { directory } = appTransaction(candidate);
  rmSync(directory, { recursive: true });
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [operation, first, second] = process.argv.slice(2);
  assert(first, 'Application path required');
  if (operation === 'prepare-app' && !second) console.log(prepareApplication(first));
  else if (operation === 'publish-app' && second) publishApplication(first, second);
  else if (operation === 'publish-prepared-app' && second) publishPreparedApplication(first, second);
  else if (operation === 'discard-app' && !second) discardApplication(first);
  else throw new Error('Usage: transaction.ts prepare-app DEST | publish-app CANDIDATE DEST | discard-app CANDIDATE');
}
