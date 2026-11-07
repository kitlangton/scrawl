//! A single source sample clock, retained without internal gap filling.
//! The initial origin is rounded UP once to an integer millisecond (16 samples
//! at 16 kHz). This shifts all real PCM by <1 ms, never cumulatively. The exact
//! mapped source origin remains available to the caller. A nonzero prefix is
//! unobserved/padded time, not evidence of recorded silence.

use anyhow::{Result, ensure};

const RATE: u64 = 16_000;
/// Bound synthetic initial padding independently of the live sample ring.
pub const MAX_INITIAL_GAP_MS: u64 = 180_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    pub source_capture_ns: u64,
    pub initial_gap_ms: u64,
}

#[derive(Default)]
pub struct Timeline {
    origin: Option<Origin>,
    first_pts_ns: u64,
    source_samples: u64,
    failed: bool,
}
impl Timeline {
    /// Validate before appending to the bounded history. Source PTS must agree
    /// with the exact 16 kHz sample count within 1 ns of integer conversion.
    /// Any refusal poisons the timeline; later buffers cannot hide a loss.
    pub fn accept(&mut self, running_pts_ns: u64, capture_ns: u64, pcm: &[f32]) -> Result<Origin> {
        let result = self.accept_inner(running_pts_ns, capture_ns, pcm);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn accept_inner(
        &mut self,
        running_pts_ns: u64,
        capture_ns: u64,
        pcm: &[f32],
    ) -> Result<Origin> {
        ensure!(!self.failed, "Microphone timeline already failed");
        ensure!(
            !pcm.is_empty() && pcm.iter().all(|v| v.is_finite()),
            "Invalid microphone samples"
        );
        let origin = if let Some(origin) = self.origin {
            let expected = self
                .first_pts_ns
                .checked_add(
                    self.source_samples
                        .checked_mul(1_000_000_000)
                        .ok_or_else(|| anyhow::anyhow!("Microphone clock overflow"))?
                        / RATE,
                )
                .ok_or_else(|| anyhow::anyhow!("Microphone clock overflow"))?;
            ensure!(
                running_pts_ns.abs_diff(expected) <= 1,
                "Microphone source clock discontinuity"
            );
            origin
        } else {
            let gap = capture_ns / 1_000_000 + u64::from(capture_ns % 1_000_000 != 0);
            ensure!(
                gap <= MAX_INITIAL_GAP_MS,
                "Microphone initial gap exceeds limit"
            );
            self.first_pts_ns = running_pts_ns;
            Origin {
                source_capture_ns: capture_ns,
                initial_gap_ms: gap,
            }
        };
        let total = self
            .source_samples
            .checked_add(pcm.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("Microphone sample count overflow"))?;
        ensure!(
            total + origin.initial_gap_ms * 16 <= (u64::from(u32::MAX) - 36) / 2,
            "Microphone WAV size limit exceeded"
        );
        self.source_samples = total;
        self.origin = Some(origin);
        Ok(origin)
    }
    pub fn finish(&self) -> Result<(Origin, u64)> {
        ensure!(
            !self.failed && self.source_samples != 0,
            "No complete microphone timeline"
        );
        Ok((
            self.origin
                .ok_or_else(|| anyhow::anyhow!("No microphone origin"))?,
            self.source_samples,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_gap_rounds_once_and_sample_clock_does_not_accumulate_rounding() {
        let mut timeline = Timeline::default();
        let origin = timeline.accept(7, 1_234_567, &[0.25]).unwrap();
        assert_eq!(
            origin,
            Origin {
                source_capture_ns: 1_234_567,
                initial_gap_ms: 2
            }
        );
        for sample in 1..16_000u64 {
            // Callback observation is deliberately unrelated to sample time.
            timeline
                .accept(7 + sample * 62_500, 99_000_000_000, &[-0.25])
                .unwrap();
        }
        assert_eq!(timeline.finish().unwrap(), (origin, 16_000));
    }
    #[test]
    fn loss_overlap_invalid_pcm_and_excessive_initial_gap_fail_closed() {
        for pts in [0, 62_502, 125_000] {
            let mut timeline = Timeline::default();
            timeline.accept(0, 0, &[1.]).unwrap();
            assert!(timeline.accept(pts, 0, &[1.]).is_err());
            assert!(timeline.accept(62_500, 0, &[1.]).is_err());
            assert!(timeline.finish().is_err());
        }
        for pcm in [vec![], vec![f32::NAN], vec![f32::INFINITY]] {
            assert!(Timeline::default().accept(0, 0, &pcm).is_err());
        }
        assert!(
            Timeline::default()
                .accept(0, (MAX_INITIAL_GAP_MS + 1) * 1_000_000, &[1.])
                .is_err()
        );
        assert!(Timeline::default().finish().is_err());
    }
}
