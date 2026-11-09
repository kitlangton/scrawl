import {test} from 'node:test';
import assert from 'node:assert/strict';
import {chromium} from '@playwright/test';
import {spawn} from 'node:child_process';
import {createRequire} from 'node:module';
import {mkdtemp, rm} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {fileURLToPath} from 'node:url';

// Synthetic renderer/pipe proof, separate from a GNOME installed-app verdict.
test('shipped controls mount, dispatch allowed actions and reflect parent busy state', {timeout: 30000}, async t => {
  const frontend = fileURLToPath(new URL('../..', import.meta.url));
  const fixture = await mkdtemp(path.join(os.tmpdir(), 'controls-'));
  const executable = createRequire(import.meta.url)('electron') as string;
  const child = spawn(executable, ['--remote-debugging-port=0', ...(process.platform === 'linux' ? ['--ozone-platform=x11'] : []), frontend, '--controls'], {env: {
    ...process.env, HOME: fixture, XDG_CONFIG_HOME: path.join(fixture, 'config'),
    XDG_DATA_HOME: path.join(fixture, 'data'),
  }});
  let exited = false;
  const exit = new Promise<void>(resolve => child.once('exit', () => { exited = true; resolve(); }));
  t.after(async () => {
    if (!exited) child.kill();
    const timer = setTimeout(() => { if (!exited) child.kill('SIGKILL'); }, 1000);
    await exit;
    clearTimeout(timer);
    await rm(fixture, {recursive: true, force: true});
  });
  const output: string[] = [];
  child.stdout.on('data', data => output.push(data.toString()));
  const endpoint = await new Promise<string>((resolve, reject) => {
    let stderr = '';
    const timer = setTimeout(() => reject(new Error(stderr || 'Missing DevTools endpoint')), 10000);
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`Electron exited ${code}: ${stderr}`)); });
    child.stderr.on('data', data => {
      stderr += data.toString();
      const match = stderr.match(/DevTools listening on (ws:\/\/\S+)/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    });
  });
  const browser = await chromium.connectOverCDP(endpoint);
  t.after(async () => { await browser.close(); });
  const context = browser.contexts()[0];
  const page = context.pages()[0] ?? await context.waitForEvent('page');
  const record = page.getByRole('button', {name: 'Record screen', exact: true});
  await record.waitFor();
  await page.waitForFunction(() => !(document.querySelector('button') as HTMLButtonElement)?.disabled);
  assert.equal(await page.evaluate(() => document.visibilityState), 'visible');
  await record.click();
  const deadline = Date.now() + 5000;
  while (!output.join('').includes('{"action":"record"}')) {
    assert.ok(Date.now() < deadline, 'Parent did not receive the record action');
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  child.stdin.write('busy\n');
  await page.getByRole('status').waitFor();
  assert.equal(await record.isDisabled(), true);
  child.stdin.write('idle\n');
  await page.waitForFunction(() => !(document.querySelector('button') as HTMLButtonElement)?.disabled);
  assert.equal(await record.isEnabled(), true);
});
