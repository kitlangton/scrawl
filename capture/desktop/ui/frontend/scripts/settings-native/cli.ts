import {pathToFileURL} from 'node:url';
import path from 'node:path';
import {execute, frontend, prepare, run, verify} from './provider.ts';

export async function main(args: string[]) {
  if (args.length === 1 && args[0] === 'prepare') { await prepare(); return; }
  if (args.length !== 2 || !['provider', 'test'].includes(args[0])) throw Error('Use prepare, provider <fixture>, or test <fixture>; no implicit Cargo build');
  const directory = path.resolve(args[1]);
  await verify(directory);
  if (args[0] === 'test') {
    await run('npm', ['run', 'build'], frontend);
    await run('npm', ['test'], frontend);
    await run('xvfb-run', ['-a', 'npm', 'run', 'test:electron'], frontend);
  } else {
    await run('node', ['--experimental-strip-types', 'scripts/build-storage-policy.ts'], frontend);
  }
  await execute(directory);
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) await main(process.argv.slice(2));
