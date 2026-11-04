import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync,mkdirSync,writeFileSync,rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { seal,targets } from './seal.ts';
import { select } from './release.ts';
test('mixed successful reruns retain original identities and reject invalid discoveries',()=>{
 const root=mkdtempSync(join(tmpdir(),'recipient-release-')); const sha='a'.repeat(40);
 const add=(target:string,attempt:string,source=sha)=>{const dir=join(root,`recipient-${source}-123-${attempt}-${target}`);mkdirSync(dir);writeFileSync(join(dir,'apparent-connect-recipient'),'binary');seal(dir,target,source,'123',attempt);return dir;};
 try {
 for(const target of targets) add(target,'1'); add(targets[2],'2');
 assert.deepEqual(select(root,sha,'123','2').map(x=>x.attempt),['1','1','2','1']);
 for(const [source,attempt] of [[sha,'3'],['b'.repeat(40),'2']]) {const dir=add(targets[0],attempt,source);assert.throws(()=>select(root,sha,'123','2'));rmSync(dir,{recursive:true});}
 const dir=add(targets[0],'2');
 const original=readFileSync(join(dir,'manifest.json'),'utf8');
 for(const key of ['run','attempt','target','version']) {const manifest=JSON.parse(original);manifest[key]='invalid';writeFileSync(join(dir,'manifest.json'),JSON.stringify(manifest));assert.throws(()=>select(root,sha,'123','2'));}
 writeFileSync(join(dir,'manifest.json'),original);writeFileSync(join(dir,'apparent-connect-recipient'),'tampered');assert.throws(()=>select(root,sha,'123','2'));
 rmSync(dir,{recursive:true});rmSync(join(root,`recipient-${sha}-123-1-${targets[0]}`),{recursive:true});assert.throws(()=>select(root,sha,'123','2'));

 } finally {rmSync(root,{recursive:true,force:true});}
});

import { existsSync,readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { publish } from './release.ts';
test('publisher authenticates every original before packaging mixed-attempt archives',()=>{
 const root=mkdtempSync(join(tmpdir(),'recipient-publish-'));const sha='a'.repeat(40);
 const input=join(root,'input');mkdirSync(input);
 for(const [i,target] of targets.entries()) {const attempt=i<2?'1':'2';const dir=join(input,`recipient-${sha}-123-${attempt}-${target}`);mkdirSync(dir);writeFileSync(join(dir,'apparent-connect-recipient'),'binary');seal(dir,target,sha,'123',attempt);}
 const old=join(input,`recipient-${sha}-123-1-${targets[2]}`);mkdirSync(old);writeFileSync(join(old,'apparent-connect-recipient'),'old');seal(old,targets[2],sha,'123','1');
 const calls:string[][]=[];
 const runner:typeof execFileSync=((command:string,args:string[],options:object)=>{
 if(command==='gh'){calls.push(args);if(fail && args[2]===join(old,'manifest.json'))throw Error('attestation rejected');return Buffer.alloc(0);}
 return execFileSync(command,args,options);
 }) as typeof execFileSync;
 let fail=true;
 try {
 const output=join(root,'output');assert.throws(()=>publish(input,output,sha,'123','2',runner));assert.equal(existsSync(output),false);
 fail=false;calls.length=0;publish(input,output,sha,'123','2',runner);assert.equal(calls.length,10);
 for(const args of calls)assert.deepEqual(args.slice(3),['--repo','tryapparent/apparent','--signer-workflow','tryapparent/apparent/.github/workflows/connect-delivery.yml','--source-ref','refs/heads/main','--source-digest',sha]);
 const index=JSON.parse(readFileSync(join(output,'recipient-identities.json'),'utf8'));assert.equal(index.releaseVersion,`recipient-${sha}-123-2`);
 assert.deepEqual(index.targets.map((x:{attempt:string})=>x.attempt),['1','1','2','2']);
 for(const identity of index.targets){const archive=join(output,`${index.releaseVersion}-${identity.target}.tar.gz`);assert.deepEqual(JSON.parse(execFileSync('tar',['-xOzf',archive,'manifest.json'],{encoding:'utf8'})),identity);}
 } finally {rmSync(root,{recursive:true,force:true});}
});
