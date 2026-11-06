import assert from 'node:assert/strict';
import { test } from 'node:test';
import { cp, mkdir, mkdtemp, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { assemble, verifyAssembly } from '../scripts/assemble.ts';
import { buildApp, buildShell, root } from '../scripts/build.ts';
import { digest } from '../scripts/artifacts.ts';
import { verifyPackagedApplication } from '../scripts/package.ts';
import { buildSyntheticStream } from './synthetic-stream.ts';

test('CI explicitly installs Electron before provisioning its sandbox helper', async () => {
  const workflow = await readFile(join(root, '../../.github/workflows/stream-platform.yml'), 'utf8');
  const job = workflow.split('\n  stream:\n')[1]?.split(/\n  [a-z-]+:\n/)[0];
  assert(job, 'Independent stream producer job must exist');
  assert(job.includes('uses: ./.github/actions/delivery-component'));
  const component = await readFile(join(root, '../../.github/actions/delivery-component/action.yml'), 'utf8');
  const build = component.indexOf('composition/delivery/cli.ts build');
  const install = component.indexOf('run: npm run prepare:electron');
  const sandbox = component.indexOf('sudo install -o root -g root -m 4755 node_modules/electron/dist/chrome-sandbox');
  const smoke = component.indexOf('xvfb-run -a npm run test:electron');
  assert(build >= 0 && install > build && sandbox > install && smoke > sandbox,
    'Package installation must precede runtime installation, helper provisioning and Electron execution');
  const metadata = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  assert.equal(metadata.scripts['prepare:electron'], 'node node_modules/electron/install.js');
  assert.equal(metadata.scripts.prestart, 'npm run prepare:electron');
  const legacy = await readFile(join(root, '../../.github/workflows/verification-platform.yml'), 'utf8');
  assert(!legacy.includes('\n  app-shell-foundation:'), 'Legacy verification must not run App Shell again');
  assert(!job.includes('needs: select') && !job.includes('workflow_run'),
    'Independent delivery must not wait for legacy verification');
  assert(!workflow.includes('workflows: [Linux verification') && !workflow.includes('workflows: [Mac verification'),
    'Native verification completion must not trigger independent delivery');
});

test('independent stream rebuild and compiler-free assembly preserve unchanged artifacts', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'app-foundation-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const shell = join(directory, 'shell');
  const alpha = join(directory, 'alpha');
  const beta = join(directory, 'beta');
  const sourceAlpha = join(directory, 'source-alpha');
  const sourceBeta = join(directory, 'source-beta');
  await buildShell(shell);
  await buildSyntheticStream(sourceAlpha, alpha, { id: 'alpha', label: 'Alpha',
    description: 'Synthetic Alpha workflow.', setting: { kind: 'checkbox', label: 'Alpha preference' } });
  await buildSyntheticStream(sourceBeta, beta, { id: 'beta', label: 'Beta',
    description: 'Synthetic Beta workflow.', setting: { kind: 'select', label: 'Beta detail',
      initial: 'brief', options: ['brief', 'full'] } });
  const betaBytes = await readFile(join(beta, 'index.js'));
  const betaManifest = await readFile(join(beta, 'manifest.json'));
  const betaTime = (await stat(join(beta, 'index.js'))).mtimeMs;
  const shellBytes = await readFile(join(shell, 'renderer.js'));
  const first = await assemble(shell, [alpha, beta], join(directory, 'first'));

  // Simulate only Alpha's source changing. Beta and shell sources are not built again.
  await buildSyntheticStream(sourceAlpha, alpha, { id: 'alpha', label: 'Alpha',
    description: 'Updated Alpha workflow.', setting: { kind: 'checkbox', label: 'Alpha preference' } });
  assert.notEqual(digest(await readFile(join(first, 'streams/alpha/index.js'))), digest(await readFile(join(alpha, 'index.js'))));
  assert.deepEqual(await readFile(join(beta, 'index.js')), betaBytes);
  assert.deepEqual(await readFile(join(beta, 'manifest.json')), betaManifest);
  assert.equal((await stat(join(beta, 'index.js'))).mtimeMs, betaTime);

  // Remove the updated producer source; invoke assembly with compilers deliberately denied.
  await rm(sourceAlpha, { recursive: true });
  const deny = join(directory, 'deny');
  await mkdir(deny);
  const marker = join(directory, 'compiler-used');
  for (const tool of ['cargo', 'rustc', 'tsc', 'esbuild', 'npm']) {
    await writeFile(join(deny, tool), `#!/bin/sh\nprintf denied > '${marker}'\nexit 99\n`, { mode: 0o755 });
  }
  const second = join(directory, 'second');
  const command = spawnSync(process.execPath, ['--experimental-strip-types', join(root, 'scripts/assemble.ts'), shell, second, alpha, beta],
    { env: { ...process.env, PATH: deny }, encoding: 'utf8' });
  assert.equal(command.status, 0, command.stderr);
  await assert.rejects(stat(marker), { code: 'ENOENT' });
  await verifyAssembly(second);
  assert.deepEqual(await readFile(join(second, 'streams/beta/index.js')), betaBytes);
  assert.deepEqual(await readFile(join(second, 'renderer.js')), shellBytes);
  await assert.rejects(assemble(shell, [alpha, beta], second), /exists/i);
  await verifyAssembly(second); // An occupied destination remains intact.

  // Match Electron Packager's actual Mac distribution layout. Checking the
  // containing directory as if it were the .app would miss assembly.json.
  const distribution = join(directory, 'mac-distribution');
  const application = join(distribution, 'apparent-foundation.app');
  const packagedPayload = join(application, 'Contents/Resources/app');
  await cp(second, packagedPayload, { recursive: true });
  assert.equal(await verifyPackagedApplication(distribution, 'darwin'), application);
  await writeFile(join(packagedPayload, 'streams/alpha/index.js'), 'tampered');
  await assert.rejects(verifyPackagedApplication(distribution, 'darwin'), /checksum/);
});

