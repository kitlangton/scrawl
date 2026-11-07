//! Owner-authenticated trusted-native-observer accepted history. This is not OS
//! attestation: the privileged host joins its native producer before forwarding.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

type Result<T> = std::result::Result<T, &'static str>;
pub const GRACE_NS: u64 = 2_000_000_000;
const MAX_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
enum Phase {
    Collecting,
    Complete,
    Unavailable,
    FrozenComplete,
    FrozenUnavailable,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Registration {
    pub revision: u64,
    registration_id: String,
    token: String,
    observation: Value,
    admission_ns: u64,
    phase: Phase,
    surfaces: BTreeMap<String, Surface>,
    current: BTreeMap<String, String>,
    digests: Vec<String>,
    ids: HashSet<String>,
    active: Option<Gesture>,
    strokes: Vec<Value>,
    total_points: usize,
    last_ns: u64,
    native_revision: u64,
    finish: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Surface {
    value: Value,
    x: f64,
    y: f64,
    width: u32,
    height: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Gesture {
    id: String,
    surface: String,
    start: u64,
    last: u64,
    rgb: Option<[u8; 3]>,
    points: Vec<i32>,
}
fn exact(v: &Value, keys: &[&str]) -> Result<()> {
    let o = v.as_object().ok_or("malformed_request")?;
    if o.len() != keys.len() || keys.iter().any(|k| !o.contains_key(*k)) {
        return Err("malformed_request");
    }
    Ok(())
}
fn text<'a>(v: &'a Value) -> Result<&'a str> {
    v.as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        .ok_or("malformed_request")
}
fn number(v: &Value) -> Result<f64> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or("malformed_request")
}
fn safe(v: &Value) -> Result<u64> {
    v.as_u64()
        .filter(|n| *n <= 9_007_199_254_740_991)
        .ok_or("malformed_request")
}
pub fn ns(v: &Value) -> Result<u64> {
    let s = v.as_str().ok_or("malformed_request")?;
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("malformed_request");
    }
    s.parse().map_err(|_| "malformed_request")
}
fn ident(v: &Value) -> Result<&str> {
    super::id(v).ok_or("malformed_request")
}
fn rectangle(v: &Value, scale: bool) -> Result<()> {
    exact(
        v,
        if scale {
            &["x", "y", "width", "height", "scale"]
        } else {
            &["x", "y", "width", "height"]
        },
    )?;
    for k in ["x", "y", "width", "height"] {
        number(&v[k])?;
    }
    if number(&v["width"])? <= 0.0 || number(&v["height"])? <= 0.0 || (scale && v["scale"] != 1) {
        return Err("source_changed");
    }
    Ok(())
}
fn key(surface: &Value) -> Result<String> {
    Ok(format!(
        "{}\0{}",
        text(&surface["surfaceId"])?,
        text(&surface["transformId"])?
    ))
}
fn parse_surface(v: &Value, source: &Value) -> Result<Surface> {
    exact(
        v,
        &[
            "surfaceId",
            "transformId",
            "nativeIncarnation",
            "target",
            "pointSpace",
            "contentRectangle",
            "plane",
        ],
    )?;
    for k in ["surfaceId", "transformId", "nativeIncarnation"] {
        text(&v[k])?;
    }
    rectangle(&v["contentRectangle"], false)?;
    exact(&v["plane"], &["width", "height"])?;
    let w = u32::try_from(safe(&v["plane"]["width"])?).map_err(|_| "source_changed")?;
    let h = u32::try_from(safe(&v["plane"]["height"])?).map_err(|_| "source_changed")?;
    if w == 0 || h == 0 {
        return Err("source_changed");
    }
    let (x, y) = match v["target"].as_str() {
        Some("capture-rectangle") => {
            let b = &source["bounds"];
            if v["pointSpace"] != "desktop-physical"
                || v["contentRectangle"]
                    != json!({"x":b["x"],"y":b["y"],"width":b["width"],"height":b["height"]})
                || f64::from(w) != number(&b["width"])?
                || f64::from(h) != number(&b["height"])?
            {
                return Err("source_changed");
            }
            (number(&b["x"])?, number(&b["y"])?)
        }
        // A resizable Board viewport is NOT its logical paper plane. Coordinates
        // are already paper-local; movement/resize changes the transform only.
        Some("whiteboard-canvas")
            if v["pointSpace"] == "paper-logical" && w == 1600 && h == 1100 =>
        {
            (0.0, 0.0)
        }
        _ => return Err("source_changed"),
    };
    Ok(Surface {
        value: v.clone(),
        x,
        y,
        width: w,
        height: h,
    })
}
pub fn limits() -> Value {
    json!({"maxWireBytes":65536,"maxBatchJsonBytes":16384,"maxPointsPerBatch":256,"maxPointsPerGesture":2400,"maxBatches":4096,"maxSurfaces":8,"maxGestures":4096,"maxTotalPoints":65536,"maxFactBytes":MAX_BYTES,"finishGraceMs":2000})
}
impl Registration {
    pub fn new(req: &Value, source: &Value, admission_ns: u64, boot: &str) -> Result<Self> {
        if req["ingressVersion"] != 1
            || req["source"] != json!({"id":source["id"],"generation":source["generation"]})
            || source["kind"] != "display"
            || number(&source["bounds"]["scale"])? != 1.0
        {
            return Err("source_changed");
        }
        let o = &req["observation"];
        exact(
            o,
            &[
                "host",
                "producerId",
                "generation",
                "bootId",
                "clock",
                "output",
                "surfaces",
            ],
        )?;
        ident(&o["producerId"])?;
        safe(&o["generation"])?;
        if o["bootId"] != boot || o["clock"] != "linux-clock-monotonic-observation" {
            return Err("source_changed");
        }
        let host = &o["host"];
        exact(host, &["processEpoch", "revision", "pid", "startTicks"])?;
        text(&host["processEpoch"])?;
        safe(&host["revision"])?;
        if safe(&host["pid"])? == 0 {
            return Err("malformed_request");
        }
        ns(&host["startTicks"])?;
        let output = &o["output"];
        exact(output, &["name", "physical", "compositor"])?;
        rectangle(&output["physical"], false)?;
        rectangle(&output["compositor"], true)?;
        let b = &source["bounds"];
        let expected = json!({"x":b["x"],"y":b["y"],"width":b["width"],"height":b["height"]});
        if output["name"] != source["name"]
            || output["physical"] != expected
            || output["compositor"] != *b
        {
            return Err("source_changed");
        }
        let surfaces = o["surfaces"].as_array().ok_or("malformed_request")?;
        if surfaces.is_empty() || surfaces.len() > 8 {
            return Err("invalid_materials");
        }
        let mut r = Self {
            revision: 1,
            registration_id: super::new_id().map_err(|_| "internal")?,
            token: format!(
                "{}{}",
                super::new_id().map_err(|_| "internal")?,
                super::new_id().map_err(|_| "internal")?
            ),
            observation: o.clone(),
            admission_ns,
            phase: Phase::Collecting,
            surfaces: BTreeMap::new(),
            current: BTreeMap::new(),
            digests: vec![],
            ids: HashSet::new(),
            active: None,
            strokes: vec![],
            total_points: 0,
            last_ns: admission_ns,
            native_revision: 0,
            finish: None,
        };
        for s in surfaces {
            let p = parse_surface(s, source)?;
            let k = key(s)?;
            let id = text(&s["surfaceId"])?.to_owned();
            if r.current.insert(id, k.clone()).is_some() {
                return Err("identity_conflict");
            }
            r.surfaces.insert(k, p);
        }
        Ok(r)
    }
    pub fn receipt(&self, capture: &str) -> Value {
        json!({"ingressVersion":1,"captureId":capture,"producerId":self.observation["producerId"],"generation":self.observation["generation"],"registrationId":self.registration_id,"producerToken":self.token,"nextSequence":0,"clock":{"kind":"linux-clock-monotonic-observation","bootId":self.observation["bootId"],"admissionMonotonicNs":self.admission_ns.to_string()},"limits":limits()})
    }
    pub fn matches(&self, req: &Value) -> bool {
        self.observation == req["observation"]
    }
    pub fn authenticate(&self, req: &Value) -> Result<()> {
        if req["ingressVersion"] != 1
            || req["registrationId"] != self.registration_id
            || req["producerToken"] != self.token
        {
            return Err("permission_denied");
        }
        Ok(())
    }
    pub fn collecting(&self) -> bool {
        self.phase == Phase::Collecting
    }
    pub fn frozen(&self) -> bool {
        matches!(self.phase, Phase::FrozenComplete | Phase::FrozenUnavailable)
    }
    pub fn invalidate(&mut self) {
        if self.frozen() {
            return;
        }
        self.phase = Phase::Unavailable;
        self.active = None;
        self.revision += 1;
    }
    fn stamp(&mut self, v: &Value, cutoff: u64) -> Result<u64> {
        let n = ns(v)?;
        if n < self.admission_ns || n < self.last_ns || n > cutoff {
            return Err("invalid_materials");
        }
        self.last_ns = n;
        Ok((n - self.admission_ns) / 1_000_000)
    }
    fn active_id(&self, e: &Value) -> Result<()> {
        if self.active.as_ref().is_none_or(|g| e["gestureId"] != g.id) {
            return Err("identity_conflict");
        }
        Ok(())
    }
    fn event(
        &mut self,
        e: &Value,
        cutoff: u64,
        lifecycle_cutoff: u64,
        source: &Value,
    ) -> Result<usize> {
        match e["kind"].as_str() {
            Some("begin") => {
                let mut fields = vec![
                    "kind",
                    "gestureId",
                    "surfaceId",
                    "transformId",
                    "nativeIncarnation",
                    "timeNs",
                    "x",
                    "y",
                ];
                if e.get("rgb").is_some() {
                    fields.push("rgb")
                }
                exact(e, &fields)?;
                if self.active.is_some() || self.ids.len() >= 4096 {
                    return Err("invalid_materials");
                }
                let id = ident(&e["gestureId"])?.to_owned();
                if !self.ids.insert(id.clone()) {
                    return Err("identity_conflict");
                }
                let k = key(e)?;
                if self.current.get(text(&e["surfaceId"])?) != Some(&k) {
                    return Err("source_changed");
                }
                let s = self.surfaces.get(&k).ok_or("source_changed")?.clone();
                if e["nativeIncarnation"] != s.value["nativeIncarnation"] {
                    return Err("source_changed");
                }
                let t = self.stamp(&e["timeNs"], cutoff)?;
                let (x, y) = point(e, &s)?;
                let rgb = if let Some(v) = e.get("rgb") {
                    let c = v
                        .as_array()
                        .filter(|v| v.len() == 3)
                        .ok_or("invalid_materials")?;
                    Some([
                        u8::try_from(safe(&c[0])?).map_err(|_| "invalid_materials")?,
                        u8::try_from(safe(&c[1])?).map_err(|_| "invalid_materials")?,
                        u8::try_from(safe(&c[2])?).map_err(|_| "invalid_materials")?,
                    ])
                } else {
                    None
                };
                self.active = Some(Gesture {
                    id,
                    surface: k,
                    start: t,
                    last: t,
                    rgb,
                    points: vec![x, y, 0],
                });
                Ok(1)
            }
            Some("points") => {
                exact(e, &["kind", "gestureId", "points"])?;
                self.active_id(e)?;
                let p = e["points"]
                    .as_array()
                    .filter(|p| !p.is_empty() && p.len() <= 256)
                    .ok_or("invalid_materials")?;
                let g = self.active.as_ref().unwrap();
                if g.points.len() / 3 + p.len() > 2400 {
                    return Err("invalid_materials");
                }
                let s = self
                    .surfaces
                    .get(&g.surface)
                    .ok_or("source_changed")?
                    .clone();
                let start = g.start;
                for v in p {
                    exact(v, &["timeNs", "x", "y"])?;
                    let t = self.stamp(&v["timeNs"], cutoff)?;
                    let (x, y) = point(v, &s)?;
                    let dt = i32::try_from(t - start).map_err(|_| "invalid_materials")?;
                    let g = self.active.as_mut().unwrap();
                    g.points.extend([x, y, dt]);
                    g.last = t;
                }
                Ok(p.len())
            }
            Some("commit") => {
                exact(e, &["kind", "gestureId", "acceptedAtNs"])?;
                self.active_id(e)?;
                let accepted = self.stamp(&e["acceptedAtNs"], cutoff)?;
                let g = self.active.take().unwrap();
                let s = &self.surfaces[&g.surface];
                let donor = apparent_capture_engine::ink::Stroke {
                    t0: g.start,
                    t1: g.last,
                    fw: s.width,
                    fh: s.height,
                    on: None,
                    pin: None,
                    laser: false,
                    rgb: g.rgb,
                    pts: g.points,
                };
                for (index, part) in apparent_capture_engine::ink::clip_recording_ink(&donor)
                    .into_iter()
                    .enumerate()
                {
                    let id = if index == 0 {
                        g.id.clone()
                    } else {
                        let id =
                            super::sha(format!("{}:{index}", g.id).as_bytes())[..32].to_owned();
                        if !self.ids.insert(id.clone()) {
                            return Err("identity_conflict");
                        }
                        id
                    };
                    let points: Vec<Value> = part
                        .pts
                        .chunks_exact(3)
                        .map(|p| json!({"timeMs":part.t0+p[2] as u64,"x":p[0],"y":p[1]}))
                        .collect();
                    if points.is_empty() {
                        continue;
                    }
                    let mut stroke = json!({"id":id,"surface":if s.value["target"]=="capture-rectangle"{"screen"}else{"whiteboard"},"surfaceId":s.value["surfaceId"],"transformId":s.value["transformId"],"startMs":part.t0,"endMs":part.t1,"acceptedAtMs":accepted,"points":points});
                    if let Some(rgb) = part.rgb {
                        stroke["rgb"] = json!(rgb)
                    }
                    self.strokes.push(stroke);
                }
                Ok(0)
            }
            Some("discard") => {
                exact(e, &["kind", "gestureId", "observedAtNs", "reason"])?;
                self.active_id(e)?;
                if ![
                    "undo-active",
                    "clear-active",
                    "source-changed",
                    "input-sealed",
                ]
                .contains(&text(&e["reason"])?)
                {
                    return Err("malformed_request");
                }
                let end = if e["reason"] == "input-sealed" {
                    lifecycle_cutoff
                } else {
                    cutoff
                };
                self.stamp(&e["observedAtNs"], end)?;
                self.active = None;
                Ok(0)
            }
            Some("undo" | "clear") => {
                exact(
                    e,
                    &[
                        "kind",
                        "surfaceId",
                        "transformId",
                        "nativeIncarnation",
                        "observedAtNs",
                        "nativeRevision",
                    ],
                )?;
                let k = key(e)?;
                let s = self.surfaces.get(&k).ok_or("source_changed")?;
                if e["nativeIncarnation"] != s.value["nativeIncarnation"]
                    || self.current.get(text(&e["surfaceId"])?) != Some(&k)
                    || self.active.is_some()
                {
                    return Err("source_changed");
                }
                self.control_stamp(e, cutoff)?;
                Ok(0)
            }
            Some("surface-state") => {
                exact(
                    e,
                    &["kind", "state", "surface", "observedAtNs", "nativeRevision"],
                )?;
                let s = &e["surface"];
                let p = parse_surface(s, source)?;
                let k = key(s)?;
                let id = text(&s["surfaceId"])?.to_owned();
                if self
                    .active
                    .as_ref()
                    .is_some_and(|g| self.surfaces[&g.surface].value["surfaceId"] == id)
                {
                    return Err("source_changed");
                }
                match text(&e["state"])? {
                    "closed" => {
                        if self.current.get(&id) != Some(&k)
                            || self.surfaces.get(&k).is_none_or(|old| old.value != *s)
                        {
                            return Err("source_changed");
                        }
                        self.current.remove(&id);
                    }
                    "opened" | "replaced" => {
                        if self.current.len() >= 8 && !self.current.contains_key(&id) {
                            return Err("invalid_materials");
                        }
                        if self.surfaces.get(&k).is_some_and(|old| old.value != *s) {
                            return Err("identity_conflict");
                        }
                        self.current.insert(id, k.clone());
                        self.surfaces.insert(k, p);
                    }
                    _ => return Err("malformed_request"),
                }
                self.control_stamp(
                    e,
                    if e["state"] == "closed" {
                        lifecycle_cutoff
                    } else {
                        cutoff
                    },
                )?;
                Ok(0)
            }
            _ => Err("malformed_request"),
        }
    }
    fn control_stamp(&mut self, e: &Value, cutoff: u64) -> Result<()> {
        let revision = safe(&e["nativeRevision"])?;
        if revision < self.native_revision {
            return Err("source_changed");
        }
        self.stamp(&e["observedAtNs"], cutoff)?;
        self.native_revision = revision;
        Ok(())
    }
    pub fn submit(&mut self, req: &Value, cutoff: u64, source: &Value) -> Result<Value> {
        self.submit_at(req, cutoff, cutoff, source)
    }
    pub fn submit_at(
        &mut self,
        req: &Value,
        cutoff: u64,
        lifecycle_cutoff: u64,
        source: &Value,
    ) -> Result<Value> {
        self.authenticate(req)?;
        let seq = safe(&req["sequence"])?;
        let raw = req["eventsJson"].as_str().ok_or("malformed_request")?;
        if raw.len() > 16384 || req["batchSha256"] != super::sha(raw.as_bytes()) {
            return Err("invalid_materials");
        }
        let digest = text(&req["batchSha256"])?;
        if (seq as usize) < self.digests.len() {
            if self.phase == Phase::Unavailable || self.digests[seq as usize] != digest {
                return Err("identity_conflict");
            }
            return Ok(self.batch_receipt(req, "replayed"));
        }
        let result = (|| {
            if self.phase != Phase::Collecting || seq != self.digests.len() as u64 || seq >= 4096 {
                return Err("source_mismatch");
            }
            let events: Vec<Value> = serde_json::from_str(raw).map_err(|_| "malformed_request")?;
            if events.is_empty() || events.len() > 256 {
                return Err("invalid_materials");
            }
            let mut points = 0;
            for e in &events {
                points += self.event(
                    e,
                    cutoff,
                    lifecycle_cutoff.min(cutoff.saturating_add(GRACE_NS)),
                    source,
                )?;
                if points > 256 {
                    return Err("invalid_materials");
                }
            }
            self.total_points += points;
            if self.total_points > 65536 || self.surfaces.len() > 4096 || self.strokes.len() > 4096
            {
                return Err("invalid_materials");
            }
            if serde_json::to_vec(&self.fact_value(
                "00000000000000000000000000000000",
                "00000000000000000000000000000000",
            ))
            .map_err(|_| "internal")?
            .len()
                > MAX_BYTES
            {
                return Err("invalid_materials");
            }
            self.digests.push(digest.to_owned());
            self.revision += 1;
            Ok(self.batch_receipt(req, "accepted"))
        })();
        if result.is_err() {
            self.invalidate()
        }
        result
    }
    fn batch_receipt(&self, req: &Value, outcome: &str) -> Value {
        json!({"ingressVersion":1,"captureId":req["captureId"],"registrationId":self.registration_id,"sequence":req["sequence"],"batchSha256":req["batchSha256"],"nextSequence":req["sequence"].as_u64().unwrap()+1,"outcome":outcome})
    }
    pub fn finish(&mut self, req: &Value, now: u64) -> Result<Value> {
        self.authenticate(req)?;
        let stable = json!({"ingressVersion":req["ingressVersion"],"ownerToken":req["ownerToken"],"captureId":req["captureId"],"registrationId":req["registrationId"],"producerToken":req["producerToken"],"finalSequence":req["finalSequence"],"seal":req["seal"]});
        if let Some(prior) = &self.finish {
            if prior != &stable {
                return Err("identity_conflict");
            }
            return Ok(self.finish_receipt(req));
        }
        if self.frozen() {
            return Err("unavailable");
        }
        exact(&req["seal"], &["state", "observedAtNs", "nativeRevision"])?;
        let expected = if self.digests.is_empty() {
            Value::Null
        } else {
            json!(self.digests.len() - 1)
        };
        let stamp = ns(&req["seal"]["observedAtNs"])?;
        let revision = safe(&req["seal"]["nativeRevision"])?;
        if self.phase != Phase::Collecting
            || req["finalSequence"] != expected
            || req["seal"]["state"] != "sealed-and-drained"
            || stamp < self.last_ns
            || stamp > now
            || revision < self.native_revision
            || self.active.is_some()
        {
            self.invalidate()
        } else {
            self.phase = Phase::Complete;
            self.revision += 1
        }
        self.finish = Some(stable);
        Ok(self.finish_receipt(req))
    }
    fn finish_receipt(&self, req: &Value) -> Value {
        json!({"ingressVersion":1,"captureId":req["captureId"],"registrationId":self.registration_id,"finalSequence":req["finalSequence"],"state":if matches!(self.phase, Phase::Complete | Phase::FrozenComplete){"complete"}else{"unavailable"}})
    }
    fn fact_value(&self, capture: &str, materials: &str) -> Value {
        json!({"schema":"apparent.capture-strokes/1","identity":{"captureId":capture,"materialsId":materials},"coordinates":"target-normalized-10000","surfaces":self.surfaces.values().map(|s|json!({"surfaceId":s.value["surfaceId"],"transformId":s.value["transformId"],"width":s.width,"height":s.height,"target":s.value["target"]})).collect::<Vec<_>>(),"strokes":self.strokes})
    }
    pub fn freeze(&mut self, capture: &str, materials: &str) -> Option<Value> {
        if !self.frozen() {
            self.phase = if self.phase == Phase::Complete {
                Phase::FrozenComplete
            } else {
                Phase::FrozenUnavailable
            };
            self.revision += 1;
        }
        self.facts(capture, materials)
    }
    pub fn facts(&self, capture: &str, materials: &str) -> Option<Value> {
        matches!(self.phase, Phase::Complete | Phase::FrozenComplete)
            .then(|| self.fact_value(capture, materials))
    }
}
fn point(v: &Value, s: &Surface) -> Result<(i32, i32)> {
    // Match the donor f32 arithmetic, rejecting overflow before narrowing. Raw
    // excursions survive until Core clips/splits; never clamp to the viewport.
    let normalize = |v: &Value, origin: f64, size: u32| -> Result<i32> {
        let input = number(v)?;
        let normalized = if s.value["target"] == "capture-rectangle" {
            ((input - origin) / f64::from(size)) as f32
        } else {
            (input as f32 - origin as f32) / size as f32
        };
        let value = (normalized * 10_000.0_f32).round();
        if !value.is_finite()
            || f64::from(value) < f64::from(i32::MIN)
            || f64::from(value) > f64::from(i32::MAX)
        {
            return Err("invalid_materials");
        }
        Ok(value as i32)
    };
    Ok((
        normalize(&v["x"], s.x, s.width)?,
        normalize(&v["y"], s.y, s.height)?,
    ))
}
#[cfg(test)]
#[path = "ink_tests.rs"]
mod tests;
