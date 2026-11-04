import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
export const targets = ['x86_64-unknown-linux-gnu','aarch64-unknown-linux-gnu','aarch64-apple-darwin','x86_64-apple-darwin'];
export function seal(directory: string, target: string, sha: string, run: string, attempt: string) {
  if (!targets.includes(target) || !/^[a-f0-9]{40}$/.test(sha) || !/^[1-9][0-9]*$/.test(run) || !/^[1-9][0-9]*$/.test(attempt)) throw Error('Invalid producer identity');
  const bytes = readFileSync(join(directory, 'apparent-connect-recipient'));
  if (!bytes.length) throw Error('Empty executable');
  const manifest = {schema:1, protocol:1, target, sourceSha:sha, version:`recipient-${sha}-${run}-${attempt}`, repository:'tryapparent/apparent', workflow:'.github/workflows/connect-delivery.yml', run, attempt, executable:'apparent-connect-recipient', sha256:createHash('sha256').update(bytes).digest('hex')};
  writeFileSync(join(directory,'manifest.json'), JSON.stringify(manifest)+'\n', {flag:'wx'});
  return manifest;
}
if (process.argv[1]?.endsWith('/seal.ts')) seal(...process.argv.slice(2) as [string,string,string,string,string]);
