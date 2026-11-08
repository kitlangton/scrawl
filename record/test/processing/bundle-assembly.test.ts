import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { assembleCanonicalBundle, type BundleAssemblyInput, type BundleValidators } from "@apparent/record/processing/bundle";

const bytes=(s:string)=>new TextEncoder().encode(s);
const sha=(b:Uint8Array)=>createHash("sha256").update(b).digest("hex");
const key={captureId:"capture-a",materialsId:"materials-a"};
const materials:any={manifest:{schema:"apparent.capture-materials/1",key:{...key,revision:"a".repeat(64)},root:"/private/materials",leaseId:"lease-a",payloadFile:{}},payload:{schema:"apparent.capture-materials-payload/1",identity:key,bundleId:"bundle-a",root:"/private/materials",leaseId:"lease-a",clock:{epoch:"capture-admission-monotonic",units:"integer-milliseconds",durationMs:4000,videoOffsetMs:125},confirmedSource:{},invocation:{},files:[],video:{file:"capture.mp4",width:800,height:600,durationMs:4000},facts:{},collection:{context:"collected",interactions:"collected",candidates:"unavailable",transcript:"collected",acceptedStrokes:"unavailable"},warnings:[],safety:{nativeRemoteDelivery:"forbidden",safeForProcessing:true}}};
const docs:any={
 context:{schema:"apparent.capture-context/1",identity:key,context:{state:"collected",value:{values:{workspace:"demo"},changes:[{timeMs:600,key:"mode",value:"edit"}]}},session:{state:"collected",value:{id:"session-a"}},windows:{state:"collected",value:[{timeMs:700,entries:[{id:2,app:"Editor",title:"PRIVATE WINDOW",bounds:[0,0,400,300],layer:0,frontmost:true}]}]},cursor:{state:"collected",value:[[900,0.25,0.5]]}},
 interactions:{schema:"apparent.capture-interactions/1",identity:key,events:[{kind:"app_change",t:800,app:"Editor",window_title:"PRIVATE WINDOW"},{kind:"drag",t:1000,t1:1100,x0:.1,y0:.2,x1:.3,y1:.4,path:[[.2,.3]]}]},
 transcript:{schema:"apparent.capture-transcript/1",identity:key,provider:"fixture",model:"fixture",text:"hello",segments:[{startMs:200,text:"hello"}]},
};
const fact=(name:string,kind:string)=>({file:{relativePath:`facts/${name}.json`,identity:{device:"1",inode:"2",size:1,modifiedNs:"3"},sha256:"b".repeat(64)},bytes:bytes("x"),kind});
const image=bytes("synthetic-png-fixture-bytes");
const sealed=(role:string,path:string,content:Uint8Array)=>({relativePath:path,bytes:content,sha256:sha(content),byteLength:content.byteLength,identity:{device:"1",inode:String(role),size:content.byteLength,modifiedNs:"3"},role});
const artifacts:any=[sealed("video","capture.mp4",bytes("video fixture")),sealed("frame","frames/frame-01.png",image),sealed("contact-sheet","contact-sheet.png",image)];
for(const a of artifacts) a.materials=structuredClone(materials.manifest.key);
materials.payload.files=artifacts.filter((a:any)=>a.role==="video").map((a:any)=>({relativePath:a.relativePath,sha256:a.sha256,identity:a.identity}));
const base:BundleAssemblyInput={manifest:{},payloadBytes:bytes("payload"),facts:{context:fact("context","context"),interactions:fact("interactions","interactions"),transcript:fact("transcript","transcript")},admission:{...key,createdAtMs:1234,bundleId:"bundle-a",bundlePath:"/captures/demo.apparent",target:{mode:"display",displayId:1,frame:{x:0,y:0,width:800,height:600,scale:1},windowTitle:"PRIVATE WINDOW",app:"Editor"},destination:{session:"session-a",title:"",via:"sdk"}},frames:[{index:1,timeMs:1000,absolutePath:"/captures/demo.apparent/frames/frame-01.png",artifactPath:"frames/frame-01.png",canonicalReason:"scene_change",event:"changed"}],marks:[],artifacts,transcriptBounds:{policy:"estimated",basis:"duration to next segment or capture end"}};
const validators:BundleValidators={materials:()=>structuredClone(materials),fact:(f,_b,_m,k)=>{assert.equal(f.relativePath,`facts/${k === "acceptedStrokes" ? "strokes" : k}.json`);return structuredClone(docs[k]);}};

