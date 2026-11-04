import { readdirSync, readFileSync, mkdirSync, writeFileSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { targets } from './seal.ts';
import { validate } from './validate.ts';
function selectAvailable(root: string, sha: string, run: string, attempt: string) {
  if (!/^[a-f0-9]{40}$/.test(sha) || !/^[1-9][0-9]*$/.test(run) || !/^[1-9][0-9]*$/.test(attempt)) throw Error('Invalid release identity');
  const candidates = readdirSync(root).map(name => {
    const match = /^recipient-([a-f0-9]{40})-([1-9][0-9]*)-([1-9][0-9]*)-(.+)$/.exec(name);
    if (!match || match[1] !== sha || match[2] !== run || BigInt(match[3]) > BigInt(attempt) || !targets.includes(match[4])) throw Error(`Invalid artifact identity: ${name}`);
    const directory = join(root,name);
    const manifest = JSON.parse(readFileSync(join(directory,'manifest.json'),'utf8'));
    const bytes = readFileSync(join(directory,'apparent-connect-recipient'));
    const expected = validate(bytes, manifest,
      {sourceSha:sha,run,attempt:match[3],target:match[4],sha256:createHash('sha256').update(bytes).digest('hex')});
    return {directory, ...expected};
  });
  const selected = targets.flatMap(target => {
    const matching = candidates.filter(c=>c.target===target).sort((a,b)=>BigInt(a.attempt)>BigInt(b.attempt)?-1:1);
    if (matching[1] && matching[0].attempt===matching[1].attempt) throw Error(`Ambiguous target: ${target}`);
    return matching.length ? [matching[0]] : [];
  });
  if (!selected.length) throw Error('No successful recipient targets to publish');
  return selected;
}
export function select(root: string, sha: string, run: string, attempt: string) {
  const selected=selectAvailable(root,sha,run,attempt);
  for(const target of targets) if(!selected.some(item=>item.target===target)) throw Error(`Missing or ambiguous target: ${target}`);
  return selected;
}
export function publish(root:string, output:string, sha:string, run:string, attempt:string, execute: typeof execFileSync = execFileSync) {
  const selected = select(root,sha,run,attempt);
  // Authenticate every discovered candidate, including superseded attempts.
  for (const name of readdirSync(root)) for (const file of ['apparent-connect-recipient','manifest.json']) execute('gh',['attestation','verify',join(root,name,file),'--repo','tryapparent/apparent','--signer-workflow','tryapparent/apparent/.github/workflows/connect-delivery.yml','--source-ref','refs/heads/main','--source-digest',sha],{stdio:'inherit'});
  mkdirSync(output,{recursive:true});
  const version=`recipient-${sha}-${run}-${attempt}`;
  for (const item of selected) {
    chmodSync(join(item.directory,'apparent-connect-recipient'),0o755);
    execute('tar',['-czf',join(output,`${version}-${item.target}.tar.gz`),'-C',item.directory,'apparent-connect-recipient','manifest.json']);
  }
  writeFileSync(join(output,'recipient-identities.json'),JSON.stringify({schema:1,releaseVersion:version,targets:selected.map(({directory,...identity})=>identity)})+'\n');
}

const architecture: Record<string,string> = {
  'x86_64-unknown-linux-gnu':'x86_64',
  'aarch64-unknown-linux-gnu':'aarch64',
  'aarch64-apple-darwin':'arm64',
  'x86_64-apple-darwin':'x86_64',
};

function assertIdentity(sha:string,run:string,attempt:string,target:string) {
  if (!/^[a-f0-9]{40}$/.test(sha) || !/^[1-9][0-9]*$/.test(run) ||
      !/^[1-9][0-9]*$/.test(attempt) || !targets.includes(target)) throw Error('Invalid target release identity');
}

function verifySource(directory:string,sha:string,execute:typeof execFileSync) {
  for(const file of ['apparent-connect-recipient','manifest.json']) execute('gh',['attestation','verify',join(directory,file),'--repo','tryapparent/apparent','--signer-workflow','tryapparent/apparent/.github/workflows/connect-delivery.yml','--source-ref','refs/heads/main','--source-digest',sha],{stdio:'inherit'});
}

/** Prepare a durable one-target asset from exactly one successful matrix job's output. */
function prepareTargetInternal(input:string,output:string,sha:string,run:string,attempt:string,target:string,
                               publicationAttempt:string,execute:typeof execFileSync,verifySourceAttestation:boolean) {
  assertIdentity(sha,run,attempt,target);
  if(!/^[1-9][0-9]*$/.test(publicationAttempt)) throw Error('Invalid target publication attempt');
  const names=readdirSync(input).sort();
  if (names.join(',')!=='apparent-connect-recipient,manifest.json') throw Error('Incomplete or unexpected target output');
  const executable=join(input,'apparent-connect-recipient');
  const manifest=JSON.parse(readFileSync(join(input,'manifest.json'),'utf8'));
  const bytes=readFileSync(executable);
  const identity=validate(bytes,manifest,{sourceSha:sha,run,attempt,target,sha256:createHash('sha256').update(bytes).digest('hex')});
  // Both build outputs must be authenticated before they are packaged.
  if(verifySourceAttestation) verifySource(input,sha,execute);
  mkdirSync(output,{recursive:true});
  const releaseTag=`recipient-${sha}-${run}-${attempt}-publication-${publicationAttempt}-${target}`;
  const asset=`${releaseTag}.tar.gz`;
  const archive=join(output,asset);
  chmodSync(executable,0o755);
  execute('tar',['-czf',archive,'-C',input,'apparent-connect-recipient','manifest.json']);
  const archiveSha256=createHash('sha256').update(readFileSync(archive)).digest('hex');
  const receipt={schema:1,releaseTag,asset,archiveSha256,provenance:{repository:'tryapparent/apparent',workflow:'.github/workflows/connect-delivery.yml',sourceSha:sha,run,attempt,publicationAttempt,target,architecture:architecture[target],executable:'apparent-connect-recipient',executableSha256:identity.sha256}};
  const receiptPath=join(output,`${releaseTag}-receipt.json`);
  writeFileSync(receiptPath,JSON.stringify(receipt)+'\n',{flag:'wx'});
  return {archive,receiptPath,releaseTag,asset,receipt};
}
export function prepareTarget(input:string,output:string,sha:string,run:string,attempt:string,target:string,
                              publicationAttempt=attempt,execute:typeof execFileSync=execFileSync) {
  return prepareTargetInternal(input,output,sha,run,attempt,target,publicationAttempt,execute,true);
}

/** Prepare every available successful target only after the shared-budget preflight. */
export function prepareTargets(root:string,output:string,sha:string,run:string,attempt:string,
                               execute:typeof execFileSync=execFileSync) {
  const selected=selectAvailable(root,sha,run,attempt);
  for(const directory of readdirSync(root).map(name=>join(root,name))) verifySource(directory,sha,execute);
  return selected.map(item=>prepareTargetInternal(item.directory,output,sha,run,item.attempt,item.target,attempt,execute,false));
}

/** Verify the packaged subjects before creating their immutable per-target release. */
function validatePreparedTarget(output:string,sha:string,run:string,sourceAttempt:string,publicationAttempt:string,target:string,
                                execute:typeof execFileSync) {
  assertIdentity(sha,run,sourceAttempt,target);
  if(!/^[1-9][0-9]*$/.test(publicationAttempt)) throw Error('Invalid target publication attempt');
  const releaseTag=`recipient-${sha}-${run}-${sourceAttempt}-publication-${publicationAttempt}-${target}`;
  const asset=`${releaseTag}.tar.gz`;
  const archive=join(output,asset);
  const receiptPath=join(output,`${releaseTag}-receipt.json`);
  const receipt=JSON.parse(readFileSync(receiptPath,'utf8'));
  const archiveSha256=createHash('sha256').update(readFileSync(archive)).digest('hex');
  const expected={schema:1,releaseTag,asset,archiveSha256,provenance:{repository:'tryapparent/apparent',workflow:'.github/workflows/connect-delivery.yml',sourceSha:sha,run,attempt:sourceAttempt,publicationAttempt,target,architecture:architecture[target],executable:'apparent-connect-recipient'}};
  for (const [key,value] of Object.entries(expected)) {
    if (key==='provenance') {
      for (const [field,expectedValue] of Object.entries(value as Record<string,unknown>))
        if (receipt.provenance?.[field]!==expectedValue) throw Error(`Incompatible target receipt ${field}`);
    } else if (receipt[key]!==value) throw Error(`Incompatible target receipt ${key}`);
  }
  if (!/^[a-f0-9]{64}$/.test(receipt.provenance?.executableSha256 ?? '')) throw Error('Invalid target receipt executable digest');
  const members=(execute('tar',['-tzf',archive],{encoding:'utf8'}) as unknown as string).trim().split('\n').sort();
  if (members.join(',')!=='apparent-connect-recipient,manifest.json') throw Error('Unexpected target archive members');
  const archivedManifest=JSON.parse(execute('tar',['-xOzf',archive,'manifest.json'],{encoding:'utf8'}) as unknown as string);
  const archivedExecutable=execute('tar',['-xOzf',archive,'apparent-connect-recipient'],{maxBuffer:64*1024*1024}) as unknown as Buffer;
  const verified=validate(archivedExecutable,archivedManifest,{sourceSha:sha,run,attempt:sourceAttempt,target,sha256:receipt.provenance.executableSha256});
  if (verified.sha256!==receipt.provenance.executableSha256) throw Error('Target receipt executable digest mismatch');
  for (const file of [archive,receiptPath]) execute('gh',['attestation','verify',file,'--repo','tryapparent/apparent','--signer-workflow','tryapparent/apparent/.github/workflows/connect-delivery.yml','--source-ref','refs/heads/main','--source-digest',sha],{stdio:'inherit'});
  return {releaseTag,asset,receiptPath,archive,archiveSha256,sourceAttempt,target};
}

/** Validate all prepared target assets before the first release mutation, then publish them. */
export function publishTargets(output:string,sha:string,run:string,publicationAttempt:string,
                               execute:typeof execFileSync=execFileSync) {
  if(!/^[1-9][0-9]*$/.test(publicationAttempt)) throw Error('Invalid target publication attempt');
  const files=readdirSync(output).sort();
  const receiptNames=files.filter(name=>name.endsWith('-receipt.json'));
  if(!receiptNames.length || receiptNames.length*2!==files.length || files.some(name=>!name.endsWith('.tar.gz')&&!name.endsWith('-receipt.json')))
    throw Error('Invalid prepared recipient target inventory');
  const prepared=receiptNames.map(name=>{
    const receipt=JSON.parse(readFileSync(join(output,name),'utf8'));
    const target=receipt.provenance?.target;
    if(!targets.includes(target)) throw Error('Invalid prepared target receipt architecture');
    const sourceAttempt=receipt.provenance?.attempt;
    if(typeof sourceAttempt!=='string' || !/^[1-9][0-9]*$/.test(sourceAttempt) || BigInt(sourceAttempt)>BigInt(publicationAttempt))
      throw Error(`Invalid target source attempt: ${target}`);
    return validatePreparedTarget(output,sha,run,sourceAttempt,publicationAttempt,target,execute);
  });
  if(new Set(prepared.map(item=>item.target)).size!==prepared.length) throw Error('Duplicate prepared recipient target');
  for(const item of prepared) execute('gh',['release','create',item.releaseTag,item.archive,item.receiptPath,'--repo','tryapparent/apparent','--target',sha,'--title',item.releaseTag,'--notes',`Standalone recipient protocol 1. Source ${sha}; successful target job run ${run} attempt ${item.sourceAttempt}; publication attempt ${publicationAttempt}; target ${item.target}. Verify the receipt and build provenance before use. This target asset does not satisfy the four-target aggregate activation contract.`,'--latest=false'],{stdio:'inherit'});
  return prepared;
}

export function publishTarget(output:string,sha:string,run:string,attempt:string,target:string,
                              execute:typeof execFileSync=execFileSync) {
  const prepared=validatePreparedTarget(output,sha,run,attempt,attempt,target,execute);
  execute('gh',['release','create',prepared.releaseTag,prepared.archive,prepared.receiptPath,'--repo','tryapparent/apparent','--target',sha,'--title',prepared.releaseTag,'--notes',`Standalone recipient protocol 1. Source ${sha}; successful target job run ${run} attempt ${prepared.sourceAttempt}; publication attempt ${attempt}; target ${target}. Verify the receipt and build provenance before use. This target asset does not satisfy the four-target aggregate activation contract.`,'--latest=false'],{stdio:'inherit'});
  return prepared;
}

if (process.argv[1]?.endsWith('/release.ts')) {
  const [command,...args]=process.argv.slice(2);
  if (command==='prepare-target') prepareTarget(...args as [string,string,string,string,string,string]);
  else if (command==='publish-target') publishTarget(...args as [string,string,string,string,string]);
  else if (command==='prepare-targets') prepareTargets(...args as [string,string,string,string,string]);
  else if (command==='publish-targets') publishTargets(...args as [string,string,string,string]);
  else publish(command!,...args as [string,string,string,string]);
}
