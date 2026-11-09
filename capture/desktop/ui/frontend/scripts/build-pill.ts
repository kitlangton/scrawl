import {readFile,writeFile} from 'node:fs/promises';
import ts from 'typescript';
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
