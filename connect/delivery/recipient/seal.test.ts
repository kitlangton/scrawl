import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { seal } from './seal.ts';
test('seal binds actual executable bytes and refuses overwrite and invalid identities', () => {
 const directory = mkdtempSync(join(tmpdir(),'recipient-seal-'));
 try {
  const bytes=Buffer.from([0x7f,0x45,0x4c,0x46,0,255]);
  writeFileSync(join(directory,'apparent-connect-recipient'),bytes);
  const manifest=seal(directory,'x86_64-unknown-linux-gnu','a'.repeat(40),'123','1');
  assert.equal(manifest.sha256,createHash('sha256').update(bytes).digest('hex'));
  assert.throws(()=>seal(directory,manifest.target,manifest.sourceSha,'123','1'));
  assert.throws(()=>seal(directory,'windows',manifest.sourceSha,'123','1'));
  assert.throws(()=>seal(directory,manifest.target,'main','123','1'));
 } finally {rmSync(directory,{recursive:true,force:true});}
});
