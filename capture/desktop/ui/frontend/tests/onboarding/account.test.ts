import {test} from 'node:test';
import assert from 'node:assert/strict';
import {projectAccount} from '../../electron/onboarding/account.ts';

test('account projection exposes only public approval data and safe failure codes', () => {
  const value = {status: 'approval', userCode: 'TEST-CODE', verificationUri: 'https://app.example.test/connect/device?code=TEST-CODE',
    accessToken: 'private', deviceCode: 'private-device', error: 'private transport diagnostic'};
  assert.deepEqual(projectAccount(value), {status: 'approval', userCode: 'TEST-CODE', verificationUri: value.verificationUri, error: null});
  const complete = projectAccount({...value, status: 'signed-in'});
  assert.equal(complete?.userCode, null); assert.equal(complete?.verificationUri, null);
  assert.equal(projectAccount({status: 'failed', error: 'expired'})?.error, 'expired');
});

test('malformed or unsafe approval is unavailable rather than a signed-in state', () => {
  for (const verificationUri of ['javascript:alert(1)', 'https://name:secret@example.test/', 'https://example.test/#private', 'http://remote.test/', 'https://example.test/a\nb']) {
    assert.equal(projectAccount({status: 'approval', userCode: 'TEST-CODE', verificationUri}), null);
  }
  for (const value of [null, [], {status: 'complete'}, {status: 'approval'}, {status: 'approval', userCode: 'private code', verificationUri: 'https://example.test/'}]) assert.equal(projectAccount(value), null);
});
