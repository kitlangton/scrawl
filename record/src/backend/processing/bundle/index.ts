import { createHash } from "node:crypto";
import { isAbsolute, resolve } from "node:path";
import { isDeepStrictEqual } from "node:util";
import type { MaterialsDocument, ContextFacts, InteractionFacts, TranscriptFacts, StrokeFacts } from "@apparent/engine-sdk/capture";
import { validateMaterialsDocument, validateFactFile } from "@apparent/engine-sdk/capture";
import { assembleProcessingOutput, type ProcessingInput, type ProcessingWarning, type SelectedFrame } from "../index.js";

/** Public host validation seam. Implementations must delegate to the packed Engine SDK. */
export interface BundleValidators {
  materials?: (manifest: unknown, payloadBytes: Uint8Array) => MaterialsDocument;
  fact?: (file: SealedFile, bytes: Uint8Array, materials: MaterialsDocument, kind: "context" | "interactions" | "transcript" | "acceptedStrokes") => unknown;
}
export interface SealedFile { relativePath:string; identity:{device:string;inode:string;size:number;modifiedNs:string}; sha256:string }
export interface FactBytes { file: SealedFile; bytes: Uint8Array }
/** Images remain caller-attested; matching metadata is not native provenance. */
export interface SealedArtifact { materials: MaterialsDocument["manifest"]["key"]; relativePath: string; bytes: Uint8Array; sha256: string; byteLength: number; identity:{device:string;inode:string;size:number;modifiedNs:string}; role: "frame" | "contact-sheet" | "video" | "audio" }
export interface CanonicalStroke { id:string;t0:number;t1:number;fw:number;fh:number;on?:string;pin?:number;laser?:boolean;rgb?:[number,number,number];pts:number[] }
/** Project only original sealed accepted strokes; no shape or history inference. */
export function projectOriginalAcceptedStrokes(materials: MaterialsDocument, fact?: FactBytes): CanonicalStroke[] | undefined {
  if (materials.payload.collection.acceptedStrokes === "unavailable") {
    if (fact) fail("unavailable accepted strokes cannot accept supplied facts");
    return undefined;
  }
  if (!fact) fail("collected accepted strokes require their original sealed fact");
  const document = validateFactFile(fact.file, fact.bytes, materials, "acceptedStrokes") as StrokeFacts;
  return document.strokes.map(stroke => {
    const surface = document.surfaces.find(surface => surface.surfaceId === stroke.surfaceId && surface.transformId === stroke.transformId);
    if (!surface) fail("accepted stroke surface/transform is unavailable");
    return { id: stroke.id, t0: stroke.startMs, t1: stroke.endMs, fw: surface.width, fh: surface.height,
      pts: stroke.points.flatMap(point => [point.x, point.y, point.timeMs - stroke.startMs]) };
  });
}
export interface CanonicalMark { id:string;strokes:number[];t0:number;t1:number;boxNorm:[number,number,number,number];shape:"circle"|"underline"|"line"|"arrow"|"tap"|"pin"|"mark"|"flag";from?:[number,number];to?:[number,number];pin?:number;on?:string;description:string;frame?:string }
export type CanonicalKeyframeReason="mark"|"click"|"laser"|"scene_change"|"speech"|"initial"|"whiteboard";
export interface RecordAdmission {
  captureId: string; materialsId: string; createdAtMs: number; bundleId: string;
  bundlePath: string;
  /** Canonical TargetInfo supplied by Record's admitted target policy. */
  target: { mode: "display" | "window" | "region" | "device"; app?: string; bundleId?: string; windowTitle?: string; windowId?: number; displayId: number; frame: {x:number;y:number;width:number;height:number;scale:number} };
  /** SDK facts do not contain the legacy canonical source/device metadata. */
  source?: string; device?: Record<string, unknown>; note?: string;
  destination?: {session:string;title:string;via:string;url?:string;directory?:string}; imageQualityWarning?:string;
}
export interface BundleAssemblyInput {
  manifest: unknown; payloadBytes: Uint8Array; facts: Partial<Record<"context"|"interactions"|"transcript"|"acceptedStrokes", FactBytes>>;
  admission: RecordAdmission;
  /** Already selected Record policy output. */
  frames: Array<SelectedFrame & { artifactPath: string; canonicalReason:CanonicalKeyframeReason }>;
  marks: unknown[];
  artifacts: SealedArtifact[];
  /** Media files retained by the host; seals come only from the validated original materials document. */
  materialReferences?: Array<{ role: "video" | "audio"; relativePath: string }>;
  processingWarnings?: ProcessingWarning[];
  /** Full canonical mapping that SDK facts cannot establish (e.g. calibrated legacy stroke transform, screen spans, cursor projection). */
  additionalCanonicalFacts?: { strokes?: CanonicalStroke[]; marks?: CanonicalMark[]; screens?: unknown[]; autocrop?: unknown };
  transcriptBounds?: { policy: "estimated"; basis: string };
}
export interface PublicationInventory { schema: "apparent.record-bundle-inventory/1"; materials:MaterialsDocument["manifest"]["key"]; retainedFacts:{confirmedSource:MaterialsDocument["payload"]["confirmedSource"];invocation:MaterialsDocument["payload"]["invocation"];references:MaterialsDocument["payload"]["facts"]}; collection: MaterialsDocument["payload"]["collection"]; observations: Record<string,"collected"|"unavailable">; warnings: MaterialsDocument["payload"]["warnings"]; entries: Array<{ path:string; role:string; sha256:string; byteLength:number; provenance:"record-generated"|"material-reference"|"caller-attested"; identity?:SealedArtifact["identity"] }> }
export interface BundleAssembly { bundlePath: string; apparentJson: Uint8Array; summaryMd: Uint8Array; processing: ProcessingInput; inventory: PublicationInventory; materials: MaterialsDocument; validatedFacts: {context?:ContextFacts;interactions?:InteractionFacts;transcript?:TranscriptFacts;acceptedStrokes?:StrokeFacts} }

