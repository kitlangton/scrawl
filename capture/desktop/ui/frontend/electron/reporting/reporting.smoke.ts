import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { _electron as electron, chromium, expect } from '@playwright/test';

// Shipped main + emitted TS policy + real same-user socket. No PostHog traffic.
for (const settings of [false, true]) {
test(`installed ${settings ? 'Settings' : 'foundation'} reporting observes renderer failure`, { timeout: 30000 }, async t => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'er-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  let application: Awaited<ReturnType<typeof electron.launch>> | undefined;
  let ownedProcess: ReturnType<typeof spawn> | undefined;
  const sockets = new Set<net.Socket>();
  const requests: Record<string, unknown>[] = [];
  const server = net.createServer(socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    socket.once('data', data => {
    if (data.toString() === 'state\n') return socket.end('{"state":{"phase":"idle"}}\n');
    const request = JSON.parse(data.toString());
    if (request.errorReporting === 'exception') {
      requests.push(request);
      return socket.end('{"accepted":true}\n');
    }
    socket.end('{"error":"feature_disabled"}\n');
    });
  });
  t.after(async () => {
    try {
      if (application && ownedProcess && ownedProcess.exitCode === null && ownedProcess.signalCode === null) {
        let timer: ReturnType<typeof setTimeout> | undefined;
        try {
          await Promise.race([application.close(), new Promise((_, reject) => {
            timer = setTimeout(() => {
              ownedProcess?.kill('SIGKILL');
              reject(Error('Owned Electron cleanup exceeded five seconds'));
            }, 5000);
          })]);
        } catch (error) {
          if (ownedProcess.exitCode === null && ownedProcess.signalCode === null) {
            const exited = once(ownedProcess, 'exit');
            ownedProcess.kill('SIGKILL');
            await exited;
          }
          throw error;
        } finally { clearTimeout(timer); }
      }
    } finally {
      for (const socket of sockets) socket.destroy();
      await new Promise<void>(resolve => server.close(() => resolve()));
      await fs.rm(home, { recursive: true, force: true });
    }
  });
  const socketPath = path.join(directory, 'capture.sock');
  await new Promise<void>(resolve => server.listen(socketPath, resolve));
  await fs.chmod(socketPath, 0o600);
  application = await electron.launch({
    args: [fileURLToPath(new URL('../../', import.meta.url)), ...(settings ? ['--settings-storage'] : [])],
    env: { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, 'config'), APPARENT_DESKTOP_REPORTING: '1' },
  });
  // Playwright may dispose its application channel as soon as Electron exits.
  // This OS handle remains valid for exit assertions and owned cleanup.
  ownedProcess = application.process();
  const page = await application.firstWindow();
  await page.waitForLoadState('load');
  const devtools = await page.context().newCDPSession(page);
  const rendererExit = settings ? once(ownedProcess, 'exit') : undefined;
  // forcefullyCrashRenderer may report "killed", which production intentionally
  // excludes. Page.crash exercises a genuine crash on both macOS and Linux.
  const crash = devtools.send('Page.crash').catch(error => {
    assert.match(error.message, /closed|crash/i);
  });
  if (settings) {
    const [code] = await rendererExit!;
    await crash;
    assert.equal(code, 1, 'Settings preserves native fallback exit status');
    assert.ok(requests.some(request => request.kind === 'renderer_gone'));
    return;
  }
  await expect.poll(() => requests.map(r => r.kind)).toContain('renderer_gone');
  // Keep the owning main process alive; only the renderer crashed above.
  assert.equal(ownedProcess.exitCode, null);
  const exited = once(ownedProcess, 'exit');
  await application.evaluate(() => { setTimeout(() => { throw Error('private-error-canary'); }, 0); });
  const [code] = await exited;
  await crash;
  assert.equal(code, 1);
  assert.equal(requests.filter(r => r.kind === 'main_exception').length, 1);
  for (const request of requests) {
    assert.deepEqual(Object.keys(request).sort(), ['deadline', 'errorReporting', 'kind']);
    assert.equal(request.errorReporting, 'exception');
    assert.ok(Number.isSafeInteger(request.deadline));
  }
  assert.ok(!JSON.stringify(requests).includes('private-error-canary'));
});
}

test('packaged Notes admits its renderer crash before its existing quit', { timeout: 30000 }, async t => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'en-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  const requests: Record<string, unknown>[] = [];
  const server = net.createServer(socket => socket.once('data', data => {
    requests.push(JSON.parse(data.toString()));
    socket.end('{"accepted":true}\n');
  }));
  let child: ReturnType<typeof spawn> | undefined;
  let browser: Awaited<ReturnType<typeof chromium.connectOverCDP>> | undefined;
  t.after(async () => {
    try {
      await browser?.close();
    } finally {
      if (child && child.exitCode === null && child.signalCode === null) {
        const exited = once(child, 'exit');
        child.kill('SIGKILL');
        await exited;
      }
      await new Promise<void>(resolve => server.close(() => resolve()));
      await fs.rm(home, { recursive: true, force: true });
    }
  });
  const address = path.join(directory, 'capture.sock');
  await new Promise<void>(resolve => server.listen(address, resolve));
  await fs.chmod(address, 0o600);
  const root = fileURLToPath(new URL('../../', import.meta.url));
  const installed = path.join(home, 'notes-app');
  const { stagePopupAssets } = await import('../../scripts/popup-entries.mjs');
  await stagePopupAssets(root, installed);
  await fs.stat(path.join(installed, 'dist/assets/reporting.cjs'));
  const { default: executable } = await import('electron');
  // Notes needs a real open parent stdin, which _electron.launch does not provide.
  child = spawn(executable as unknown as string, ['--no-sandbox', '--remote-debugging-port=0',
    path.join(installed, 'electron/notes/main.cjs'), '--capture-frame=0,0,1280,800',
    '--notes-profile=' + path.join(home, 'profile')], {
    stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, 'config'), APPARENT_DESKTOP_REPORTING: '1' },
  });
  const endpoint = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => reject(Error('Notes debugger did not become ready')), 10000);
    child!.stderr!.on('data', chunk => {
      const match = chunk.toString().match(/DevTools listening on (ws:\/\/\S+)/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    });
    child!.once('exit', () => { clearTimeout(timer); reject(Error('Notes exited before debugger readiness')); });
  });
  browser = await chromium.connectOverCDP(endpoint);
  await expect.poll(() => browser!.contexts()[0].pages().length).toBe(2);
  const page = browser.contexts()[0].pages()[0];
  // CDP exposes pages before loadURL resolves. Inject into a running notes
  // renderer, not the separate startup/load-failure fallback path.
  await page.waitForLoadState('load');
  await page.waitForFunction(() => Boolean((window as unknown as { captureNotes?: unknown }).captureNotes));
  const devtools = await page.context().newCDPSession(page);
  const exited = once(child, 'exit');
  const crash = devtools.send('Page.crash').catch(error => {
    assert.match(error.message, /closed|crash/i);
  });
  const [code] = await exited;
  await crash;
  assert.equal(code, 0, 'Notes preserves its existing quit behavior');
  assert.ok(requests.some(request => request.kind === 'renderer_gone'));
  for (const request of requests) assert.deepEqual(Object.keys(request).sort(), ['deadline', 'errorReporting', 'kind']);
});
