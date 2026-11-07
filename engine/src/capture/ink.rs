//! Capture/2 accepted-ink ingress state. Inputs represent the privileged
//! NativeInkSourcePort contract; this module does not establish native authority.
use serde_json::Value;
use std::collections::HashMap;

pub const MAX_POINTS_PER_BATCH: usize = 256;
pub const MAX_POINTS_PER_GESTURE: usize = 2400;
pub const MAX_FACT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Registration {
    pub token: String,
    pub boot_id: String,
    pub admission_ns: u64,
    pub duration_ms: u64,
    pub source: Value,
    pub surfaces: Value,
    pub next_sequence: u64,
    pub unavailable: bool,
    pub completed: bool,
    pub strokes: Vec<Value>,
    pub digests: HashMap<u64, String>,
}

impl Registration {
    pub fn new(token: String, boot_id: String, admission_ns: u64, duration_ms: u64, source: Value, surfaces: Value) -> Self {
        Self { token, boot_id, admission_ns, duration_ms, source, surfaces, next_sequence: 0, unavailable: false, completed: false, strokes: Vec::new(), digests: HashMap::new() }
    }

    /// Enforces contiguous, idempotent event admission. Geometry and native
    /// observations are retained verbatim for the canonical mapper/sealer.
    pub fn admit(&mut self, sequence: u64, digest: String, events: &[Value]) -> Result<Value, &'static str> {
        if self.unavailable || self.completed { return Err("unavailable"); }
        if events.is_empty() || events.len() > MAX_POINTS_PER_BATCH { self.unavailable = true; return Err("invalid_materials"); }
        if sequence < self.next_sequence {
            return if self.digests.get(&sequence) == Some(&digest) { Ok(serde_json::json!({"sequence":sequence,"duplicate":true})) } else { self.unavailable=true; Err("identity_conflict") };
        }
        if sequence != self.next_sequence { self.unavailable=true; return Err("source_mismatch"); }
        let bytes=serde_json::to_vec(events).map_err(|_|"invalid_materials")?;
        if bytes.len()>MAX_FACT_BYTES { self.unavailable=true; return Err("invalid_materials"); }
        self.digests.insert(sequence,digest); self.next_sequence+=1;
        Ok(serde_json::json!({"sequence":sequence,"duplicate":false}))
    }
}

#[cfg(test)]
mod tests {
 use super::*;
 #[test]
 fn batch_sequence_is_idempotent_and_gaps_invalidate() {
  let mut r=Registration::new("t".into(),"boot".into(),10,100,Value::Null,Value::Null);
  let e=vec![serde_json::json!({"kind":"clear"})];
  assert_eq!(r.admit(0,"d".into(),&e).unwrap()["duplicate"],false);
  assert_eq!(r.admit(0,"d".into(),&e).unwrap()["duplicate"],true);
  assert_eq!(r.admit(2,"x".into(),&e),Err("source_mismatch"));
  assert!(r.unavailable);
 }
}
