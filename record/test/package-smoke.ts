/** Clean packed consumer proof: no repository siblings are available to the installed package. */
import assert from 'node:assert/strict';
import {mkdtemp, readFile, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
const run=promisify(execFile);
const scratch=await mkdtemp(path.join(tmpdir(),'record-packed-'));
const root=fileURLToPath(new URL('../',import.meta.url));
try {
  await run('npm',['pack','--pack-destination',scratch],{cwd:root});
  await writeFile(path.join(scratch,'package.json'),JSON.stringify({private:true,type:'module'}));
  await run('npm',['install','--ignore-scripts','--no-audit','--no-fund','--package-lock=false',path.join(scratch,'apparent-record-0.1.0.tgz')],{cwd:scratch});
  const installed=path.join(scratch,'node_modules/@apparent/record');
  const manifest=JSON.parse(await readFile(path.join(installed,'package.json'),'utf8'));
  for(const value of Object.values(manifest.dependencies??{})) assert.ok(!String(value).startsWith('file:'));
  const css=await readFile(path.join(installed,'dist/ui/style.css'),'utf8');
  assert.ok(!/@import|@source|shared\/assets/.test(css),'stylesheet must be compiled and self-contained');
  assert.ok((await readFile(path.join(installed,'dist/ui/HostGrotesk-Variable.ttf'))).length>1000);
  const {stdout}=await run(process.execPath,['--input-type=module','-e',`
    import assert from 'node:assert/strict';
    import React from 'react';
    import {renderToString} from 'react-dom/server';
    import {RecordWorkflow} from '@apparent/record';
    import {hasConfirmedArtifactOpen} from '@apparent/record/capabilities';
    import {createRecordFixture} from '@apparent/record/fixtures';
    import {RecordingPill} from '@apparent/record/ui';
    import {SqliteCaptureMappingStore, PrivateBundleAllocator} from '@apparent/record/host';
    assert.equal(typeof RecordWorkflow,'function');
    assert.equal(typeof SqliteCaptureMappingStore,'function');
    assert.equal(typeof PrivateBundleAllocator,'function');
    assert.equal(hasConfirmedArtifactOpen(createRecordFixture().driver.current(),'unknown'),false);
    assert.match(renderToString(React.createElement(RecordingPill)),/Connecting/);
    console.log('clean packed root/capabilities/fixtures/ui consumer passed');
  `],{cwd:scratch});
  process.stdout.write(stdout);
} finally { await rm(scratch,{recursive:true,force:true}); }
