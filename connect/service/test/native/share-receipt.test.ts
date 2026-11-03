import {test} from 'node:test';
import assert from 'node:assert/strict';
import {captureFailureReceipt} from './share-receipt.ts';
test('native failure keeps protocol reason while excluding arbitrary output and paths', () => {
  assert.deepEqual(captureFailureReceipt('stop', JSON.stringify({status:'failed', error:'finalization_failed', bundle_path:'/private/synthetic-secret', message:'private token'})),
    {operation:'stop', status:'failed', error:'finalization_failed'});
  for (const stdout of ['private token', 'null', '[]', JSON.stringify({status:'private token', error:'/private/synthetic-secret', extra:'secret'})]) {
    const result = captureFailureReceipt('private token', stdout);
    assert(!JSON.stringify(result).includes('private'));
    assert(!JSON.stringify(result).includes('secret'));
  }
  assert.deepEqual(captureFailureReceipt('start', JSON.stringify({status:['saved'], error:{toString:'private token'}})),
    {operation:'start', status:'unknown', error:'unknown'});
});
