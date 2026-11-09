#!/usr/bin/env -S node --experimental-strip-types
import { readFileSync, writeFileSync, appendFileSync, existsSync } from 'node:fs';
const args = process.argv.slice(2);
const file = process.env.APPARENT_SYNTHETIC_RECEIPT!;
const trace = process.env.APPARENT_SYNTHETIC_TRACE!;
appendFileSync(trace, JSON.stringify(args) + '\n');
let receipt: { capture_id: string; status: string; source?: string };
if (args[0] === 'launch') { writeFileSync(file + '.launched', '1'); process.exit(0); }
if (args[1] === 'state') {
  if (process.env.APPARENT_SYNTHETIC_NEEDS_LAUNCH === '1' && !existsSync(file + '.launched')) process.exit(1);
  console.log('{"state":{"phase":"idle"}}'); process.exit(0);
}
if (args[1] === 'start') {
  receipt = { capture_id: args[args.indexOf('--request-id') + 1], status: 'recording' };
  writeFileSync(file, JSON.stringify(receipt));
} else {
  receipt = JSON.parse(readFileSync(file, 'utf8'));
  if (args[1] === 'inspect' && existsSync(file + '.finished')) receipt.status = 'saved';
  if (args[1] === 'handoff') receipt.source = '/synthetic/owned.apparent';
}
console.log(JSON.stringify(receipt));