test('default build can assemble a usable shell without installed streams', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'app-empty-shell-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const artifacts = join(directory, 'artifacts');
  const independentArtifact = join(artifacts, 'owner-stream', 'index.js');
  await mkdir(join(artifacts, 'owner-stream'), { recursive: true });
  await writeFile(independentArtifact, 'independently built stream artifact');
  await buildApp(directory);
  const output = join(directory, 'app');
  await assemble(join(directory, 'shell'), [], output);
  await verifyAssembly(output);
  assert.match(await readFile(join(output, 'registry.js'), 'utf8'), /\[\]/);
  assert.equal(await readFile(independentArtifact, 'utf8'), 'independently built stream artifact');
});

test('assembly rejects incompatible, duplicate, tampered and unsafe inputs before publication', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'app-rejection-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const shell = join(directory, 'shell');
  const alpha = join(directory, 'alpha');
  const output = join(directory, 'output');
  await buildShell(shell);
  await buildSyntheticStream(join(directory, 'source-alpha'), alpha, { id: 'alpha', label: 'Alpha',
    description: 'Synthetic Alpha workflow.', setting: { kind: 'checkbox', label: 'Alpha preference' } });
  await assert.rejects(assemble(shell, [alpha, alpha], output), /Duplicate/);
  const manifest = await readFile(join(alpha, 'manifest.json'), 'utf8');
  await writeFile(join(alpha, 'manifest.json'), JSON.stringify({ ...JSON.parse(manifest), shellApi: 2 }));
  await assert.rejects(assemble(shell, [alpha], output), /Unsupported/);
  await writeFile(join(alpha, 'manifest.json'), manifest);
  const original = await readFile(join(alpha, 'index.js'));
  await writeFile(join(alpha, 'index.js'), 'tampered');
  await assert.rejects(assemble(shell, [alpha], output), /checksum/);
  await rm(join(alpha, 'index.js'));
  const outside = join(directory, 'outside.js');
  await writeFile(outside, original);
  await symlink(outside, join(alpha, 'index.js'));
  await assert.rejects(assemble(shell, [alpha], output), /symlink/);
  await rm(join(alpha, 'index.js'));
  await writeFile(join(alpha, 'index.js'), original);
  await writeFile(join(alpha, 'unexpected.js'), 'extra');
  await assert.rejects(assemble(shell, [alpha], output), /Unexpected/);
  await rm(join(alpha, 'unexpected.js'));
  await assert.rejects(stat(output), { code: 'ENOENT' });
  await assemble(shell, [alpha], output);
  await writeFile(join(output, 'streams/alpha/index.js'), 'modified after assembly');
  await assert.rejects(verifyAssembly(output), /checksum/);
});
