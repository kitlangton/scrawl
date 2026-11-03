// Synthetic parent deliberately splits a frame across native window creation.
const {PassThrough}=require('node:stream');
const {app}=require('electron');
const parent=new PassThrough();
Object.defineProperty(process,'stdin',{value:parent});
parent.write(JSON.stringify({type:'place',point:[100,100]})+'\n'+ '{"type":"state","view":');
app.whenReady().then(()=>setTimeout(()=>{
 parent.write(JSON.stringify({revision:1,phase:'recording',clock:'00:12',label:'Recording',warning:null,
  reducedMotion:false,draggable:true,actions:[{id:'finish',label:'Finish',enabled:true,selected:false,primary:true}]})+'}\n');
 parent.write(JSON.stringify({type:'activate'})+'\n');
},100));
require('../../electron/pill/main.cjs');
