import {lstat, rm} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const root=fileURLToPath(new URL('..',import.meta.url));
const output=new URL('../dist',import.meta.url);
const existing=await lstat(output).catch((error:NodeJS.ErrnoException)=>{
  if(error.code!=='ENOENT') throw error;
});
if(existing && (!existing.isDirectory() || existing.isSymbolicLink())) {
  throw new Error('Refusing unsafe crypto build output');
}
await rm(output,{recursive:true,force:true});
execFileSync(process.execPath,[fileURLToPath(new URL('../node_modules/typescript/bin/tsc',import.meta.url))],
  {cwd:root,stdio:'inherit'});
