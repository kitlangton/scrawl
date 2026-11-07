//! Strict accepted-history mapper for the agreed privileged NativeInkSourcePort v1.
//! Values entering this module are still not OS-attested; the caller must authenticate
//! the capture owner and privileged source port before constructing a Registration.
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_POINTS_PER_BATCH: usize = 256;
pub const MAX_POINTS_PER_GESTURE: usize = 2400;
pub const MAX_FACT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SURFACES: usize = 8;
pub const SCALE: f64 = 10_000.0;

#[derive(Clone, Debug)]
pub struct Registration {
    pub token: String, pub boot_id: String, pub admission_ns: u64, pub source: Value,
    pub surfaces: BTreeMap<String, Surface>, pub next_sequence: u64,
    pub unavailable: bool, pub completed: bool, pub strokes: Vec<Value>,
    digests: HashMap<u64, String>, ids: HashSet<String>, active: Option<Gesture>, fact_bytes: usize,
}
#[derive(Clone, Debug)]
pub struct Surface { pub id:String, pub transform:String, pub incarnation:String, pub target:String, pub x:f64,pub y:f64,pub width:f64,pub height:f64 }
#[derive(Clone, Debug)]
struct Gesture { id:String, surface:String, transform:String, start:u64, last:u64, rgb:Option<Value>, points:Vec<Value> }

