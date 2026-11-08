import {test} from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {prepareStorageChange, type PickerDriver} from './mac-picker.ts';
function deferred<T>() {
 let resolve!: (value:T)=>void;
 const promise=new Promise<T>(done=>{resolve=done;});
 return {promise,resolve};
}
function fixture(patch: Partial<PickerDriver> = {}) {
 const calls={change:0,openGo:0,navigate:0,picker:0};
 const driver:PickerDriver={storageIdle:async()=>{},change:async()=>{calls.change++;},picker:async()=>{calls.picker++;return 'owned-picker';},openGo:async()=>{calls.openGo++;},goSheet:async()=> 'owned-go-sheet',navigate:async()=>{calls.navigate++;},...patch};
 return {calls,driver};
}
test('the actual Storage/Retention receipt precedes Change, and Go sheet readiness precedes path entry',async()=>{
 const storage=deferred<void>(),go=deferred<string>(),opened=deferred<void>();
 const {calls,driver}=fixture({storageIdle:()=>storage.promise,goSheet:()=>go.promise});
 driver.openGo=async()=>{calls.openGo++;opened.resolve();};
 const pending=prepareStorageChange(driver,'/synthetic/selected captures');
 await Promise.resolve();assert.equal(calls.change,0);assert.equal(calls.openGo,0);
 storage.resolve();await opened.promise;
 assert.equal(calls.change,1);assert.equal(calls.openGo,1);assert.equal(calls.navigate,0);
 go.resolve('owned-go-sheet');
 assert.deepEqual(await pending,{beforeGo:'owned-picker',goSheet:'owned-go-sheet',beforeChoose:'owned-picker'});
 assert.deepEqual(calls,{change:1,openGo:1,navigate:1,picker:2});
});
test('Storage readiness failure never issues a Change mutation',async()=>{
 const {calls,driver}=fixture({storageIdle:async()=>{throw Error('retention receipt not ready');}});
 await assert.rejects(prepareStorageChange(driver,'/synthetic/path'),/retention receipt/);
 assert.deepEqual(calls,{change:0,openGo:0,navigate:0,picker:0});
});
test('missing or ambiguous owned Go sheet fails without retrying the shortcut or entering data',async()=>{
 const {calls,driver}=fixture({goSheet:async()=>{throw Error('Owned Go to Folder sheet missing or ambiguous');}});
 await assert.rejects(prepareStorageChange(driver,'/synthetic/path'),/missing or ambiguous/);
 assert.deepEqual(calls,{change:1,openGo:1,navigate:0,picker:1});
});
test('an unknown navigation outcome is never replayed',async()=>{
 const {calls,driver}=fixture();
 driver.navigate=async()=>{calls.navigate++;throw Error('navigation outcome unknown');};
 await assert.rejects(prepareStorageChange(driver,'/synthetic/path'),/outcome unknown/);
 assert.deepEqual(calls,{change:1,openGo:1,navigate:1,picker:1});
});
test('path characters remain opaque data for the owned native adapter',async()=>{
 const path='/synthetic/selected captures/"café"';let received='';
 const {driver}=fixture({navigate:async value=>{received=value;}});
 await prepareStorageChange(driver,path);assert.equal(received,path);
});
test('the existing CommonJS driver can load the TypeScript helper using the installed Node runtime',()=>{
 const source="const h=require('./enablement/prerequisites/mac-picker.ts'); if(typeof h.prepareStorageChange!=='function'||typeof h.panelScript!=='string')process.exit(1);";
 execFileSync(process.execPath,['-e',source],{stdio:'pipe'});
});
test('native integration retains its consent, deadlines, persistence assertions and one-shot mutation adapter',()=>{
 const source=readFileSync('capture/desktop/ui/frontend/tests/mac-storage-native.cjs','utf8');
 assert.match(source,/prepareStorageChange\(\{/);
 assert.match(source,/APPARENT_STORAGE_NATIVE_CI/);
 assert.match(source,/timeout: 120000/);assert.match(source,/Date\.now\(\) \+ 95000/);
 assert.match(source,/timeout: budget\(3500\)/);
 for(const contract of ['Saved location readback','Cancel changed config bytes','Cancel replaced config','Cancel rewrote config','Reset location readback'])assert.ok(source.includes(contract));
});
