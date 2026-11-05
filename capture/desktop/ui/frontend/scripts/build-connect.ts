// Compile the stream-owned account controller/transport into the existing
// Desktop runtime asset tree. Generation needs TypeScript, never Cargo.
import ts from 'typescript';
import path from 'node:path';
import fs from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('..', import.meta.url));
const source = path.resolve(root, '../../../../connect/delivery/client/src');
const output = path.join(root, 'electron/connect-client');
await fs.mkdir(output, { recursive: true });
// The pure presentation module is ESM; controller and transport have explicit
// .cjs extensions. Keep interpretation independent of the staging package type.
await fs.writeFile(path.join(output, 'package.json'), JSON.stringify({ private: true, type: 'module' })+'\n');
function emit(entry: string, rootDir: string, outDir: string, declaration: boolean) {
  const program = ts.createProgram([entry], {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.NodeNext,
    moduleResolution: ts.ModuleResolutionKind.NodeNext, strict: true,
    skipLibCheck: true, types: ['node'], typeRoots: [path.join(root,'node_modules/@types')],
    moduleDetection: ts.ModuleDetectionKind.Force, rootDir, outDir,
    noEmitOnError: true, declaration,
  });
  const diagnostics = ts.getPreEmitDiagnostics(program);
  if (diagnostics.length) throw Error(ts.formatDiagnosticsWithColorAndContext(diagnostics, {
    getCurrentDirectory:()=>root, getCanonicalFileName:(file:string)=>file, getNewLine:()=> '\n',
  }));
  if (program.emit().emitSkipped) throw Error('Connect runtime emission failed');
}
emit(path.join(source,'settings-transport.cts'), source, output, true);
emit(path.join(root,'electron/connect.cts'), path.join(root,'electron'), path.join(root,'electron'), false);