fn uint(v:&Value)->Option<u64>{let s=v.as_str()?;if s.is_empty()||(s.len()>1&&s.starts_with('0'))||!s.bytes().all(|b|b.is_ascii_digit()){return None}s.parse().ok()}
fn finite(v:&Value)->Option<f64>{let n=v.as_f64()?; n.is_finite().then_some(n)}
fn stroke_id(v:&Value)->bool{v.as_str().is_some_and(|s|s.len()==32&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)))}
fn valid_id(v:&Value)->bool{v.as_str().is_some_and(|s|!s.is_empty()&&s.len()<=128&&s.bytes().all(|b|b.is_ascii_alphanumeric()||b"-_".contains(&b)))}
impl Registration {
 pub fn new(token:String, boot_id:String, admission_ns:u64, source:&Value, surfaces:&Value)->Result<Self,&'static str>{
  if !valid_id(&source["id"])||!valid_id(&source["generation"])||source["bounds"]["scale"].as_f64()!=Some(1.0)||source["kind"]!="display" {return Err("unavailable")}
  let list=surfaces.as_array().ok_or("malformed_request")?; if list.is_empty()||list.len()>MAX_SURFACES{return Err("invalid_materials")}
  let mut parsed=BTreeMap::new();
  for s in list {let idv=&s["surfaceId"];let tr=&s["transformId"];let inc=&s["incarnation"];if !valid_id(idv)||!valid_id(tr)||!valid_id(inc){return Err("malformed_request")}
   let target=s["target"].as_str().ok_or("malformed_request")?;let (x,y,w,h)=if target=="capture-rectangle" {let b=&source["bounds"];(finite(&b["x"]).ok_or("invalid_materials")?,finite(&b["y"]).ok_or("invalid_materials")?,finite(&b["width"]).ok_or("invalid_materials")?,finite(&b["height"]).ok_or("invalid_materials")?)}else if target=="whiteboard-canvas" {(0.0,0.0,1600.0,1100.0)}else{return Err("invalid_materials")};
   if w<=0.0||h<=0.0{return Err("invalid_materials")} let key=idv.as_str().unwrap().to_owned();if parsed.insert(key,Surface{id:idv.as_str().unwrap().into(),transform:tr.as_str().unwrap().into(),incarnation:inc.as_str().unwrap().into(),target:target.into(),x,y,width:w,height:h}).is_some(){return Err("invalid_materials")}
  }
  Ok(Self{token,boot_id,admission_ns,source:source.clone(),surfaces:parsed,next_sequence:0,unavailable:false,completed:false,strokes:vec![],digests:HashMap::new(),ids:HashSet::new(),active:None,fact_bytes:0})
 }
 fn time(&self,v:&Value,cutoff_ns:u64)->Result<u64,&'static str>{let ns=uint(v).ok_or("invalid_materials")?;if ns>cutoff_ns{return Err("invalid_materials")}let rel=ns.checked_sub(self.admission_ns).ok_or("invalid_materials")?;Ok(rel/1_000_000)}
 pub fn submit(&mut self,seq:u64,digest:String,events:&[Value],cutoff_ns:u64)->Result<Value,&'static str>{
  if self.unavailable||self.completed{return Err("unavailable")} if events.is_empty()||events.len()>MAX_POINTS_PER_BATCH{self.unavailable=true;return Err("invalid_materials")}
  if seq<self.next_sequence{return if self.digests.get(&seq)==Some(&digest){Ok(json!({"sequence":seq,"duplicate":true}))}else{self.unavailable=true;Err("identity_conflict")}}
  if seq!=self.next_sequence{self.unavailable=true;return Err("source_mismatch")}
  for e in events {if self.event(e,cutoff_ns).is_err(){self.unavailable=true;self.active=None;return Err("invalid_materials")}}
  let bytes=serde_json::to_vec(&self.strokes).map_err(|_|"internal")?;if bytes.len()>MAX_FACT_BYTES{self.unavailable=true;return Err("invalid_materials")}
  self.fact_bytes=bytes.len();self.digests.insert(seq,digest);self.next_sequence+=1;Ok(json!({"sequence":seq,"duplicate":false}))
 }
 fn event(&mut self,e:&Value,cutoff:u64)->Result<(), &'static str>{let kind=e["kind"].as_str().ok_or("malformed_request")?;match kind{
  "begin"=>{if self.active.is_some()||!stroke_id(&e["gestureId"]){return Err("invalid_materials")}let id=e["gestureId"].as_str().unwrap().to_owned();if !self.ids.insert(id.clone()){return Err("identity_conflict")}let surface=self.surfaces.get(e["surfaceId"].as_str().ok_or("malformed_request")?).ok_or("source_mismatch")?;if e["transformId"]!=surface.transform||e["incarnation"]!=surface.incarnation{return Err("source_changed")}let t=self.time(&e["timeNs"],cutoff)?;let rgb=e.get("rgb").cloned();if let Some(c)=&rgb{if !c.as_array().is_some_and(|a|a.len()==3&&a.iter().all(|x|x.as_u64().is_some_and(|n|n<=255))){return Err("invalid_materials")}}let p=point(e,surface,t)?;self.active=Some(Gesture{id,surface:surface.id.clone(),transform:surface.transform.clone(),start:t,last:t,rgb,points:vec![p]});}
  "points"=>{let list=e["points"].as_array().ok_or("malformed_request")?;if list.is_empty()||list.len()>MAX_POINTS_PER_BATCH{return Err("invalid_materials")}let (surface_id,last,count)={let g=self.active.as_ref().ok_or("invalid_materials")?;(g.surface.clone(),g.last,g.points.len())};let surface=self.surfaces.get(&surface_id).ok_or("source_mismatch")?.clone();let mut mapped=Vec::with_capacity(list.len());let mut prior=last;for raw in list{let t=self.time(raw.get("timeNs").ok_or("malformed_request")?,cutoff)?;if t<prior||count+mapped.len()>=MAX_POINTS_PER_GESTURE{return Err("invalid_materials")}mapped.push(point(raw,&surface,t)?);prior=t;}let g=self.active.as_mut().unwrap();g.points.extend(mapped);g.last=prior;}
  "commit"=>{let g=self.active.take().ok_or("invalid_materials")?;let t=self.time(&e["acceptedAtNs"],cutoff)?;if t<g.last{return Err("invalid_materials")}let mut value=json!({"id":g.id,"surface":if self.surfaces[&g.surface].target=="capture-rectangle"{"screen"}else{"whiteboard"},"surfaceId":g.surface,"transformId":g.transform,"startMs":g.start,"endMs":g.last,"acceptedAtMs":t,"points":g.points});if let Some(rgb)=g.rgb{value["rgb"]=rgb}self.strokes.push(value);}
  "undo"|"clear"|"surface-state"=>{if kind=="surface-state"&&e["state"]=="replace"{if self.active.as_ref().is_some_and(|g|e["surfaceId"]==g.surface){self.active=None}}}
  _=>return Err("malformed_request") } Ok(()) }
 pub fn finish(&mut self,final_sequence:u64,drained:bool)->bool{if !drained||final_sequence!=self.next_sequence.saturating_sub(1)||self.active.is_some()||self.unavailable{self.unavailable=true;return false}self.completed=true;true}
 pub fn facts(&self,capture_id:&str,materials_id:&str)->Option<Value>{if !self.completed||self.unavailable{return None}Some(json!({"schema":"apparent.capture-strokes/1","identity":{"captureId":capture_id,"materialsId":materials_id},"coordinates":"target-normalized-10000","surfaces":self.surfaces.values().map(|s|json!({"surfaceId":s.id,"transformId":s.transform,"width":s.width as u64,"height":s.height as u64,"target":s.target})).collect::<Vec<_>>(),"strokes":self.strokes}))}
}
fn point(v:&Value,s:&Surface,time:u64)->Result<Value,&'static str>{let x=finite(&v["x"]).ok_or("invalid_materials")?;let y=finite(&v["y"]).ok_or("invalid_materials")?;let nx=((x-s.x)/s.width*SCALE).round() as i64;let ny=((y-s.y)/s.height*SCALE).round() as i64;Ok(json!({"timeMs":time,"x":nx,"y":ny}))}

