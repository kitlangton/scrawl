import ts from 'typescript';
import path from 'node:path';
import { copyFile, mkdir, rm, cp, realpath, lstat } from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {build} from '../../../../../record/storage/build.ts';

const root = fileURLToPath(new URL('..', import.meta.url));
await build(ts);
const packageRoot = path.resolve(root, '../../../../record/storage');
const installed = path.join(root, 'node_modules/@apparent/record-storage');
// npm's file dependency may be an installed copy or a source link. Stage the
// producer's compiled exports explicitly; never make a consumer load private TS.
if (await realpath(installed) !== await realpath(packageRoot)) {
  if ((await lstat(installed)).isSymbolicLink()) throw Error('Unexpected Record Storage dependency link');
  const destination = path.join(installed, 'dist');
  try {
    const info = await lstat(destination);
    if (!info.isDirectory() || info.isSymbolicLink()) throw Error('Unsafe installed Record Storage output');
    await rm(destination, {recursive: true});
  } catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; }
  await cp(path.join(packageRoot, 'dist'), destination, {recursive: true});
}
const owner = path.join(packageRoot, 'src');
const source = path.join(owner, 'location-policy.ts');
const temporary = path.join(root, '.storage-policy-build');
const program = ts.createProgram([source], {
  target: ts.ScriptTarget.ES2022,
  module: ts.ModuleKind.CommonJS,
  moduleResolution: ts.ModuleResolutionKind.Node10,
  strict: true,
  skipLibCheck: true,
  types: [],
  lib: ['lib.es2022.d.ts', 'lib.dom.d.ts'],
  outDir: temporary,
  rootDir: owner,
  noEmitOnError: true,
});
const diagnostics = ts.getPreEmitDiagnostics(program);
if (diagnostics.length) {
  throw Error(ts.formatDiagnosticsWithColorAndContext(diagnostics, {
    getCurrentDirectory: () => root,
    getCanonicalFileName: file => file,
    getNewLine: () => '\n',
  }));
}
if (program.emit().emitSkipped) throw Error('Storage policy emission failed');
await mkdir(path.join(root, 'electron'), { recursive: true });
await copyFile(path.join(temporary, 'location-policy.js'), path.join(root, 'electron/storage-policy.cjs'));
await rm(temporary, { recursive: true, force: true });
