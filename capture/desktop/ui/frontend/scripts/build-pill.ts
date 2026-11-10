import {readFile,writeFile} from 'node:fs/promises';
import ts from 'typescript';
import {mkdir} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
// Record owns the source; the installed app uses its already packaged Node runtime.
// No native compiler, Electron API or renderer is a dependency of this worker.
const completion = new URL('../electron/record-completion/', import.meta.url);
const recordSources = ['policy', 'worker'].map(name=>fileURLToPath(new URL(`../../../../../record/completion/src/${name}.ts`,import.meta.url)));
const program = ts.createProgram(recordSources,{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.NodeNext,moduleResolution:ts.ModuleResolutionKind.NodeNext,
 strict:true,skipLibCheck:true,noEmit:true,allowImportingTsExtensions:true,types:['node'],typeRoots:[fileURLToPath(new URL('../node_modules/@types',import.meta.url))]});
const diagnostics=ts.getPreEmitDiagnostics(program);
if(diagnostics.length)throw Error(ts.formatDiagnosticsWithColorAndContext(diagnostics,{getCanonicalFileName:f=>f,getCurrentDirectory:()=>process.cwd(),getNewLine:()=> '\n'}));
await mkdir(completion, {recursive: true});
for (const name of ['policy', 'worker']) {
 const source=await readFile(new URL(`../../../../../record/completion/src/${name}.ts`,import.meta.url),'utf8');
 const output=ts.transpileModule(source,{fileName:`${name}.ts`,compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,rewriteRelativeImportExtensions:true}});
 await writeFile(new URL(`${name}.js`,completion),output.outputText);
}
await writeFile(new URL('package.json',completion),JSON.stringify({type:'module'}));
// CommonJS is required by Electron's sandboxed preload; generated files stay out of Git.
for (const name of ['main','preload','protocol','geometry']) {
 const input=new URL(`../electron/pill/${name}.cts`,import.meta.url);
 const source=await readFile(input,'utf8');
 const output=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.CommonJS}});
 await writeFile(new URL(`../electron/pill/${name}.cjs`,import.meta.url),output.outputText);
}

for (const name of ['host','framing']) {
 const source=await readFile(new URL(`../tests/pill/${name}.cts`,import.meta.url),'utf8');
 await writeFile(new URL(`../tests/pill/${name}.cjs`,import.meta.url),ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.CommonJS}}).outputText);
}
