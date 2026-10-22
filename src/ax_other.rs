//! What's under a click or a mark comes from the macOS Accessibility tree (`ax.rs`, the
//! `agentContext` experiment). Elsewhere clicks and marks are recorded without it; these keep the
//! same items so the capture code is the same everywhere.

use std::collections::HashMap;

use crate::model::{InteractionEvent, Stroke, TargetFrame};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Pointer(u64),
    Stroke(u64),
}

pub struct Hit;

pub struct Prober;

static PROBER: Prober = Prober;

impl Prober {
    pub fn shared() -> &'static Prober {
        &PROBER
    }

    pub fn begin(&self) {}

    pub fn probe(&self, _target: Target, _x: f64, _y: f64) {}

    pub fn finish(&self) -> HashMap<Target, Hit> {
        HashMap::new()
    }
}

pub fn apply(_hits: &HashMap<Target, Hit>, _events: &mut [InteractionEvent], _strokes: &mut [Stroke]) {}

pub fn stroke_probe_point(_stroke: &Stroke, _frame: TargetFrame) -> Option<(f64, f64)> {
    None
}

pub(crate) fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(byte) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
