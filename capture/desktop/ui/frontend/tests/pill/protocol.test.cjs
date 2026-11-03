const {test}=require('node:test');
const assert=require('node:assert/strict');
const {view,command}=require('../../electron/pill/protocol.cjs');
const state={revision:12,phase:'recording',clock:'00:12',label:'Recording',warning:null,reducedMotion:false,draggable:true,
 actions:[{id:'finish',label:'Finish',enabled:true,selected:false,primary:true}]};
test('pill only dispatches available actions for the displayed revision',()=>{
 assert.deepEqual(command(view(state),12,'finish'),{revision:12,action:'finish'});
 for(const args of [[11,'finish'],[12,'paste'],[12,'__proto__'],[12,'select-window']])assert.throws(()=>command(view(state),...args));
 assert.throws(()=>command(null,12,'finish'));
 assert.throws(()=>command(view({...state,actions:[{...state.actions[0],enabled:false}]}),12,'finish'));
});
test('pill projects state and rejects malformed or unbounded controls',()=>{
 assert.equal(view({...state,bundlePath:'/private',token:'secret'}).bundlePath,undefined);
 for(const change of [{revision:-1},{phase:'unknown'},{clock:'x'.repeat(21)},{warning:{}},{draggable:1},
 {actions:[...state.actions,...state.actions]},{actions:[{...state.actions[0],id:'shell'}]}])assert.throws(()=>view({...state,...change}));
});
const geometry=require('../../electron/pill/geometry.cjs');
test('count is bounded and notes require current action authority',()=>{
 const notes={id:'notes',label:'Open notes',enabled:true,selected:false,primary:false};
 assert.equal(view(state).noteCount,0);
 for(const noteCount of [-1,1.5,101,'1'])assert.throws(()=>view({...state,noteCount}));
 assert.throws(()=>view({...state,phase:'completed',noteCount:1}));
 const current=view({...state,noteCount:12,actions:[...state.actions,notes]});
 assert.deepEqual(command(current,12,'notes'),{revision:12,action:'notes'});
 assert.throws(()=>command(current,11,'notes'));
 assert.throws(()=>command(view(state),12,'notes'));
});
test('transparent count reserve cannot catch clicks at zero and row anchor stays fixed',()=>{
 const area={x:0,y:0,width:1000,height:700};
 const before={x:300,y:30,width:480,height:58};
 const next=geometry.layout(before,area,480,58,58,0,408,408);
 assert.deepEqual(next.bounds,before);
 assert.equal(geometry.contains({x:750,y:59},before,0,58,408,408),false);
 assert.equal(geometry.contains({x:750,y:59},before,0,58,480,408),true);
 const zero=geometry.shape(480,58,0,58,408,408);
 assert.ok(zero.every(rect=>rect.x+rect.width<=408));
});
test('pill input silhouette lets transparent corners through and clamps to negative-origin displays',()=>{
 const bounds={x:-100,y:30,width:600,height:58};
 assert.equal(geometry.contains({x:-100,y:30},bounds),false);
 assert.equal(geometry.contains({x:200,y:30},bounds),true);
 assert.equal(geometry.contains({x:200,y:90},bounds),false);
 const shape=geometry.shape(600,58);
 assert.equal(shape.length,58);assert.ok(shape[0].x>0);assert.equal(shape[28].x,0);
 assert.deepEqual(geometry.clamp({...bounds,x:-2000,y:2000},{x:-1920,y:0,width:1920,height:1080}),{x:-1920,y:1022});
});

test('pocket is bounded, strips private extras, and references authoritative actions',()=>{
 const pocket={tone:'failure',title:'Too large to upload',description:'Saved locally',actionIds:['finish']};
 assert.deepEqual(view({...state,pocket:{...pocket,path:'/private'}}).pocket,pocket);
 assert.equal(view(state).pocket,null);
 for(const change of [{tone:'success'},{title:'x'.repeat(81)},{description:'x'.repeat(301)},
 {actionIds:['paste']},{actionIds:['finish','finish']},{actionIds:['shell']}])assert.throws(()=>view({...state,pocket:{...pocket,...change}}));
 assert.throws(()=>command(view({...state,pocket}),11,'finish'));
});
test('vertical pocket preserves leading-row anchor and compound click-through footprint',()=>{
 const area={x:-1920,y:0,width:1920,height:1080};
 const before={x:-1200,y:950,width:500,height:58};
 const open=geometry.layout(before,area,500,220,58);
 assert.equal(open.side,'up');assert.equal(open.bounds.y+open.rowY,before.y);
 assert.equal(open.bounds.x,before.x);
 const closed=geometry.layout(open.bounds,area,500,58,58,open.rowY);
 assert.deepEqual(closed.bounds,before);
 for(const [rowY,rowHeight] of [[0,58],[162,58]]){
   const bounds={x:0,y:0,width:500,height:220}, shape=geometry.shape(500,220,rowY,rowHeight);
   for(let y=0;y<220;y++)for(const x of [0,10,250,489,499])assert.equal(geometry.contains({x:x+.5,y:y+.5},bounds,rowY,rowHeight),shape.some(r=>y===r.y && x>=r.x && x<r.x+r.width));
 }
 const down=geometry.layout({...before,y:34},area,500,220,58);
 assert.equal(down.side,'down');assert.equal(down.bounds.y,34);
});
