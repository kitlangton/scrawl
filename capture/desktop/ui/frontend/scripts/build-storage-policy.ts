import ts from 'typescript';
import path from 'node:path';
import { copyFile, mkdir, rm } from 'node:fs/promises';
import {fileURLToPath} from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const source = path.join(root, 'src/storage/location-policy.ts');
const temporary = path.join(root, '.storage-policy-build');
const program = ts.createProgram([source], {
  target: ts.ScriptTarget.ES2022,
  module: ts.ModuleKind.CommonJS,
  moduleResolution: ts.ModuleResolutionKind.Node10,
  strict: true,
  skipLibCheck: true,
  types: ['vite/client', 'node'],
  lib: ['lib.es2022.d.ts', 'lib.dom.d.ts'],
  outDir: temporary,
  rootDir: path.join(root, 'src'),
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
await copyFile(path.join(temporary, 'storage/location-policy.js'), path.join(root, 'electron/storage-policy.cjs'));
await rm(temporary, { recursive: true, force: true });
