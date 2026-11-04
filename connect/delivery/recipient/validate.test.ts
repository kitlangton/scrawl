import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { validate } from './validate.ts';
import { targets } from './seal.ts';
test('synthetic headers: all supported combinations and incompatible contracts fail closed', () => {
  for (const target of targets) {
    const bytes = Buffer.alloc(64);
    const arm = target.startsWith('aarch64');
    if (target.endsWith('linux-gnu')) {
      bytes.write('7f454c46',0,'hex'); bytes[4]=2; bytes[5]=1;
      bytes.writeUInt16LE(3,16); bytes.writeUInt16LE(arm?183:62,18);
    } else {
      bytes.writeUInt32LE(0xfeedfacf); bytes.writeUInt32LE(arm?0x100000c:0x1000007,4); bytes.writeUInt32LE(2,12);
    }
    const pin={target,sourceSha:'a'.repeat(40),run:'123',attempt:'1',sha256:createHash('sha256').update(bytes).digest('hex')};
    const manifest={schema:1,protocol:1,repository:'tryapparent/apparent',workflow:'.github/workflows/connect-delivery.yml',executable:'apparent-connect-recipient',...pin,version:`recipient-${pin.sourceSha}-123-1`};
    assert.doesNotThrow(()=>validate(bytes,manifest,pin));
    for (const key of Object.keys(manifest)) assert.throws(()=>validate(bytes,{...manifest,[key]:'invalid'},pin));
    assert.throws(()=>validate(Buffer.alloc(0),manifest,pin));
    const wrong=Buffer.from(bytes); wrong[0]^=255;
    assert.throws(()=>validate(wrong,manifest,pin));
    const digest=createHash('sha256').update(wrong).digest('hex');
    assert.throws(()=>validate(wrong,{...manifest,sha256:digest},{...pin,sha256:digest}));
  }
});
