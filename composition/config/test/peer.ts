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
    : request.operation === 'read'
    ? { revision: 0, value: config.endsWith('identity.json') ? process.argv[1] : null }
    : { status: 'saved', revision: 1 };
  process.stdout.write(JSON.stringify({ protocol: 1, id, result }) + '\n');
}
