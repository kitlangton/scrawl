import {fileURLToPath, pathToFileURL} from 'node:url';
import {lstat, rm} from 'node:fs/promises';
import path from 'node:path';
import type compiler from 'typescript';

const root = fileURLToPath(new URL('.', import.meta.url));
export async function build(ts: typeof compiler) {
  const output = path.join(root, 'dist');
  const config = ts.readConfigFile(path.join(root, 'tsconfig.json'), ts.sys.readFile);
  if (config.error) throw Error('Cannot read Record Storage compiler configuration');
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, root);
  if (parsed.options.outDir !== output || parsed.options.declarationDir) throw Error('Redirected Record Storage output refused');
  try {
    const info = await lstat(output);
    if (!info.isDirectory() || info.isSymbolicLink()) throw Error('Unsafe Record Storage output');
    await rm(output, {recursive: true});
  } catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; }
  const program = ts.createProgram(parsed.fileNames, parsed.options);
  const errors = [...parsed.errors, ...ts.getPreEmitDiagnostics(program)];
  if (errors.length) throw Error(ts.formatDiagnosticsWithColorAndContext(errors, {
    getCurrentDirectory: () => root, getCanonicalFileName: file => file, getNewLine: () => '\n',
  }));
  if (program.emit().emitSkipped) throw Error('Record Storage build failed');
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) await build((await import('typescript')).default);
