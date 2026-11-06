import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdir, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { _electron as electron, expect } from '@playwright/test';
import { fixture } from './fixture.ts';
import { buildShell, root } from '../../scripts/build.ts';
import { assemble, verifyAssembly } from '../../scripts/assemble.ts';
import { packageApplication } from '../../scripts/package.ts';
const { stopElectron } = createRequire(import.meta.url)('../../../../enablement/native/owned-processes.ts') as {
  stopElectron(app: Awaited<ReturnType<typeof electron.launch>> | undefined): Promise<void>;
};

test('packaged trusted Record binding receives only the package-local native closure root', {
  skip: process.platform !== 'linux' || process.arch !== 'x64', timeout: 90_000,
}, async t => {
  const f = await fixture();
  let app: Awaited<ReturnType<typeof electron.launch>> | undefined;
  t.after(async () => { await stopElectron(app); await rm(f.work, { recursive: true, force: true }); });
  const binding = join(f.work, 'binding.ts');
  await writeFile(binding, `import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { writeFile } from 'node:fs/promises';
import { createRecordFixture } from '@apparent/record/fixtures';
export async function createRecordHostServices({profilePath,nativeRoot}) {
  const entries = ['capture/apparent-engine-capture-launch','media/apparent-engine-frames','config/apparent-config-host'];
  for (const entry of entries) execFileSync(join(nativeRoot,entry), [], {timeout:3000});
  await writeFile(join(profilePath,'native-launches.json'),JSON.stringify({nativeRoot,entries}));
  return {record:createRecordFixture('idle').client,canClose:async()=>true,close:async()=>{}};
}`);
  await buildShell(f.output, undefined, { module: binding, nativeArtifacts: f.input, resolvePaths: [join(root, 'node_modules')] });
  const assembly = await assemble(f.output, [], join(f.work, 'assembly'));
  const bundle = await packageApplication(assembly, join(f.work, 'packages'));
  const helper = process.env.APPARENT_FOUNDATION_SANDBOX_HELPER;
  if (helper) {
    const info = await stat(helper);
    assert(info.isFile() && info.uid === 0 && info.mode & 0o4000 && !(info.mode & 0o022));
    await rm(join(bundle, 'chrome-sandbox'));
    await symlink(helper, join(bundle, 'chrome-sandbox'));
  }
  const profile = join(f.work, 'profile'); await mkdir(profile);
  await verifyAssembly(join(bundle, 'resources/app'));
  app = await electron.launch({ executablePath: join(bundle, 'apparent-foundation'),
    args: [], env: { ...process.env, APPARENT_FOUNDATION_PROFILE: profile } });
  const window = await app.firstWindow();
  await expect(window.getByText('No streams installed.')).toBeVisible();
  assert.deepEqual(JSON.parse(await readFile(join(profile, 'native-launches.json'), 'utf8')), {
    nativeRoot: join(bundle, 'resources/app/native-record'),
    entries: ['capture/apparent-engine-capture-launch', 'media/apparent-engine-frames', 'config/apparent-config-host'],
  });
  assert.equal(await window.evaluate(() => typeof (globalThis as unknown as { process?: unknown }).process), 'undefined');
  await app.close(); app = undefined;
});
