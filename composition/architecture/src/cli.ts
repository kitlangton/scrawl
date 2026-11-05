import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkArchitecture } from './guard.ts';

const args = process.argv.slice(2);
let root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
for (let i = 0; i < args.length; i++) if (args[i] === '--root' && args[i + 1]) root = path.resolve(args[++i]);
const errors = checkArchitecture(root);
if (errors.length) { console.error(errors.join('\n')); process.exitCode = 1; }
else console.log('Architecture boundaries are valid.');
