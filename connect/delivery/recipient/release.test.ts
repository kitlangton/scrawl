import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync,mkdirSync,writeFileSync,rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { seal,targets } from './seal.ts';
import { prepareTarget,prepareTargets,publishTarget,publishTargets,select } from './release.ts';
function executable(target:string) {
 const bytes=Buffer.alloc(64); const arm=target.startsWith('aarch64');
 if(target.endsWith('linux-gnu')) {bytes.write('7f454c46',0,'hex');bytes[4]=2;bytes[5]=1;bytes.writeUInt16LE(3,16);bytes.writeUInt16LE(arm?183:62,18);}
 else {bytes.writeUInt32LE(0xfeedfacf,0);bytes.writeUInt32LE(arm?0x100000c:0x1000007,4);bytes.writeUInt32LE(2,12);}
 return bytes;
}
test('mixed successful reruns retain original identities and reject invalid discoveries',()=>{
 const root=mkdtempSync(join(tmpdir(),'recipient-release-')); const sha='a'.repeat(40);
 const add=(target:string,attempt:string,source=sha)=>{const dir=join(root,`recipient-${source}-123-${attempt}-${target}`);mkdirSync(dir);writeFileSync(join(dir,'apparent-connect-recipient'),executable(target));seal(dir,target,source,'123',attempt);return dir;};
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
 for(const [i,target] of targets.entries()) {const attempt=i<2?'1':'2';const dir=join(input,`recipient-${sha}-123-${attempt}-${target}`);mkdirSync(dir);writeFileSync(join(dir,'apparent-connect-recipient'),executable(target));seal(dir,target,sha,'123',attempt);}
 const old=join(input,`recipient-${sha}-123-1-${targets[2]}`);mkdirSync(old);writeFileSync(join(old,'apparent-connect-recipient'),executable(targets[2]));seal(old,targets[2],sha,'123','1');
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

test('release rejects an honestly hashed but wrong-architecture component before publication',()=>{
 const root=mkdtempSync(join(tmpdir(),'recipient-architecture-'));const sha='a'.repeat(40);
 try {
 for(const target of targets) {const dir=join(root,`recipient-${sha}-123-1-${target}`);mkdirSync(dir);writeFileSync(join(dir,'apparent-connect-recipient'),executable(target));seal(dir,target,sha,'123','1');}
 const dir=join(root,`recipient-${sha}-123-1-${targets[0]}`);
 writeFileSync(join(dir,'apparent-connect-recipient'),executable(targets[1]));
 rmSync(join(dir,'manifest.json'));seal(dir,targets[0],sha,'123','1');
 const output=join(tmpdir(),`recipient-invalid-${Date.now()}`);
 assert.throws(()=>publish(root,output,sha,'123','1',(()=>{throw Error('must reject before external operations');}) as typeof execFileSync),/architecture/);
 assert.equal(existsSync(output),false);
 }finally{rmSync(root,{recursive:true,force:true});}
});

test('one successful target creates an authenticated immutable asset receipt without the other targets',()=>{
 const root=mkdtempSync(join(tmpdir(),'recipient-target-release-'));const sha='c'.repeat(40);const target=targets[0];
 const input=join(root,'input');const output=join(root,'output');mkdirSync(input);
 writeFileSync(join(input,'apparent-connect-recipient'),executable(target));seal(input,target,sha,'456','2');
 const calls:string[][]=[];
 const runner:typeof execFileSync=((command:string,args:string[],options:object)=>{calls.push([command,...args]);return command==='tar'?execFileSync(command,args,options):Buffer.alloc(0);}) as typeof execFileSync;
 try {
  const prepared=prepareTarget(input,output,sha,'456','2',target,'2',runner);
  assert.equal(calls.filter(call=>call[0]==='gh').length,2);
  assert.equal(prepared.releaseTag,`recipient-${sha}-456-2-publication-2-${target}`);
  assert.equal(prepared.receipt.provenance.architecture,'x86_64');
  const published=publishTarget(output,sha,'456','2',target,runner);
  assert.equal(published.releaseTag,prepared.releaseTag);
  const create=calls.find(call=>call[0]==='gh'&&call[1]==='release');
  assert.ok(create);
  assert.deepEqual(create.slice(2,5),['create',prepared.releaseTag,prepared.archive]);
  assert.ok(create.some(value=>value===prepared.receiptPath));
  assert.ok(calls.filter(call=>call[0]==='gh'&&call[1]==='attestation').length===4);
  const receipt=JSON.parse(readFileSync(prepared.receiptPath,'utf8'));
  assert.equal(receipt.provenance.run,'456');assert.equal(receipt.provenance.attempt,'2');
  assert.equal(receipt.provenance.sourceSha,sha);assert.equal(receipt.provenance.target,target);
  assert.match(receipt.archiveSha256,/^[a-f0-9]{64}$/);
  assert.throws(()=>publishTarget(output,sha,'456','1',target,runner),/ENOENT/);
 } finally {rmSync(root,{recursive:true,force:true});}
});

test('target publication rejects incomplete or tampered output before release creation',()=>{
 const root=mkdtempSync(join(tmpdir(),'recipient-target-reject-'));const sha='d'.repeat(40);const target=targets[1];
 const input=join(root,'input');const output=join(root,'output');mkdirSync(input);
 writeFileSync(join(input,'apparent-connect-recipient'),executable(target));seal(input,target,sha,'789','1');
 const calls:string[][]=[];const runner:typeof execFileSync=((command:string,args:string[],options:object)=>{calls.push([command,...args]);return command==='tar'?execFileSync(command,args,options):Buffer.alloc(0);}) as typeof execFileSync;
 try {
  writeFileSync(join(input,'unexpected'),'no');
  assert.throws(()=>prepareTarget(input,output,sha,'789','1',target,'1',runner),/Incomplete or unexpected/);
  rmSync(join(input,'unexpected'));
  const rejected:typeof execFileSync=((command:string,args:string[],options:object)=>{if(command==='gh')throw Error('attestation rejected');return execFileSync(command,args,options);}) as typeof execFileSync;
  assert.throws(()=>prepareTarget(input,output,sha,'789','1',target,'1',rejected),/attestation rejected/);
  assert.equal(existsSync(output),false);
  prepareTarget(input,output,sha,'789','1',target,'1',runner);
  const receiptPath=join(output,`recipient-${sha}-789-1-publication-1-${target}-receipt.json`);
  const receipt=JSON.parse(readFileSync(receiptPath,'utf8'));receipt.provenance.target=targets[0];writeFileSync(receiptPath,JSON.stringify(receipt));
  calls.length=0;
  assert.throws(()=>publishTarget(output,sha,'789','1',target,runner),/Incompatible target receipt target/);
  assert.equal(calls.length,0);
 } finally {rmSync(root,{recursive:true,force:true});}
});

test('budget-gated publisher prepares and ships available targets while keeping source and publication attempts distinct',()=>{
 const root=mkdtempSync(join(tmpdir(),'recipient-targets-release-'));const sha='e'.repeat(40);const input=join(root,'input');const output=join(root,'output');mkdirSync(input);
 const add=(target:string,attempt:string)=>{const directory=join(input,`recipient-${sha}-321-${attempt}-${target}`);mkdirSync(directory);writeFileSync(join(directory,'apparent-connect-recipient'),executable(target));seal(directory,target,sha,'321',attempt);};
 add(targets[0],'1');add(targets[0],'2');add(targets[1],'1');add(targets[3],'2');
 const calls:string[][]=[];const runner:typeof execFileSync=((command:string,args:string[],options:object)=>{calls.push([command,...args]);return command==='tar'?execFileSync(command,args,options):Buffer.alloc(0);}) as typeof execFileSync;
 try {
  const prepared=prepareTargets(input,output,sha,'321','3',runner);
  assert.equal(prepared.length,3);assert.equal(prepared.find(item=>item.receipt.provenance.target===targets[0])?.receipt.provenance.attempt,'2');
  assert.ok(prepared.every(item=>item.receipt.provenance.publicationAttempt==='3'));
  calls.length=0;
  const published=publishTargets(output,sha,'321','3',runner);
  assert.equal(published.length,3);
  assert.equal(calls.filter(call=>call[0]==='gh'&&call[1]==='release'&&call[2]==='create').length,3);
  assert.ok(published.every(item=>item.releaseTag.includes('-publication-3-')));
  assert.throws(()=>select(input,sha,'321','3'),/Missing or ambiguous target/);
 } finally {rmSync(root,{recursive:true,force:true});}
});
