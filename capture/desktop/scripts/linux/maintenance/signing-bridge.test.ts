import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { LEGACY, sourceOfBridge, legacyRevisions, checkAssets } from './signing-bridge.ts';
const source = { sha: 'c'.repeat(40), tree: { sha: 'd'.repeat(40) } };
const bridge = { sha: 'b'.repeat(40), tree: source.tree, parents: [{ sha: LEGACY }, { sha: source.sha }] };
test('bridge authenticates exact old/source ancestry and identical tree', () => {
  assert.equal(sourceOfBridge(bridge, source), source.sha);
  for (const change of [
    { tree: { sha: 'e'.repeat(40) } },
    { parents: [...bridge.parents].reverse() },
    { parents: [bridge.parents[0]] },
    { parents: [...bridge.parents, { sha: 'e'.repeat(40) }] },
    { sha: 'bad' },
  ]) assert.throws(() => sourceOfBridge({ ...bridge, ...change }, source));
});
test('late legacy users never see v2 releases, even when published later', () => {
  const releases = [
    { tag_name: 'linux-main-v2-' + source.sha, draft: false, prerelease: true },
    { tag_name: 'linux-main-' + bridge.sha, draft: false, prerelease: true },
    { tag_name: 'linux-main-' + 'e'.repeat(40), draft: true, prerelease: true },
  ];
  assert.deepEqual(legacyRevisions(releases), [bridge.sha]);
  // This exact tag predicate is frozen in the already-installed v1 updater.
  const visible = releases.filter(r => r.tag_name.startsWith('linux-main-') &&
    /^[0-9a-f]{40}$/.test(r.tag_name.slice('linux-main-'.length)) && !r.draft);
  assert.equal(visible[0].tag_name, 'linux-main-' + bridge.sha);
});
test('alias assets must be byte-identical to both verified release images', () => {
  const directory = mkdtempSync(join(tmpdir(), 'bridge-test-'));
  try {
    const assets: any[] = [];
    const hash = (bytes: string) => createHash('sha256').update(bytes).digest('hex');
    for (const arch of ['aarch64', 'x86_64']) {
      const name = `Apparent-0.3.13-linux-${arch}.AppImage`;
      for (const [file, text] of [[name, arch], [name + '.sha256', `${hash(arch)}  ${name}\n`]]) {
        writeFileSync(join(directory, file), text);
        assets.push({ name: file, state: 'uploaded', digest: 'sha256:' + hash(text) });
      }
    }
    assert.equal(checkAssets(directory, assets).length, 4);
    assert.throws(() => checkAssets(directory, assets.slice(1)));
    writeFileSync(join(directory, assets[0].name), 'corrupted');
    assert.throws(() => checkAssets(directory, assets));
  } finally { rmSync(directory, { recursive: true }); }
});

