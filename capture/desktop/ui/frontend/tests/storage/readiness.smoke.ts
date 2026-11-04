import { test } from 'node:test';
import assert from 'node:assert/strict';
import { _electron } from '@playwright/test';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

// Copy the shipped host/preloads/assets, then remove just its React entry module.
// No replacement main-process or renderer implementation stands in for startup.
test('missing Settings renderer stays hidden and exits for native fallback', { timeout: 25000 }, async t => {
  const frontend = fileURLToPath(new URL('../..', import.meta.url));
  const fixture = await fs.mkdtemp(path.join(os.tmpdir(), 'ap-ready-'));
  let app: Awaited<ReturnType<typeof _electron.launch>> | undefined;
  let exited = false;
  t.after(async () => {
    if (app && !exited) await app.close();
    await fs.rm(fixture, { recursive: true, force: true });
  });
  await fs.cp(path.join(frontend, 'electron'), path.join(fixture, 'electron'), { recursive: true });
  await fs.cp(path.join(frontend, 'dist'), path.join(fixture, 'dist'), { recursive: true });
  await fs.symlink(path.join(frontend, 'node_modules'), path.join(fixture, 'node_modules'), 'dir');
  await fs.writeFile(path.join(fixture, 'package.json'), JSON.stringify({ main: 'electron/main.cjs' }));
  const html = await fs.readFile(path.join(fixture, 'dist/settings.html'), 'utf8');
  const entry = html.match(/<script[^>]*src="([^"]+)"/)?.[1];
  assert.ok(entry);
  const module = path.resolve(fixture, 'dist', entry);
  assert.ok(module.startsWith(path.join(fixture, 'dist') + path.sep));
  await fs.rm(module);
  app = await _electron.launch({ args: [fixture, '--settings-storage'],
    env: { ...process.env, HOME: fixture, XDG_CONFIG_HOME: path.join(fixture, 'config') } });
  const output: string[] = [];
  app.process().stdout?.on('data', data => output.push(data.toString()));
  const exit = new Promise<number | null>(resolve => app!.process().once('exit', code => { exited = true; resolve(code); }));
  const page = await app.firstWindow();
  await page.waitForLoadState('load');
  assert.equal(await page.locator('#root').innerHTML(), '');
  assert.equal(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].isVisible()), false);
  assert.equal(await exit, 1, 'A missing entry must release the parent into native fallback');
  assert.equal(output.join('').includes('"action":"close"'), false, 'Failure must not masquerade as user dismissal');
});