const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const utf8 = (s: string) => new TextEncoder().encode(s);
function fail(message: string): never { throw new TypeError(`bundle assembly unsupported: ${message}`); }
function sameIdentity(value: {captureId:string;materialsId:string}, m: MaterialsDocument) {
  if (value.captureId !== m.payload.identity.captureId || value.materialsId !== m.payload.identity.materialsId) fail("fact identity is stale or foreign to supplied materials");
}
function artifactPath(p: string) { if (!p || isAbsolute(p) || /^[A-Za-z]:/u.test(p) || p.includes("\0") || p.split(/[\\/]/u).some(x=>x===".."||x==="."||x==="")) fail(`escaping or invalid artifact reference: ${p}`); return p.replaceAll("\\", "/"); }

/** Assemble canonical v1 DOCUMENT/INVENTORY bytes from validated inputs, without filesystem effects. */
export function assembleCanonicalBundle(input: BundleAssemblyInput, validators: BundleValidators = {}): BundleAssembly {
  input = structuredClone(input);
  const materialsValidator=validators.materials??((manifest,bytes)=>validateMaterialsDocument(manifest,bytes));
  const factValidator=validators.fact??((file,bytes,materials,kind)=>validateFactFile(file,bytes,materials,kind));
  const m = materialsValidator(input.manifest, input.payloadBytes);
  const originalMaterials = input.materialReferences === undefined ? m : validateMaterialsDocument(input.manifest, input.payloadBytes);
  if (input.materialReferences !== undefined && !isDeepStrictEqual(m, originalMaterials)) fail("material reference source differs from SDK-validated original materials");
  if (!m.payload.safety.safeForProcessing) fail("materials are not declared safe for processing");
  if (input.admission.captureId !== m.payload.identity.captureId || input.admission.materialsId !== m.payload.identity.materialsId) fail("admission identity does not match materials");
  if(input.admission.bundleId!==m.payload.bundleId) fail("admission bundle identity does not match materials");
  if (!isAbsolute(input.admission.bundlePath)||resolve(input.admission.bundlePath)!==input.admission.bundlePath) fail("bundlePath must be absolute and normalized");
  const t=input.admission.target;
  if(!["display","window","region","device"].includes(t.mode)||!Number.isSafeInteger(t.displayId)||t.displayId<0||![t.frame.x,t.frame.y,t.frame.width,t.frame.height,t.frame.scale].every(Number.isFinite)||t.frame.width<=0||t.frame.height<=0||t.frame.scale<=0||(t.windowId!==undefined&&(!Number.isSafeInteger(t.windowId)||t.windowId<0))) fail("admission target cannot be mapped to canonical v1");
  if (!Number.isSafeInteger(input.admission.createdAtMs) || input.admission.createdAtMs < 0) fail("createdAtMs must be explicit persisted admission metadata");
  const contextDoc = input.facts.context && factValidator(input.facts.context.file,input.facts.context.bytes,m,"context") as ContextFacts;
  const interactionDoc = input.facts.interactions && factValidator(input.facts.interactions.file,input.facts.interactions.bytes,m,"interactions") as InteractionFacts;
  const transcriptDoc = input.facts.transcript && factValidator(input.facts.transcript.file,input.facts.transcript.bytes,m,"transcript") as TranscriptFacts;
  const strokesDoc = input.facts.acceptedStrokes && factValidator(input.facts.acceptedStrokes.file,input.facts.acceptedStrokes.bytes,m,"acceptedStrokes") as StrokeFacts;
  for (const doc of [contextDoc,interactionDoc,transcriptDoc,strokesDoc]) if (doc) sameIdentity(doc.identity,m);
  for (const key of ["context","interactions","transcript","acceptedStrokes"] as const) {
    const supplied = input.facts[key]; const state = m.payload.collection[key];
    if (state === "collected" && !supplied) fail(`${key} collection is declared collected but bytes are missing`);
    if (state === "unavailable" && supplied) fail(`${key} bytes supplied while collection is unavailable`);
  }
  if (!input.frames.length) fail("standard profile requires selected sealed frames");
  const byPath = new Map(input.artifacts.map(a=>[artifactPath(a.relativePath),a]));
  if (byPath.size !== input.artifacts.length) fail("duplicate artifact path");
  for(const [path,a] of byPath){if(path==="apparent.json"||path==="summary.md")fail(`artifact collides with generated document: ${path}`);if(!isDeepStrictEqual(a.materials,m.manifest.key))fail(`artifact materials binding is stale or foreign: ${path}`);if(!["frame","contact-sheet","video","audio"].includes(a.role)||!a.identity.device||!a.identity.inode||!/^[0-9]+$/u.test(a.identity.modifiedNs)||!Number.isSafeInteger(a.identity.size)||a.byteLength!==a.bytes.byteLength||a.identity.size!==a.byteLength||hash(a.bytes)!==a.sha256)fail(`sealed artifact hash/length mismatch ${path}`);}
  function matchMaterial(a:SealedArtifact) {
    const listed=m.payload.files.find(f=>f.relativePath===a.relativePath);
    if(!listed||listed.sha256!==a.sha256||!isDeepStrictEqual(listed.identity,a.identity)) fail(`media artifact differs from sealed materials reference: ${a.relativePath}`);
  }
  const materialReferences = new Map<string, { role: "video" | "audio"; file: MaterialsDocument["payload"]["files"][number] }>();
  for (const reference of input.materialReferences ?? []) {
    if (reference.role !== "video" && reference.role !== "audio") fail("material reference has an unsupported media role");
    const path = artifactPath(reference.relativePath);
    if (path !== reference.relativePath || materialReferences.has(path)) fail(`duplicate or non-canonical material reference: ${path}`);
    if (path === "apparent.json" || path === "summary.md") fail(`material reference collides with generated document: ${path}`);
    if (byPath.has(path)) fail(`material reference conflicts with supplied artifact bytes: ${path}`);
    const expectedPath = reference.role === "video" ? originalMaterials.payload.video.file : originalMaterials.payload.audio?.file;
    if (!expectedPath || path !== expectedPath) fail(`material reference is not the declared ${reference.role} file: ${path}`);
    const file = originalMaterials.payload.files.find(item => item.relativePath === path);
    if (!file) fail(`declared ${reference.role} file is missing from validated materials: ${path}`);
    materialReferences.set(path, { role: reference.role, file });
  }
  const videoPath = artifactPath(m.payload.video.file);
  const videoArtifact=byPath.get(videoPath);
  const videoReference = materialReferences.get(videoPath);
  if(videoArtifact){if(videoArtifact.role!=="video")fail("canonical video reference has a mismatched supplied media role");matchMaterial(videoArtifact);}
  if(videoReference && videoReference.role!=="video")fail("canonical video reference has a mismatched material-reference role");
  if(!videoArtifact&&!videoReference)fail("canonical video reference lacks its supplied sealed video artifact or material reference");
  if(m.payload.audio){
    const audioPath=artifactPath(m.payload.audio.file), audioArtifact=byPath.get(audioPath), audioReference=materialReferences.get(audioPath);
    if(audioArtifact){if(audioArtifact.role!=="audio")fail("canonical audio reference has a mismatched supplied media role");matchMaterial(audioArtifact);}
    if(audioReference && audioReference.role!=="audio")fail("canonical audio reference has a mismatched material-reference role");
    if(!audioArtifact&&!audioReference)fail("canonical audio reference lacks its supplied sealed audio artifact or material reference");
  }
  for(const reference of materialReferences.values())if(reference.role==="audio"&&!m.payload.audio)fail("audio material reference is not declared by validated materials");
  for(const artifact of input.artifacts)if((artifact.role==="video"&&artifact!==videoArtifact)||(artifact.role==="audio"&&artifact.relativePath!==m.payload.audio?.file))fail("unreferenced media artifact");
  let previousIndex=0, previousTime=-1;
  const frameFacts = input.frames.map(f=>{
    if (!["mark","click","laser","scene_change","speech","initial","whiteboard"].includes(f.canonicalReason)||!Number.isSafeInteger(f.index)||f.index<1||!Number.isSafeInteger(f.timeMs)||f.timeMs<0||f.timeMs>m.payload.clock.durationMs) fail("invalid selected frame reason/index/time");
    if(f.index<=previousIndex||f.timeMs<previousTime) fail("selected frames require unique increasing positive indices and capture time order");
    previousIndex=f.index; previousTime=f.timeMs;
    const path=artifactPath(f.artifactPath), a=byPath.get(path); if(!a||a.role!=="frame") fail(`missing sealed frame artifact ${path}`);
    if(!isAbsolute(f.absolutePath)||resolve(f.absolutePath)!==resolve(input.admission.bundlePath,path)) fail(`selected frame path is foreign or stale for ${path}`);
    return {...f, file:path};
  });
  const sheets=input.artifacts.filter(a=>a.role==="contact-sheet"); if(sheets.length!==1) fail("standard profile requires exactly one sealed contact sheet");
  const sheet=sheets[0]!;
  const context=contextDoc?.context.state==="collected" ? contextDoc.context.value : undefined;
  const transcript=transcriptDoc ? {source:transcriptDoc.provider,...(transcriptDoc.model!==undefined?{model:transcriptDoc.model}:{}),text:transcriptDoc.text,segments:transcriptDoc.segments.map((s,i)=>{
    if(s.endMs===undefined && !input.transcriptBounds) fail("transcript segment endMs absent; provide explicit estimated Record bounds policy or unsupported");
    if(s.endMs===undefined && input.transcriptBounds?.policy!=="estimated") fail("unsupported transcript bounds policy");
    if(s.endMs===undefined && !input.transcriptBounds?.basis.trim()) fail("estimated transcript bounds require a documented Record policy basis");
    const t1=s.endMs ?? transcriptDoc.segments[i+1]?.startMs ?? m.payload.clock.durationMs; if(t1<s.startMs || t1>m.payload.clock.durationMs) fail("transcript segment end is invalid for capture duration");
    return {t0:s.startMs,t1,text:s.text};
  }),...(transcriptDoc.segments.some(s=>s.endMs===undefined)?{estimated:true}:{})} : {source:"none",text:"",segments:[]};
  const strokes = strokesDoc?.strokes.map(s=>{
    // The v1 consumer contract needs an explicit Record mapping of the accepted surface transform.
    const mapped=input.additionalCanonicalFacts?.strokes?.find((x:any)=>x.id===s.id) as any;
    if(!mapped) fail(`accepted stroke ${s.id} requires a Record-owned canonical v1 surface/transform mapping`);
    if(!Number.isSafeInteger(mapped.t0)||!Number.isSafeInteger(mapped.t1)||mapped.t0!==s.startMs||mapped.t1!==s.endMs||!Number.isSafeInteger(mapped.fw)||mapped.fw<1||!Number.isSafeInteger(mapped.fh)||mapped.fh<1||!Array.isArray(mapped.pts)||mapped.pts.length===0||mapped.pts.length%3!==0||mapped.pts.some((v:number,i:number)=>!Number.isSafeInteger(v)||(i%3===0||i%3===1)&&(v<0||v>10000)||(i%3===2&&(v<0||v>mapped.t1-mapped.t0)))) fail(`accepted stroke ${s.id} has malformed canonical geometry mapping`);
    const surface=strokesDoc!.surfaces.find(x=>x.surfaceId===s.surfaceId&&x.transformId===s.transformId);
    const points=s.points.flatMap(p=>[p.x,p.y,p.timeMs-s.startMs]);
    if(!surface||mapped.fw!==surface.width||mapped.fh!==surface.height||!isDeepStrictEqual(mapped.pts,points))fail(`accepted stroke ${s.id} mapping changes the observed surface geometry`);
    if (Object.hasOwn(mapped, "rgb")) fail(`accepted stroke ${s.id} mapping cannot supply observed RGB`);
    const {id:_sourceStrokeId,...canonical}=mapped; void _sourceStrokeId;
    return s.rgb === undefined ? canonical : { ...canonical, rgb: [...s.rgb] };
  }) ?? [];
  const events=interactionDoc?.events.map(e=>({...e})) ?? [];
  const windows=contextDoc?.windows.state==="collected"?contextDoc.windows.value.map(snapshot=>({t:snapshot.timeMs,windows:snapshot.entries.map(w=>({id:w.id,app:w.app,...(w.bundleId?{bundleId:w.bundleId}:{}),...(w.regular===undefined?{}:{regular:w.regular}),...(w.title?{title:w.title}:{}),bounds:w.bounds.map(n=>{if(!Number.isInteger(n))fail("window bounds require integer canonical coordinates");return n;}),layer:w.layer,...(w.frontmost===undefined?{}:{frontmost:w.frontmost})}))})):undefined;
  const cursor=contextDoc?.cursor.state==="collected"?contextDoc.cursor.value:undefined;
  if(input.marks.length && !input.additionalCanonicalFacts?.marks) fail("Record marks require an explicit canonical v1 geometry/time/frame mapping");
  const canonicalMarks=input.additionalCanonicalFacts?.marks??[];
  if(new Set(canonicalMarks.map(mark=>mark.id)).size!==canonicalMarks.length)fail("duplicate canonical mark id");
  for(const mark of canonicalMarks){if(!mark.id||!["circle","underline","line","arrow","tap","pin","mark","flag"].includes(mark.shape)||!Number.isSafeInteger(mark.t0)||!Number.isSafeInteger(mark.t1)||mark.t0<0||mark.t1<mark.t0||mark.t1>m.payload.clock.durationMs||!Array.isArray(mark.strokes)||mark.strokes.some(n=>!Number.isSafeInteger(n)||n<0||n>=strokes.length)||!mark.description||!Array.isArray(mark.boxNorm)||mark.boxNorm.length!==4||!mark.boxNorm.every(n=>Number.isFinite(n)&&n>=0&&n<=1)||([mark.from,mark.to].some(p=>p!==undefined&&(!Array.isArray(p)||p.length!==2||!p.every(n=>Number.isFinite(n)&&n>=0&&n<=1)))))fail(`canonical mark ${mark.id||"(empty)"} is malformed`);if(mark.frame){const p=artifactPath(mark.frame);if(byPath.get(p)?.role!=="frame")fail(`canonical mark frame reference is missing: ${p}`);}}
  if(strokesDoc&&(input.additionalCanonicalFacts?.strokes?.length??0)!==strokesDoc.strokes.length)fail("canonical stroke mapping count does not match accepted strokes");
  if(!strokesDoc&&(input.additionalCanonicalFacts?.strokes?.length??0)>0)fail("canonical stroke mapping has no matching accepted-strokes fact");
  const mappedStrokeIds=input.additionalCanonicalFacts?.strokes?.map(s=>s.id)??[];
  if(new Set(mappedStrokeIds).size!==mappedStrokeIds.length)fail("duplicate canonical stroke mapping");
  for(const frame of frameFacts)if(frame.markId&&!canonicalMarks.some(mark=>mark.id===frame.markId))fail(`selected frame references missing canonical mark ${frame.markId}`);
  const target=input.admission.target;
  const document: Record<string,unknown>={version:1,id:input.admission.bundleId,createdAtMs:input.admission.createdAtMs,durationMs:m.payload.clock.durationMs,target,transcript,
    strokes,marks:canonicalMarks,events,...(m.payload.collection.interactions==="unavailable"?{interactionTelemetry:"videoOnly"}:{}),
    keyframes:frameFacts.map(f=>({index:f.index,t:f.timeMs,reason:f.canonicalReason,file:f.file,event:f.event??"",...(f.text?{said:f.text}:{}),...(f.markId?{markId:f.markId}:{})})),contactSheet:artifactPath(sheet.relativePath),video:m.payload.video.file,...(m.payload.clock.videoOffsetMs?{videoOffsetMs:m.payload.clock.videoOffsetMs}:{}),
    ...(m.payload.audio?{audio:m.payload.audio.file}:{}),...(input.admission.source?{source:input.admission.source}:{}),...(input.admission.device?{device:input.admission.device}:{}),...(input.admission.note?{note:input.admission.note}:{}),
    ...(input.admission.imageQualityWarning?{imageQualityWarning:input.admission.imageQualityWarning}:{}),
    ...(context&&Object.keys(context.values).length?{context:context.values}:{}),
    ...(context&&context.changes.length?{contextChanges:context.changes.map(c=>({t:c.timeMs,key:c.key,...(c.value===null?{value:null}:{value:c.value})}))}:{}),
    ...(input.admission.destination?{destination:input.admission.destination}:{}),
    ...(windows?.length?{windows}:{}),...(input.additionalCanonicalFacts?.screens?.length?{screens:input.additionalCanonicalFacts.screens}:{}),...(cursor?.length?{cursor}:{}),...(input.additionalCanonicalFacts?.autocrop?{autocrop:input.additionalCanonicalFacts.autocrop}:{})};
  if(input.admission.destination&&(!input.admission.destination.session||typeof input.admission.destination.title!=="string"||!input.admission.destination.via))fail("explicit destination metadata is incomplete");
  if (input.processingWarnings?.some(warning => warning.kind === "audioInitialGap")) fail("initial audio gap must derive from the original sealed materials warning");
  if (input.processingWarnings?.some(warning => warning.kind === "transcriptUnavailable")) fail("transcript unavailability warning must derive from original sealed materials");
  const initialGaps = m.payload.warnings.filter(warning => warning.code === "audio-initial-gap");
  if (initialGaps.length > 1) fail("duplicate initial audio gap warnings");
  const gap = initialGaps[0];
  if (gap && (!m.payload.audio || m.payload.clock.audioOffsetMs !== 0 || !Number.isSafeInteger(gap.atMs) || gap.atMs! < 0 || gap.atMs! >= m.payload.clock.durationMs || gap.atMs! >= m.payload.audio.durationMs))
    fail("initial audio gap requires aligned original audio and an explicit first sample time within capture duration");
  const transcriptStatus: ProcessingInput["transcriptStatus"] = m.payload.collection.transcript === "unavailable"
    ? { state: "unavailable" }
    : { state: "collected", content: transcriptDoc!.text.trim().length > 0 || transcriptDoc!.segments.some(segment => segment.text.trim().length > 0) ? "nonempty" : "empty", segmentCount: transcriptDoc!.segments.length };
  const transcriptUnavailable = m.payload.warnings.some(warning => warning.code === "transcript-unavailable");
  if (transcriptUnavailable && transcriptStatus.state !== "unavailable") fail("transcript failure warning contradicts collected original facts");
  const processingWarnings: ProcessingWarning[] = [...(input.processingWarnings ?? []), ...(gap ? [{ kind: "audioInitialGap" as const, atMs: gap.atMs! }] : []), ...(transcriptUnavailable ? [{ kind: "transcriptUnavailable" as const }] : [])];
  const processing: ProcessingInput={durationMs:m.payload.clock.durationMs,bundlePath:input.admission.bundlePath,target:{app:target.app,windowTitle:target.windowTitle},transcript:{text:transcript.text,segments:transcript.segments.map(s=>({startMs:s.t0,text:s.text}))},transcriptStatus,frames:frameFacts.map(f=>({...f,absolutePath:resolve(input.admission.bundlePath,f.file)})),marks:input.marks as ProcessingInput["marks"],context:context?.values,contextChanges:context?.changes.map(c=>({timeMs:c.timeMs,key:c.key,value:c.value??undefined})),contactSheetPath:resolve(input.admission.bundlePath,sheet.relativePath),videoPath:resolve(input.admission.bundlePath,m.payload.video.file),destination:input.admission.destination,events:events.map(e=>({kind:e.kind,timeMs:e.t,...(e.kind==="note"?{pin:e.pin,text:e.text}:e.kind==="app_event"?{label:e.label}:{})} as never)),warnings:processingWarnings.length ? processingWarnings : input.processingWarnings};
  const strokeFact = strokesDoc ? input.facts.acceptedStrokes! : undefined;
  if (strokeFact) processing.acceptedStrokeDocument = { path: resolve(input.admission.bundlePath, artifactPath(strokeFact.file.relativePath)), strokeCount: strokesDoc!.strokes.length };
  const summary=utf8(assembleProcessingOutput(processing).summaryMarkdown), apparentJson=utf8(`${JSON.stringify(document,null,2)}\n`);
  const entries=[{path:"apparent.json",role:"manifest",bytes:apparentJson,provenance:"record-generated" as const},{path:"summary.md",role:"summary",bytes:summary,provenance:"record-generated" as const},...input.artifacts.map(a=>({path:artifactPath(a.relativePath),role:a.role,bytes:a.bytes,identity:a.identity,provenance:(a.role==="video"||a.role==="audio"?"material-reference":"caller-attested") as "material-reference"|"caller-attested"}))].map(e=>({path:e.path,role:e.role,sha256:hash(e.bytes),byteLength:e.bytes.byteLength,provenance:e.provenance,...("identity" in e?{identity:e.identity}:{})}));
  for (const [path, reference] of materialReferences) entries.push({path,role:reference.role,sha256:reference.file.sha256,byteLength:reference.file.identity.size,provenance:"material-reference",identity:reference.file.identity});
  if (strokeFact) {
    const path = artifactPath(strokeFact.file.relativePath);
    if (entries.some(entry => entry.path === path)) fail("original stroke document collides with bundle output");
    entries.push({ path, role: "companion", sha256: strokeFact.file.sha256, byteLength: strokeFact.bytes.byteLength,
      provenance: "material-reference", identity: strokeFact.file.identity });
  }
  entries.sort((a,b)=>a.path<b.path?-1:a.path>b.path?1:0);
  const inventory:PublicationInventory={schema:"apparent.record-bundle-inventory/1",materials:{...m.manifest.key},retainedFacts:{confirmedSource:structuredClone(m.payload.confirmedSource),invocation:structuredClone(m.payload.invocation),references:structuredClone(m.payload.facts)},collection:{...m.payload.collection},observations:{context:contextDoc?.context.state??"unavailable",session:contextDoc?.session.state??"unavailable",windows:contextDoc?.windows.state??"unavailable",cursor:contextDoc?.cursor.state??"unavailable",interactions:m.payload.collection.interactions,transcript:m.payload.collection.transcript,acceptedStrokes:m.payload.collection.acceptedStrokes},warnings:m.payload.warnings.map(w=>({...w})),entries};
  return {bundlePath:input.admission.bundlePath,apparentJson,summaryMd:summary,processing:structuredClone(processing),inventory,materials:m,validatedFacts:{...(contextDoc?{context:contextDoc}:{}),...(interactionDoc?{interactions:interactionDoc}:{}),...(transcriptDoc?{transcript:transcriptDoc}:{}),...(strokesDoc?{acceptedStrokes:strokesDoc}:{})}};
}
