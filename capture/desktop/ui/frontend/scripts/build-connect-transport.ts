import ts from 'typescript';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

// Keep the existing Electron/main and package consumer path. Authored source is
// TypeScript; emitted CommonJS is a build artifact and has no Rust prerequisite.
const root = path.resolve(fileURLToPath(new URL('..', import.meta.url)));
const program = ts.createProgram([`${root}/electron/connect.cts`], {
  target: ts.ScriptTarget.ES2022,
  module: ts.ModuleKind.CommonJS,
  moduleResolution: ts.ModuleResolutionKind.Node10,
  strict: true,
  skipLibCheck: true,
  esModuleInterop: true,
  types: ['node'],
  typeRoots: [`${root}/node_modules/@types`],
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
if (program.emit(undefined, (file, content) => {
  // Type-only renderer contracts participate in checking, never runtime output.
  if (file === `${root}/electron/connect.cjs`) ts.sys.writeFile(file, content);
}).emitSkipped) throw Error('Connect transport emission failed');
