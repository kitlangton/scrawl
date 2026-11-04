import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const require=createRequire(import.meta.url);
const {installBuildIdentity}=require('../../electron/runtime.cjs');

test('identity is frozen from backend metadata and restricted to owned packaged main frames',()=>{
  const before=process.env.APPARENT_RUNNING_BUILD_LABEL;
  try {
    for(const supplied of [undefined,'main','1.0 · abc1234\nprivate','0.3.13 · abc1234 +modified']) {
      if(supplied===undefined)delete process.env.APPARENT_RUNNING_BUILD_LABEL;
      else process.env.APPARENT_RUNNING_BUILD_LABEL=supplied;
      const sender:any={};let read:any;
      installBuildIdentity({ipcMain:{handle:(name:string,fn:any)=>{
        assert.equal(name,'apparent:build-identity');read=fn;
      }},BrowserWindow:{fromWebContents:(value:any)=>value===sender?{}:null}});
      process.env.APPARENT_RUNNING_BUILD_LABEL='9.9.9 · fffffff';
      for(const page of ['index.html','settings.html','onboarding.html']) {
        sender.mainFrame={url:pathToFileURL(path.resolve(import.meta.dirname,'../../dist',page)).href+'#notes-badge'};
        const event={sender,senderFrame:sender.mainFrame};
        assert.equal(read(event),supplied==='0.3.13 · abc1234 +modified'?supplied:'build unknown');
        assert.throws(()=>read(event,'argument'));
        assert.throws(()=>read({...event,senderFrame:{...sender.mainFrame}}));
        assert.throws(()=>read({...event,sender:{mainFrame:sender.mainFrame}}));
      }
      for(const url of ['https://example.com/index.html','file:///tmp/index.html']) {
        sender.mainFrame={url};assert.throws(()=>read({sender,senderFrame:sender.mainFrame}));
      }
    }
  } finally {
    if(before===undefined)delete process.env.APPARENT_RUNNING_BUILD_LABEL;
    else process.env.APPARENT_RUNNING_BUILD_LABEL=before;
  }
});
