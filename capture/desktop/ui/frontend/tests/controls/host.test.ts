import {test} from 'node:test';
import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import vm from 'node:vm';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const filename = fileURLToPath(new URL('../../electron/main.cjs', import.meta.url));
const require = createRequire(import.meta.url);
function host() {
  const input = new EventEmitter();
  const lines = new EventEmitter();
  const written: string[] = [];
  const output = Object.assign(new EventEmitter(), {write: (line: string) => written.push(line)});
  const handlers = new Map<string, (...args: any[]) => any>();
  let window: Window;
  let quits = 0;
  class Window extends EventEmitter {
    visible = false;
    webContents = Object.assign(new EventEmitter(), {
      mainFrame: {url: ''}, setWindowOpenHandler() {}, send() {},
    });
    constructor() { super(); window = this; }
    loadURL(url: string) { this.webContents.mainFrame.url = url; }
    isDestroyed() { return false; }
    show() { this.visible = true; }
    restore() {}
    focus() {}
  }
  const app = Object.assign(new EventEmitter(), {
    setName() {}, setPath() {}, getPath: () => '/synthetic',
    whenReady: () => ({then: (fn: () => void) => fn()}),
    quit: () => quits++, exit: () => quits++,
  });
  const theme = Object.assign(() => {}, {backgroundColor: () => '#faf9f6'});
  vm.runInNewContext(readFileSync(filename, 'utf8'), {
    require: (name: string) => name === 'electron' ? {
      app, BrowserWindow: Window, ipcMain: {handle: (id: string, fn: (...args: any[]) => any) => handlers.set(id, fn)},
      Menu: {setApplicationMenu() {}}, nativeTheme: {}, session: {defaultSession: {
        setPermissionRequestHandler() {}, setPermissionCheckHandler() {},
      }},
    } : name === '@apparent/ui/window-theme' ? theme : name === 'node:readline' ? {
      createInterface: () => lines,
    } : require(name),
    __dirname: path.dirname(filename), process: {argv: ['electron', '.', '--controls'], stdin: input, stdout: output},
  });
  const event = {sender: window!.webContents, senderFrame: window!.webContents.mainFrame};
  return {window: window!, lines, written, input, handlers, event, quits: () => quits};
}

test('controls require both React mount and first paint; early raise stays hidden', () => {
  const h = host();
  h.lines.emit('line', 'show');
  assert.equal(h.window.visible, false);
  h.handlers.get('apparent:controls-mounted')!(h.event);
  assert.equal(h.window.visible, false);
  h.window.emit('ready-to-show');
  assert.equal(h.window.visible, true);
  assert.deepEqual(h.written, ['{"ready":true}\n']);
});

test('controls never show after parent EOF, including a late ready/mount', () => {
  const h = host();
  h.input.emit('end');
  h.window.emit('ready-to-show');
  h.handlers.get('apparent:controls-mounted')!(h.event);
  h.lines.emit('line', 'show');
  assert.equal(h.window.visible, false);
  assert.deepEqual(h.written, []);
  assert.equal(h.quits(), 1);
});

test('only four explicit actions cross the bridge; busy and foreign frames cannot command', () => {
  const h = host();
  const command = h.handlers.get('apparent:controls-command')!;
  for (const action of ['record', 'choose', 'latest', 'settings']) command(h.event, action);
  assert.equal(h.written.length, 4);
  assert.throws(() => command(h.event, 'quit'));
  assert.throws(() => command(h.event, 'record', 'extra'));
  assert.throws(() => command({...h.event, senderFrame: {url: h.event.senderFrame.url}}, 'record'));
  h.lines.emit('line', 'busy');
  command(h.event, 'record');
  assert.equal(h.written.length, 4);
  h.lines.emit('line', 'idle');
  command(h.event, 'record');
  assert.equal(h.written.length, 5);
  assert.throws(() => h.handlers.get('apparent:controls-mounted')!(h.event, 'extra'));
});

test('controls project the parent warning on mount and ignore malformed status', () => {
  const h = host();
  h.lines.emit('line', 'status "Typed notes unavailable · recording continues"');
  h.lines.emit('line', 'busy');
  const state = h.handlers.get('apparent:controls-mounted')!(h.event);
  assert.equal(state.busy, true);
  assert.equal(state.status, 'Typed notes unavailable · recording continues');
  h.lines.emit('line', 'status {"unexpected":true}');
  assert.equal(h.handlers.get('apparent:controls-mounted')!(h.event).status, state.status);
});
