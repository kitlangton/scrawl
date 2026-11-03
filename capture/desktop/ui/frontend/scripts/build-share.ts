// Emit the two surface-owned CommonJS runtime entries from checked TypeScript.
import ts from 'typescript';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=fileURLToPath(new URL('..',import.meta.url));
const directory=path.join(root,'electron/share');
const program=ts.createProgram(['main.cts','preload.cts'].map(name=>path.join(directory,name)),{
 target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.NodeNext,moduleResolution:ts.ModuleResolutionKind.NodeNext,
 strict:true,skipLibCheck:true,types:['node'],lib:['lib.es2022.d.ts'],
 moduleDetection:ts.ModuleDetectionKind.Force,outDir:directory,rootDir:directory,noEmitOnError:true,
});
const diagnostics=ts.getPreEmitDiagnostics(program);
if(diagnostics.length)throw Error(ts.formatDiagnosticsWithColorAndContext(diagnostics,{
 getCurrentDirectory:()=>root,getCanonicalFileName:(file:string)=>file,getNewLine:()=>String.fromCharCode(10),
}));
if(program.emit().emitSkipped)throw Error('Sharing runtime emission failed');