#[cfg(test)] mod tests {use super::*;fn reg()->Registration{let s=json!({"id":"source","generation":"gen","kind":"display","bounds":{"x":10,"y":20,"width":100,"height":50,"scale":1.0}});let surfaces=json!([{"surfaceId":"screen","transformId":"t1","incarnation":"i1","target":"capture-rectangle"}]);Registration::new("secret".into(),"boot".into(),1_000_000,&s,&surfaces).unwrap()}#[test]fn maps_native_clock_and_normalizes(){let mut r=reg();let ev=vec![json!({"kind":"begin","gestureId":"0123456789abcdef0123456789abcdef","surfaceId":"screen","transformId":"t1","incarnation":"i1","timeNs":"2000000","x":10,"y":20}),json!({"kind":"points","points":[{"timeNs":"3000000","x":110,"y":70}]}),json!({"kind":"commit","acceptedAtNs":"4000000"})];r.submit(0,"d".into(),&ev,10_000_000).unwrap();assert!(r.finish(0,true));let f=r.facts("0123456789abcdef0123456789abcdef","0123456789abcdef0123456789abcdef").unwrap();assert_eq!(f["strokes"][0]["points"][1]["x"],10000);assert_eq!(f["strokes"][0]["points"][1]["timeMs"],2);}
#[test]fn foreign_transform_future_time_and_sequence_gap_invalidate(){for bad in ["foreign","future"]{let mut r=reg();let time=if bad=="future"{20_000_000}else{2_000_000};let tr=if bad=="foreign"{"t2"}else{"t1"};let ev=vec![json!({"kind":"begin","gestureId":"0123456789abcdef0123456789abcdef","surfaceId":"screen","transformId":tr,"incarnation":"i1","timeNs":time,"x":10,"y":20})];assert!(r.submit(0,"d".into(),&ev,10_000_000).is_err());assert!(r.unavailable)}let mut r=reg();assert!(r.submit(1,"d".into(),&[json!({"kind":"clear"})],10_000_000).is_err());assert!(r.unavailable);}}
