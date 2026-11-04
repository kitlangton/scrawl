import { readdirSync, readFileSync, mkdirSync, writeFileSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { targets } from './seal.ts';
import { validate } from './validate.ts';
export function select(root: string, sha: string, run: string, attempt: string) {
  if (!/^[a-f0-9]{40}$/.test(sha) || !/^[1-9][0-9]*$/.test(run) || !/^[1-9][0-9]*$/.test(attempt)) throw Error('Invalid release identity');
  const candidates = readdirSync(root).map(name => {
    const match = /^recipient-([a-f0-9]{40})-([1-9][0-9]*)-([1-9][0-9]*)-(.+)$/.exec(name);
    if (!match || match[1] !== sha || match[2] !== run || BigInt(match[3]) > BigInt(attempt) || !targets.includes(match[4])) throw Error(`Invalid artifact identity: ${name}`);
    const directory = join(root,name);
    const manifest = JSON.parse(readFileSync(join(directory,'manifest.json'),'utf8'));
    const expected = {schema:1,protocol:1,repository:'tryapparent/apparent',workflow:'.github/workflows/connect-delivery.yml',executable:'apparent-connect-recipient',sourceSha:sha,run,attempt:match[3],target:match[4],version:`recipient-${sha}-${run}-${match[3]}`,sha256:createHash('sha256').update(readFileSync(join(directory,'apparent-connect-recipient'))).digest('hex')};
    if (Object.entries(expected).some(([key,value])=>manifest[key] !== value)) throw Error(`Invalid artifact manifest: ${name}`);
    validate(readFileSync(join(directory,'apparent-connect-recipient')), manifest,
      {sourceSha:sha,run,attempt:match[3],target:match[4],sha256:expected.sha256});
    return {directory, ...expected};
  });
  return targets.map(target => {
    const matching = candidates.filter(c=>c.target===target).sort((a,b)=>BigInt(a.attempt)>BigInt(b.attempt)?-1:1);
    if (!matching.length || (matching[1] && matching[0].attempt===matching[1].attempt)) throw Error(`Missing or ambiguous target: ${target}`);
    return matching[0];
  });
}
export function publish(root:string, output:string, sha:string, run:string, attempt:string, execute: typeof execFileSync = execFileSync) {
  const selected = select(root,sha,run,attempt);
  // Authenticate every discovered candidate, including superseded attempts.
  for (const name of readdirSync(root)) for (const file of ['apparent-connect-recipient','manifest.json']) execute('gh',['attestation','verify',join(root,name,file),'--repo','tryapparent/apparent','--signer-workflow','tryapparent/apparent/.github/workflows/connect-delivery.yml','--source-ref','refs/heads/main','--source-digest',sha],{stdio:'inherit'});
  mkdirSync(output);
  const version=`recipient-${sha}-${run}-${attempt}`;
  for (const item of selected) {
    chmodSync(join(item.directory,'apparent-connect-recipient'),0o755);
    execute('tar',['-czf',join(output,`${version}-${item.target}.tar.gz`),'-C',item.directory,'apparent-connect-recipient','manifest.json']);
  }
  writeFileSync(join(output,'recipient-identities.json'),JSON.stringify({schema:1,releaseVersion:version,targets:selected.map(({directory,...identity})=>identity)})+'\n');
}
if (process.argv[1]?.endsWith('/release.ts')) publish(...process.argv.slice(2) as [string,string,string,string,string]);