test("assembles deterministic v1 bytes, preserves context/variants, and inventories sealed bytes",()=>{
 const original=structuredClone(base), one=assembleCanonicalBundle(base,validators), two=assembleCanonicalBundle(base,validators);
 assert.deepEqual(one.apparentJson,two.apparentJson);assert.deepEqual(one.summaryMd,two.summaryMd);assert.deepEqual(base,original);
 const doc=JSON.parse(new TextDecoder().decode(one.apparentJson));
 assert.deepEqual(doc,{version:1,id:"bundle-a",createdAtMs:1234,durationMs:4000,target:{mode:"display",displayId:1,frame:{x:0,y:0,width:800,height:600,scale:1},windowTitle:"PRIVATE WINDOW",app:"Editor"},transcript:{source:"fixture",model:"fixture",text:"hello",segments:[{t0:200,t1:4000,text:"hello"}],estimated:true},strokes:[],marks:[],events:docs.interactions.events,keyframes:[{index:1,t:1000,reason:"scene_change",file:"frames/frame-01.png",event:"changed"}],contactSheet:"contact-sheet.png",video:"capture.mp4",videoOffsetMs:125,context:{workspace:"demo"},contextChanges:[{t:600,key:"mode",value:"edit"}],destination:{session:"session-a",title:"",via:"sdk"},windows:[{t:700,windows:[{id:2,app:"Editor",title:"PRIVATE WINDOW",bounds:[0,0,400,300],layer:0,frontmost:true}]}],cursor:[[900,.25,.5]],recordOutput:{schema:"apparent.record-output/1",transcriptStatus:{state:"collected",content:"nonempty",segmentCount:1},warnings:[],acceptedStrokeDocument:null}});
 assert.equal(doc.createdAtMs,1234);assert.equal(doc.videoOffsetMs,125);assert.equal(doc.keyframes[0].index,1);
 assert.equal(doc.events[0].kind,"app_change");assert.equal(doc.events[1].kind,"drag");assert.equal(doc.windows[0].windows[0].title,"PRIVATE WINDOW");
 assert.match(new TextDecoder().decode(one.summaryMd),/\/captures\/demo\.apparent/);assert.doesNotMatch(new TextDecoder().decode(one.summaryMd),/PRIVATE WINDOW/);
 assert.equal(new TextDecoder().decode(one.summaryMd),[
  "[apparent 0:04 · Editor · /captures/demo.apparent]",
  "For session: session-a (session-a)",
  "Visuals are saved in this bundle, but are optional for this text-first handoff. Use apparent zoom if the words need screen context; contact-sheet.png and frames/ are also available there.",
  "workspace: demo",
  "Transcript collected; 1 segment.",
  "  · 0:00 “hello”",
  "  · 0:00 mode → edit",
  "- #1 (0:01) changed",
  "  /captures/demo.apparent/frames/frame-01.png",
  "Video: /captures/demo.apparent/capture.mp4",
  "Accepted strokes: unavailable.",
  "Contact sheet: /captures/demo.apparent/contact-sheet.png",
  "Bundle: /captures/demo.apparent",
  "",
 ].join("\n"));
 assert.equal(one.inventory.collection.candidates,"unavailable");
 for(const a of artifacts){const entry=one.inventory.entries.find(e=>e.path===a.relativePath)!;assert.equal(entry.sha256,sha(a.bytes));assert.equal(entry.byteLength,a.bytes.byteLength);}
});
test("refuses stale identities, absent frame/sheet, escaping references and unestimated transcript bounds",()=>{
 assert.throws(()=>assembleCanonicalBundle({...base,admission:{...base.admission,captureId:"foreign"}},validators),/identity/);
 assert.throws(()=>assembleCanonicalBundle({...base,frames:[{...base.frames[0]!,artifactPath:"../escape.png"}]},validators),/escaping/);
 assert.throws(()=>assembleCanonicalBundle({...base,artifacts:artifacts.filter((a:any)=>a.role!=="contact-sheet")},validators),/contact sheet/);
 assert.throws(()=>assembleCanonicalBundle({...base,transcriptBounds:undefined},validators),/endMs absent/);
});
test("refuses malformed transcript bounds and accepted-stroke canonical mappings",()=>{
 const malformedTranscript={...docs,transcript:{...docs.transcript,segments:[{startMs:5000,endMs:400,text:"bad"}]}};
 const transcriptValidator:BundleValidators={...validators,fact:(f,b,m,k)=>k==="transcript"?malformedTranscript.transcript:validators.fact!(f,b,m,k)};
 assert.throws(()=>assembleCanonicalBundle(base,transcriptValidator),/transcript segment end is invalid/);
 const strokeFacts={schema:"apparent.capture-strokes/1",identity:key,coordinates:"target-normalized-10000",surfaces:[],strokes:[{id:"stroke-a",surface:"screen",surfaceId:"surface-a",transformId:"transform-a",startMs:100,endMs:200,acceptedAtMs:250,points:[{timeMs:100,x:100,y:100}]}]};
 const strokeMaterials=structuredClone(materials);strokeMaterials.payload.collection.acceptedStrokes="collected";
 strokeMaterials.payload.facts.acceptedStrokes={file:"facts/strokes.json",schema:"apparent.capture-strokes/1"};
 const strokeValidator:BundleValidators={materials:()=>strokeMaterials,fact:(f,b,m,k)=>k==="acceptedStrokes"?strokeFacts:validators.fact!(f,b,m,k)};
 const strokeInput={...base,facts:{...base.facts,acceptedStrokes:fact("strokes","acceptedStrokes")},additionalCanonicalFacts:{strokes:[{id:"stroke-a",t0:100,t1:200,fw:800,fh:600,pts:[10001,100,0]}]}};
 assert.throws(()=>assembleCanonicalBundle(strokeInput,strokeValidator),/malformed canonical geometry mapping/);
});

