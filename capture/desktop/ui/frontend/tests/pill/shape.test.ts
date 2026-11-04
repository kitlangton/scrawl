import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import {createRequire} from 'node:module';
import ts from 'typescript';

const require=createRequire(import.meta.url);
const geometry=require('../../electron/pill/geometry.cjs');
const source=fs.readFileSync(new URL('../../electron/pill/main.cjs',import.meta.url),'utf8');
const parsed=ts.createSourceFile('main.cjs',source,ts.ScriptTarget.Latest,true);
let measure:string|undefined;
function visit(node:ts.Node){
  if(ts.isFunctionDeclaration(node)&&node.name?.text==='measure')measure=source.slice(node.getStart(parsed),node.end);
  ts.forEachChild(node,visit);
}
visit(parsed);
assert.ok(measure);

function host(){
  const shapes:unknown[]=[];
  const sizes={width:512,height:58,mainWidth:440,rowHeight:56,revision:1};
  const display={workArea:{x:0,y:0,width:1920,height:1200},scaleFactor:1};
  const bounds={x:700,y:34,width:512,height:58};
  const scope={loaded:true,current:{phase:'recording',noteCount:0,revision:1},lastInput:Date.now(),
    appliedShape:null,footprint:{rowY:0,mainWidth:440},geometry,
    process:{platform:'linux'},app:{exit:()=>assert.fail('measurement exited')},
    screen:{getDisplayMatching:()=>display},
    window:{isDestroyed:()=>false,getBounds:()=>({...bounds}),
      setShape:(shape:unknown)=>shapes.push(shape),
      webContents:{executeJavaScript:async(script:string)=>script.startsWith('(()')?{...sizes}:undefined}},
    position:(next:typeof bounds)=>Object.assign(bounds,next),
  };
  const context=vm.createContext(scope);
  vm.runInContext(measure!,context);
  return {shapes,sizes,display,scope,measure:()=>vm.runInContext('measure()',context)};
}

test('recording heartbeats preserve the existing native drawing region',async()=>{
  const h=host();
  await Promise.all(Array.from({length:8},()=>h.measure()));
  for(let n=0;n<40;n++)await h.measure();
  assert.equal(h.shapes.length,1,'unchanged measurements must not reset the native region');
  assert.deepEqual(JSON.parse(JSON.stringify(h.shapes[0])),geometry.shape(512,58,0,56,440,440));
});

test('native region follows note count, pocket geometry and display scale changes',async()=>{
  const h=host();
  await h.measure();
  h.scope.current.noteCount=1;await h.measure();
  assert.equal(h.shapes.length,2,'extension must become interactive and drawable');
  assert.deepEqual(JSON.parse(JSON.stringify(h.shapes[1])),geometry.shape(512,58,0,56,512,440));
  h.sizes.height=180;await h.measure();
  assert.equal(h.shapes.length,3,'pocket must resize the native region');
  h.display.scaleFactor=1.25;await h.measure();
  assert.equal(h.shapes.length,4,'DIP-to-pixel conversion changes must reapply the shape');
  await h.measure();assert.equal(h.shapes.length,4);
  h.sizes.height=58;h.scope.current.noteCount=0;await h.measure();
  assert.equal(h.shapes.length,5,'closing pocket and extension must restore click-through corners');
});
