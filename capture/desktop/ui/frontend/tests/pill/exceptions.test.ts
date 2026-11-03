import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, chmod, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const { _electron: electron, expect } = require('@playwright/test');

test('sandboxed renderer main-world throws and rejections reach the private socket',
  { skip: !process.env.DISPLAY && process.platform === 'linux', timeout: 20000 }, async t => {
  const home = await mkdtemp(path.join(tmpdir(), 'exception-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const received: Record<string, unknown>[] = [];
  const server = net.createServer(socket => {
    socket.on('error', () => {});
    socket.once('data', data => {
      try { received.push(JSON.parse(data.toString())); } catch {}
      socket.end('{"accepted":true}\n');
    });
  });
  const address = path.join(directory, 'capture.sock');
  await new Promise<void>(resolve => server.listen(address, resolve));
  await chmod(address, 0o600);
  t.after(async () => { server.close(); await rm(home, { recursive: true, force: true }); });
  const application = await electron.launch({
    args: [path.resolve('electron/main.cjs'), '--no-sandbox'],
    env: { ...process.env, APPARENT_DESKTOP_REPORTING: '1', HOME: home, XDG_CONFIG_HOME: path.join(home, 'config') },
  });
  t.after(() => application.close());
  const page = await application.firstWindow();
  await page.waitForLoadState('load');
  await page.evaluate(() => {
    setTimeout(() => { throw new Error('PRIVATE_TOKEN_DO_NOT_SEND'); }, 0);
    setTimeout(() => { void Promise.reject('PRIVATE_PATH_DO_NOT_SEND'); }, 0);
  });
  await expect.poll(() => received.filter(r => r.errorReporting === 'exception').length).toBe(2);
  assert.deepEqual(received.filter(r => r.errorReporting === 'exception').map(r => r.kind).sort(),
    ['renderer_exception', 'renderer_rejection']);
  assert.ok(!JSON.stringify(received).includes('PRIVATE_'));
});