test("artifact scopes, generated names and inventory provenance stay explicit",()=>{
 const foreign=structuredClone(base);foreign.artifacts[1]!.materials.materialsId="foreign";
 assert.throws(()=>assembleCanonicalBundle(foreign,validators),/materials binding/);
 const colliding=structuredClone(base);colliding.artifacts[1]!.relativePath="summary.md";
 assert.throws(()=>assembleCanonicalBundle(colliding,validators),/collides with generated/);
 const result=assembleCanonicalBundle(base,validators);
 assert.equal(result.inventory.entries.find(e=>e.role==="manifest")!.provenance,"record-generated");
 assert.equal(result.inventory.entries.find(e=>e.role==="manifest")!.identity,undefined);
 assert.equal(result.inventory.entries.find(e=>e.role==="video")!.provenance,"material-reference");
 assert.equal(result.inventory.entries.find(e=>e.role==="frame")!.provenance,"caller-attested");
 result.inventory.entries.find(e=>e.role==="frame")!.identity!.inode="mutated";
 assert.notEqual(base.artifacts[1]!.identity.inode,"mutated");
});

test("multiple accepted strokes preserve their actual surface geometry without source IDs in v1",()=>{
 const surface={surfaceId:"surface-a",transformId:"transform-a",width:800,height:600,target:"capture-rectangle"};
 const strokes=["stroke-a","stroke-b"].map(id=>({id,surface:"screen",surfaceId:surface.surfaceId,transformId:surface.transformId,startMs:100,endMs:200,acceptedAtMs:250,points:[{timeMs:100,x:100,y:100},{timeMs:200,x:300,y:200}]}));
 const strokeMaterials=structuredClone(materials);strokeMaterials.payload.collection.acceptedStrokes="collected";
 const strokeValidator:BundleValidators={materials:()=>strokeMaterials,fact:(f,b,m,k)=>k==="acceptedStrokes"?{identity:key,surfaces:[surface],strokes}:validators.fact!(f,b,m,k)};
 const input:BundleAssemblyInput={...structuredClone(base),facts:{...base.facts,acceptedStrokes:fact("strokes","acceptedStrokes")},additionalCanonicalFacts:{strokes:strokes.map(s=>({id:s.id,t0:100,t1:200,fw:800,fh:600,pts:[100,100,0,300,200,100]}))}};
 const doc=JSON.parse(new TextDecoder().decode(assembleCanonicalBundle(input,strokeValidator).apparentJson));
 assert.equal(doc.strokes.length,2);assert.equal(doc.strokes[0].id,undefined);
 const changed=structuredClone(input);changed.additionalCanonicalFacts!.strokes![0]!.pts[0]=200;
 assert.throws(()=>assembleCanonicalBundle(changed,strokeValidator),/changes the observed surface geometry/);
});

test("refuses materials explicitly unsafe for processing",()=>{
 const unsafe=structuredClone(materials);unsafe.payload.safety.safeForProcessing=false;
 assert.throws(()=>assembleCanonicalBundle(base,{...validators,materials:()=>unsafe}),/not declared safe/);
});
