// Playwright starts Electron with ignored stdin. Keep a synthetic parent stream
// for renderer/IPC checks; this fixture is never included in packaged assets.
const {PassThrough}=require('node:stream');
Object.defineProperty(process,'stdin',{value:new PassThrough()});
require(process.argv.includes('--notes-fixture') ? '../../electron/notes/main.cjs' : '../../electron/pill/main.cjs');