test('publication CLI preserves admission, private drafts and immutable public assets', async () => {
  const { execFileSync } = await import('node:child_process');
  const { chmodSync, mkdirSync, existsSync } = await import('node:fs');
  const script = new URL('./signing-bridge.ts', import.meta.url).pathname;
  for (const fault of ['', 'bad-digest', 'create-lost-reply', 'edit-lost-reply', 'wrong-tree']) {
    const root = mkdtempSync(join(tmpdir(), 'bridge-cli-'));
    try {
      const bin = join(root, 'bin'); mkdirSync(bin);
      const dist = join(root, 'dist'); mkdirSync(dist);
      const assets: any[] = [];
      for (const arch of ['aarch64', 'x86_64']) {
        const name = `Apparent-0.3.13-linux-${arch}.AppImage`;
        const hash = createHash('sha256').update(arch).digest('hex');
        for (const [file, data] of [[name, arch], [name + '.sha256', `${hash}  ${name}\n`]]) {
          writeFileSync(join(dist, file), data);
          assets.push({ name: file, state: 'uploaded', digest: 'sha256:' + createHash('sha256').update(data).digest('hex') });
        }
      }
      writeFileSync(join(root, 'assets'), JSON.stringify(assets));
      // Generated fake executables stay in this owned temporary fixture only.
      writeFileSync(join(bin, 'gh'), `#!/usr/bin/env node
const fs=require('node:fs'),path=require('node:path');
const root=process.env.BRIDGE_TEST_ROOT,args=process.argv.slice(2),fault=process.env.BRIDGE_TEST_FAULT;
fs.appendFileSync(path.join(root,'calls'),JSON.stringify(args)+'\\n');
const source='c'.repeat(40),bridge='b'.repeat(40),legacy='${LEGACY}',tag='linux-main-'+bridge;
const assets=JSON.parse(fs.readFileSync(path.join(root,'assets')));
const state=()=>fs.existsSync(path.join(root,'state'))?fs.readFileSync(path.join(root,'state'),'utf8'):null;
const release=()=>({tag_name:tag,draft:state()==='draft',prerelease:true,published_at:state()==='public'?'2026-11-03':null,assets,target_commitish:bridge});
const output=v=>console.log(JSON.stringify(v));
if(args[0]==='api'){
 const p=args[1];
 if(p.endsWith('/git/commits/'+bridge)) output({sha:bridge,parents:[{sha:legacy},{sha:source}],tree:{sha:(fault==='wrong-tree'?'e':'d').repeat(40)},author:{email:'32648751+simonvanlaak@users.noreply.github.com'},verification:{verified:true}});
 else if(p.endsWith('/git/commits/'+source)) output({sha:source,tree:{sha:'d'.repeat(40)}});
 else if(p.includes('/releases/tags/linux-main-v2-')) output({draft:false,prerelease:true,published_at:'2026-11-03',target_commitish:source,assets:fault==='bad-digest'?assets.map(a=>({...a,digest:'sha256:'+'0'.repeat(64)})):assets});
 else if(p.includes('/releases/tags/')) output(release());
 else if(p.includes('/releases?')) output([state()?[release()]:[]]);
 else if(p.includes('/commits/linux-main-')) output({sha:bridge});
 else throw Error('unexpected API '+p);
}else if(args[0]==='release'){
 if(args[1]==='download') for(const name of fs.readdirSync(path.join(root,'dist')))fs.copyFileSync(path.join(root,'dist',name),path.join(args[args.indexOf('--dir')+1],name));
 else if(args[1]==='create'){fs.writeFileSync(path.join(root,'state'),'draft');if(fault==='create-lost-reply')process.exit(1);}
 else if(args[1]==='edit'){fs.writeFileSync(path.join(root,'state'),'public');if(fault==='edit-lost-reply')process.exit(1);}
 else if(args[1]==='delete'){if(state()!=='draft')throw Error('must not retract public');fs.unlinkSync(path.join(root,'state'));}
 else throw Error('unexpected release op');
}else throw Error('unexpected gh call');
`);
      writeFileSync(join(bin, 'python3'), '#!/bin/sh\nprintf "%s\\n" "$*" >> "$BRIDGE_TEST_ROOT/admission"\n');
      for (const name of ['gh', 'python3']) chmodSync(join(bin, name), 0o700);
      let failed = false;
      try {
        execFileSync(process.execPath, ['--experimental-strip-types', script, 'publish', bridge.sha], {
          env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, BRIDGE_TEST_ROOT: root, BRIDGE_TEST_FAULT: fault,
            GITHUB_ACTIONS: 'true', GITHUB_REF: 'refs/heads/main', GITHUB_WORKFLOW: 'Release (Linux)', GITHUB_REPOSITORY: 'tryapparent/apparent' },
          stdio: 'pipe',
        });
      } catch { failed = true; }
      assert.equal(failed, fault !== '', fault);
      const state = existsSync(join(root, 'state')) ? readFileSync(join(root, 'state'), 'utf8') : null;
      assert.equal(state, ['', 'edit-lost-reply'].includes(fault) ? 'public' : null, fault);
      const calls = readFileSync(join(root, 'calls'), 'utf8');
      if (state === 'public') assert.ok(!calls.includes('"delete"'), fault);
      if (fault !== 'wrong-tree') assert.match(readFileSync(join(root, 'admission'), 'utf8'), /--platform linux --sha c{40}/);
      if (fault === 'bad-digest' || fault === 'wrong-tree') assert.ok(!calls.includes('"create"'), fault);
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});
