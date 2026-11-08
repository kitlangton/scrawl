// Parse source only: no module linking/evaluation, archive extraction or VM.
import assert from 'node:assert/strict';
import * as fs from 'node:fs';
import {basename,resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule} from 'node:vm';
import {pathToFileURL} from 'node:url';

function small(path:string,max:number){const fd=fs.openSync(path,fs.constants.O_RDONLY|fs.constants.O_NOFOLLOW);try{const a=fs.fstatSync(fd);assert(a.isFile()&&a.size<=max,'fixture metadata/source exceeds bound');const b=fs.readFileSync(fd);for(const z of[fs.fstatSync(fd),fs.lstatSync(path)])for(const k of['dev','ino','size','mtimeMs','ctimeMs']as const)assert.equal(a[k],z[k],'fixture changed');return b.toString('utf8');}finally{fs.closeSync(fd);}}

export function validateFixtureClosure(binary:string,files:string[]){
 assert(files.length<=128,'too many flat fixture files');
 const staged=new Map<string,string>([['apparent',resolve(binary)]]);
 for(const file of files){const name=basename(file);assert(!staged.has(name),'duplicate staged basename: '+name);staged.set(name,resolve(file));}
 const manifest=staged.get('input-seals.sha256');assert(manifest,'missing input-seals.sha256');
 const sealed=new Set<string>();
 for(const line of small(manifest,65536).split('\n').filter(Boolean)){
   const match=/^[a-f0-9]{64}  ([A-Za-z0-9_][A-Za-z0-9_.-]*)$/.exec(line);assert(match,'unsupported seal record');const name=match[1];assert(!sealed.has(name),'duplicate seal: '+name);sealed.add(name);assert(staged.has(name),'sealed file not staged: '+name);
 }
 assert(sealed.size>0,'empty fixture manifest');
 const dependencies=[]as {file:string;dependency:string}[];
 for(const[name,path]of staged){if(!name.endsWith('.ts'))continue;
   // The Node parser understands multiline imports, comments and re-exports.
   // Construction does not link/evaluate even adversarial top-level code.
   const code=stripTypeScriptTypes(small(path,4*1024*1024));
   const parsed=new SourceTextModule(code);
   for(const specifier of parsed.dependencySpecifiers){
     if(specifier.startsWith('node:'))continue;
     assert(/^\.\/[A-Za-z0-9_][A-Za-z0-9_.-]*$/.test(specifier),'unsupported non-flat guest import in '+name);
     const dependency=specifier.slice(2);assert(staged.has(dependency),'imported file not staged: '+dependency);assert(sealed.has(dependency),'imported file not sealed: '+dependency);dependencies.push({file:name,dependency});
   }
   // Relative dynamic imports cannot be inferred from host-side paths. This
   // contract accepts computed archive imports only via existing installer
   // seals; it does not claim to resolve arbitrary runtime expressions.
   assert(!/\bimport\s*\(\s*['"`]\./.test(code),'relative dynamic guest import requires explicit static staging');
 }
 return{sealed:sealed.size,staged:staged.size,dependencies,scope:'flat seals and static ESM imports only; runtime archive/install guards unchanged'};
}

export function validateRunnerClosure(argv:string[]){
 assert(Array.isArray(argv)&&argv.length<=512&&argv.every(v=>typeof v==='string'),'invalid runner arguments');
 assert.equal(argv.filter(v=>v==='--binary').length,1,'exactly one binary required');
 const binary=argv[argv.indexOf('--binary')+1];assert(binary&&!binary.startsWith('--'),'missing binary value');
 const files=argv.flatMap((v,i)=>{if(v!=='--fixture-file')return[];assert(argv[i+1]&&!argv[i+1].startsWith('--'),'missing fixture value');return[argv[i+1]];});
 return validateFixtureClosure(binary,files);
}

export function validateConfigClosure(path:string,expected:string){
 assert(/^[a-f0-9]{64}$/.test(expected),'expected config SHA required');
 const bytes=small(path,4*1024*1024);assert.equal(createHash('sha256').update(bytes).digest('hex'),expected,'config changed');
 return validateRunnerClosure(JSON.parse(bytes).runner);
}

if(process.argv[1]&&import.meta.url===pathToFileURL(process.argv[1]).href){try{
 if(process.argv[2]==='check-config'){assert.equal(process.argv.length,5);console.log(JSON.stringify(validateConfigClosure(process.argv[3],process.argv[4])));}
 else{assert.equal(process.argv[2],'check');assert(process.argv.length>=4);console.log(JSON.stringify(validateFixtureClosure(process.argv[3],process.argv.slice(4))));}
}catch(e){console.error('FIXTURE_CLOSURE_REFUSED '+(e as Error).message);process.exitCode=1;}}
