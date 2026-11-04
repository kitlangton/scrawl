import assert from 'node:assert/strict';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { discardApplication, prepareApplication, publishApplication, publishValidatedDirectory } from './transaction.ts';
function app(path: string, version: string) {
  mkdirSync(join(path, 'Contents/MacOS'), { recursive: true });
  writeFileSync(join(path, 'Contents/MacOS/apparent'), version);
  writeFileSync(join(path, 'Contents/MacOS/apparent-helper'), version);
  for (const component of ['inspection', 'recipient']) {
    mkdirSync(join(path, 'Contents/MacOS', component));
    writeFileSync(join(path, 'Contents/MacOS', component, 'version'), version);
  }
}
test('exclusive publication rejects a destination created after its absence check', () => {
  const root = mkdtempSync(join(tmpdir(), 'publication-race-'));
  const destination = join(root, 'output');
  const candidate = join(root, 'candidate');
  mkdirSync(candidate);
  writeFileSync(join(candidate, 'payload'), 'validated');
  const original = fs.existsSync;
  let competingInode: number | undefined;
  fs.existsSync = ((path) => {
    if (path === destination && competingInode === undefined) {
      mkdirSync(destination);
      competingInode = statSync(destination).ino;
      return false; // The competing process published just after this check.
    }
    return original(path);
  }) as typeof fs.existsSync;
  syncBuiltinESMExports();
  try {
    assert.throws(() => publishValidatedDirectory(candidate, destination), /EEXIST/);
    assert.equal(statSync(destination).ino, competingInode);
    assert.equal(readFileSync(join(candidate, 'payload'), 'utf8'), 'validated');
  } finally {
    fs.existsSync = original;
    syncBuiltinESMExports();
    rmSync(root, { recursive: true });
  }
});
test('failed exclusive publication returns validated contents to the candidate', () => {
  const root = mkdtempSync(join(tmpdir(), 'publication-rollback-'));
  try {
    const candidate = join(root, 'candidate');
    const destination = join(root, 'output');
    mkdirSync(candidate);
    writeFileSync(join(candidate, 'payload'), 'validated');
    assert.throws(() => publishValidatedDirectory(candidate, destination, undefined,
      () => { throw Error('admission failed'); }), /admission failed/);
    assert(!existsSync(destination));
    assert.equal(readFileSync(join(candidate, 'payload'), 'utf8'), 'validated');
  } finally { rmSync(root, { recursive: true }); }
});
test('candidate cleanup failure rolls back publication and retains validated contents', () => {
  const root = mkdtempSync(join(tmpdir(), 'publication-cleanup-'));
  const candidate = join(root, 'candidate');
  const destination = join(root, 'output');
  mkdirSync(candidate);
  writeFileSync(join(candidate, 'payload'), 'validated');
  const original = fs.rmSync;
  fs.rmSync = ((path, options) => {
    if (path === candidate) throw Error('candidate cleanup failed');
    return original(path, options);
  }) as typeof fs.rmSync;
  syncBuiltinESMExports();
  try {
    assert.throws(() => publishValidatedDirectory(candidate, destination), /candidate cleanup failed/);
    assert(!existsSync(destination));
    assert.equal(readFileSync(join(candidate, 'payload'), 'utf8'), 'validated');
  } finally {
    fs.rmSync = original;
    syncBuiltinESMExports();
    rmSync(root, { recursive: true });
  }
});
test('one managed app transaction upgrades both components; failed admission leaves installed bytes intact', () => {
  const root = mkdtempSync(join(tmpdir(), 'application-transaction-'));
  try {
    const destination = join(root, 'Apparent.app');
    for (const version of ['initial', 'repeat', 'pin-upgrade']) {
      const candidate = prepareApplication(destination);
      assert.equal(statSync(join(candidate, '..')).mode & 0o777, 0o700);
      app(candidate, version);
      publishApplication(candidate, destination);
      discardApplication(candidate);
      for (const component of ['inspection', 'recipient']) assert.equal(readFileSync(join(destination, 'Contents/MacOS', component, 'version'), 'utf8'), version);
      assert.equal(statSync(destination).mode & 0o777, 0o755);
    }
    const failed = prepareApplication(destination);
    app(failed, 'unadmitted');
    // A component/signature admission failure never calls publish.
    discardApplication(failed);
    assert.equal(readFileSync(join(destination, 'Contents/MacOS/apparent'), 'utf8'), 'pin-upgrade');
    const candidate = prepareApplication(destination); app(candidate, 'rollback');
    assert.throws(() => publishValidatedDirectory(candidate, destination, () => {}, () => { throw Error('final publication failed'); }), /final publication failed/);
    assert.equal(readFileSync(join(candidate, 'Contents/MacOS/apparent'), 'utf8'), 'rollback');
    discardApplication(candidate);
    assert.equal(readFileSync(join(destination, 'Contents/MacOS/apparent'), 'utf8'), 'pin-upgrade');
    const collision = join(root, 'component'); mkdirSync(collision);
    const source = join(root, 'validated'); mkdirSync(source); chmodSync(source, 0o700);
    assert.throws(() => publishValidatedDirectory(source, collision), /already exists/);
    assert(existsSync(source) && existsSync(collision));
  } finally { rmSync(root, { recursive: true }); }
});
test('post-commit cleanup failure reports committed publication and retains the private backup', () => {
  const root = mkdtempSync(join(tmpdir(), 'publication-committed-cleanup-'));
  const destination = join(root, 'Apparent.app');
  const candidate = join(root, 'candidate.app');
  app(destination, 'old');
  app(candidate, 'new');
  const originalRemove = fs.rmSync;
  const originalWrite = fs.writeSync;
  const warnings: string[] = [];
  fs.rmSync = ((path, options) => {
    if (String(path).startsWith(join(root, '.package-publication-'))) throw Error('cleanup unavailable');
    return originalRemove(path, options);
  }) as typeof fs.rmSync;
  fs.writeSync = ((_fd: number, message: string) => { warnings.push(message); return message.length; }) as typeof fs.writeSync;
  syncBuiltinESMExports();
  try {
    assert.doesNotThrow(() => publishValidatedDirectory(candidate, destination, () => {}));
    assert.equal(readFileSync(join(destination, 'Contents/MacOS/apparent'), 'utf8'), 'new');
    const retained = readdirSync(root).filter(name => name.startsWith('.package-publication-'));
    assert.equal(retained.length, 1);
    assert.equal(readFileSync(join(root, retained[0], 'previous/Contents/MacOS/apparent'), 'utf8'), 'old');
    assert.equal(statSync(join(root, retained[0])).mode & 0o777, 0o700);
    assert.equal(warnings.length, 1);
    assert.match(warnings[0], /Publication committed; private cleanup retained at/);
  } finally {
    fs.rmSync = originalRemove;
    fs.writeSync = originalWrite;
    syncBuiltinESMExports();
    rmSync(root, { recursive: true });
  }
});
test('diagnostic failure cannot change a committed result or replace a publication error', () => {
  const root = mkdtempSync(join(tmpdir(), 'publication-report-failure-'));
  const originalRemove = fs.rmSync;
  const originalWrite = fs.writeSync;
  fs.rmSync = ((path, options) => {
    if (String(path).includes('.package-publication-')) throw Error('cleanup failed');
    return originalRemove(path, options);
  }) as typeof fs.rmSync;
  fs.writeSync = (() => { throw Error('diagnostic failed'); }) as typeof fs.writeSync;
  syncBuiltinESMExports();
  try {
    for (const fail of [false, true]) {
      const destination = join(root, `installed-${fail}`);
      const candidate = join(root, `candidate-${fail}`);
      app(destination, 'old'); app(candidate, 'new');
      const primary = Error('publication failed');
      const publish = () => publishValidatedDirectory(candidate, destination, () => {},
        () => { if (fail) throw primary; });
      if (fail) assert.throws(publish, error => error === primary);
      else assert.doesNotThrow(publish);
      assert.equal(readFileSync(join(destination, 'Contents/MacOS/apparent'), 'utf8'), fail ? 'old' : 'new');
      if (fail) assert.equal(readFileSync(join(candidate, 'Contents/MacOS/apparent'), 'utf8'), 'new');
    }
  } finally {
    fs.rmSync = originalRemove; fs.writeSync = originalWrite;
    syncBuiltinESMExports();
    rmSync(root, { recursive: true });
  }
});
