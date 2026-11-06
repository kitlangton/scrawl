#!/usr/bin/env -S node --experimental-strip-types
import { createInterface } from 'node:readline';
const config = process.argv[process.argv.indexOf('--config') + 1];
for await (const line of createInterface({ input: process.stdin })) {
  const request = JSON.parse(line);
  if (config.endsWith('hang.json')) continue;
  if (config.endsWith('exit.json')) process.exit(1);
  if (config.endsWith('malformed.json')) { process.stdout.write('{bad}\n'); continue; }
  if (config.endsWith('oversized.json')) { process.stdout.write('x'.repeat(70_000)); continue; }
  const id = config.endsWith('wrong-id.json') ? 'wrong' : request.id;
  const result = config.endsWith('unknown.json') && request.operation !== 'read'
    ? { status: 'unknown', message: 'Native publication unconfirmed' }
    : request.operation === 'readForAdoption'
    ? { owner: { present: false, revision: 0, value: null }, legacy: { present: true, value: { seen: true } } }
    : request.operation === 'readPenColor'
    ? { present: true, value: null }
    : request.operation === 'compareAndSwapPenColor' && config.endsWith('conflict.json')
    ? { status: 'conflict', current: { present: true, value: null } }
    : request.operation === 'compareAndSwapPenColor'
    ? { status: 'saved' }
    : request.operation === 'adoptLegacy' && config.endsWith('conflict.json')
    ? { status: 'conflict', snapshot: { owner: { present: true, revision: 2, value: null }, legacy: { present: false, value: null } } }
    : request.operation === 'adoptLegacy' && config.endsWith('invalid-adoption.json')
    ? { status: 'saved', revision: 2 }
    : request.operation === 'read'
    ? { revision: 0, value: config.endsWith('identity.json') ? process.argv[1] : null }
    : { status: 'saved', revision: 1 };
  process.stdout.write(JSON.stringify({ protocol: 1, id, result }) + '\n');
}
