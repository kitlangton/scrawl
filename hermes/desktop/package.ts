// Generate the loader-required JavaScript outside Git. Source and policy remain TypeScript.
import { createRequire } from 'node:module';
import { cp, mkdir, realpath } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('..', import.meta.url));
const destination = process.argv[2];
if (!destination || !path.isAbsolute(destination) || !destination.startsWith('/tmp/apparent-dev-')) throw new Error('Use an absolute per-job /tmp/apparent-dev-… package directory');
const require = createRequire(new URL('../../capture/desktop/ui/frontend/package.json', import.meta.url));
const { build } = require('esbuild');
await mkdir(destination, { recursive: true, mode: 0o700 });
const target = await realpath(destination);
if (!target.startsWith('/tmp/apparent-dev-')) throw new Error('Unsafe package destination');
await cp(root, target, { recursive: true, filter: p => !['__pycache__', '.venv', 'desktop'].includes(path.basename(p)) });
await build({ entryPoints: [path.join(root, 'desktop/plugin.ts')], bundle: true, format: 'esm', platform: 'browser', external: ['@hermes/plugin-sdk', 'react/jsx-runtime'], outfile: path.join(target, 'desktop/plugin.js') });
