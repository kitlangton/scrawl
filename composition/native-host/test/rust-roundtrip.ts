// Invoked only by the Rust listener integration test with an isolated synthetic HOME.
import assert from 'node:assert/strict';
import { createNativeHostPort } from '../src/client.ts';

const port = createNativeHostPort();
const snapshot = await port.readSnapshot();
assert.equal(snapshot.processEpoch, 'rust-fixture');
assert.equal(snapshot.scope, 'running-desktop-process');
assert.deepEqual(await port.saveRecordShortcut({ keyCode: 1, modifiers: 2560, label: '⌥⇧S' }, {
  processEpoch: snapshot.processEpoch, revision: snapshot.revision,
}), { status: 'outcome-unknown' });
console.log('Public Node client and production Rust socket preserve an admitted lost-reply outcome');
