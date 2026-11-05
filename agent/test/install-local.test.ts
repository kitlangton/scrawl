import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, existsSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

test('offline installer rejects missing trusted digests before touching a prefix', () => {
  const root=mkdtempSync(join('/tmp','apparent-install-negative-'));
  try {
    const prefix=join(root,'fresh prefix');
    const script=fileURLToPath(new URL('../src/install-local.ts',import.meta.url));
    const result=spawnSync(process.execPath,['--experimental-strip-types',script,'--agent','/missing.tgz','--native-root','/missing','--prefix',prefix],{encoding:'utf8',timeout:10000});
    assert.notEqual(result.status,0);
    assert.match(result.stderr,/usage:/);
    assert.equal(existsSync(prefix),false);
  } finally { rmSync(root,{recursive:true,force:true}); }
});

test('offline installer rejects a mismatched Agent archive digest before prefix creation', () => {
  const root=mkdtempSync(join('/tmp','apparent-install-hash-'));
  try {
    const prefix=join(root,'fresh');
    const script=fileURLToPath(new URL('../src/install-local.ts',import.meta.url));
    const result=spawnSync(process.execPath,['--experimental-strip-types',script,'--agent','/missing.tgz','--agent-sha256','0'.repeat(64),'--native-root','/missing','--manifest-sha256','0'.repeat(64),'--prefix',prefix],{encoding:'utf8',timeout:10000});
    assert.notEqual(result.status,0);
    assert.match(result.stderr,/ENOENT/);
    assert.equal(existsSync(prefix),false);
  } finally { rmSync(root,{recursive:true,force:true}); }
});
