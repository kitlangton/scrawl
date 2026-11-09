// Playwright starts Electron with ignored stdin. Keep a synthetic parent stream
// for renderer/IPC checks; this fixture is never included in packaged assets.
const {PassThrough}=require('node:stream');
Object.defineProperty(process,'stdin',{value:new PassThrough()});
require('../../electron/pill/main.cjs');
