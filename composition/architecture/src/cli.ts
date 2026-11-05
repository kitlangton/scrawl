import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkArchitecture, type Owner } from './guard.ts';

const args = process.argv.slice(2);
let root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const owners: Owner[] = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--root' && args[i + 1]) root = path.resolve(args[++i]);
  else if (args[i] === '--owner' && ['engine', 'record', 'agent', 'onboarding', 'connect', 'shared', 'composition'].includes(args[i + 1])) owners.push(args[++i] as Owner);
  else throw new Error('Usage: cli.ts [--root PATH] [--owner engine|record|agent|onboarding|connect|shared|composition]');
}
const errors = checkArchitecture(root, owners.length ? owners : undefined);
if (errors.length) { console.error(errors.join('\n')); process.exitCode = 1; }
else console.log('Architecture boundaries are valid.');
