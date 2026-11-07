//! Clock-domain conversion for strict ScreenCaptureKit samples.
//!
//! `screencapturekit` exposes its stream synchronization clock and `apple-cf` exposes
//! `CMClock::time`, but the locked public wrapper has no cross-clock conversion method.
//! CoreMedia's `CMSyncConvertTime` is therefore declared here as the only narrow FFI edge.

use std::ffi::c_void;

use screencapturekit::cm::{CMClock, CMTime};

/// Maximum integer timestamp accepted by the public TypeScript capture document.
pub const MAX_CAPTURE_MILLISECONDS: u64 = (1_u64 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockTimestamp {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

impl From<CMTime> for ClockTimestamp {
    fn from(time: CMTime) -> Self {
        Self {
            value: time.value,
            timescale: time.timescale,
            flags: time.flags,
            epoch: time.epoch,
        }
    }
}

/// Host-clock time sampled at the caller's capture-admission point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureAdmissionTime(CMTime);

impl CaptureAdmissionTime {
    /// Read the CoreMedia host clock. Call exactly at the owning capture-admission point;
    /// this does not derive an epoch from `Instant`, a callback, or stream setup.
    pub fn now() -> Result<Self, ClockError> {
        let time = CMClock::host_time_clock().time();
        validate_numeric(time)?;
        Ok(Self(time))
    }

    pub fn timestamp(self) -> ClockTimestamp {
        self.0.into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureClockSample {
    /// Original timestamp reported by the accepted SCK content sample.
    pub source_pts: ClockTimestamp,
    /// The same timestamp converted through the stream clock into CoreMedia host time.
    pub host_pts: ClockTimestamp,
    /// Host-time delta from caller-supplied capture admission, floored to integer ms.
    pub capture_relative_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    ClockUnavailable,
    InvalidTime,
    InvalidTimescale,
    EpochMismatch,
    BeforeAdmission,
    ArithmeticOverflow,
    OutsidePublicRange,
}

impl std::fmt::Display for ClockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "capture clock unavailable: {self:?}")
    }
}

impl std::error::Error for ClockError {}

fn validate_numeric(time: CMTime) -> Result<(), ClockError> {
    if !time.is_numeric() {
        return Err(ClockError::InvalidTime);
    }
    if time.timescale <= 0 {
        return Err(ClockError::InvalidTimescale);
    }
    Ok(())
}

/// Convert an SCK output sample PTS through the stream synchronization clock to host time,
/// then express it relative to the explicit host-clock admission observation.
pub fn map_sample_to_capture_clock(
    source_pts: CMTime,
    stream_clock: &CMClock,
    host_clock: &CMClock,
    admission: CaptureAdmissionTime,
) -> Result<CaptureClockSample, ClockError> {
    validate_numeric(source_pts)?;
    validate_numeric(admission.0)?;
    // SAFETY: both pointers are live CMClock objects retained for this call, and CMTime is
    // the repr(C) CoreMedia structure re-exported by the locked apple-cf dependency.
    let host_pts =
        unsafe { CMSyncConvertTime(source_pts, stream_clock.as_ptr(), host_clock.as_ptr()) };
    validate_numeric(host_pts)?;
    let capture_relative_ms = checked_relative_milliseconds(admission.0, host_pts)?;
    Ok(CaptureClockSample {
        source_pts: source_pts.into(),
        host_pts: host_pts.into(),
        capture_relative_ms,
    })
}

/// Exact rational subtraction followed by floor-to-milliseconds. The public clock is a
/// nonnegative JS-safe integer, so values beyond that range fail instead of saturating.
pub fn checked_relative_milliseconds(admission: CMTime, sample: CMTime) -> Result<u64, ClockError> {
    validate_numeric(admission)?;
    validate_numeric(sample)?;
    if admission.epoch != sample.epoch {
        return Err(ClockError::EpochMismatch);
    }

    let admission_scale = i128::from(admission.timescale);
    let sample_scale = i128::from(sample.timescale);
    let sample_scaled = i128::from(sample.value)
        .checked_mul(admission_scale)
        .ok_or(ClockError::ArithmeticOverflow)?;
    let admission_scaled = i128::from(admission.value)
        .checked_mul(sample_scale)
        .ok_or(ClockError::ArithmeticOverflow)?;
    let delta = sample_scaled
        .checked_sub(admission_scaled)
        .ok_or(ClockError::ArithmeticOverflow)?;
    if delta < 0 {
        return Err(ClockError::BeforeAdmission);
    }
    let denominator = sample_scale
        .checked_mul(admission_scale)
        .ok_or(ClockError::ArithmeticOverflow)?;
    checked_floor_milliseconds(delta, denominator)
}

fn checked_floor_milliseconds(delta: i128, denominator: i128) -> Result<u64, ClockError> {
    if denominator <= 0 {
        return Err(ClockError::InvalidTimescale);
    }
    if delta < 0 {
        return Err(ClockError::BeforeAdmission);
    }
    let milliseconds = delta
        .checked_mul(1000)
        .ok_or(ClockError::ArithmeticOverflow)?
        / denominator;
    let milliseconds = u64::try_from(milliseconds).map_err(|_| ClockError::OutsidePublicRange)?;
    if milliseconds > MAX_CAPTURE_MILLISECONDS {
        return Err(ClockError::OutsidePublicRange);
    }
    Ok(milliseconds)
}

// `screencapturekit` 11 reexports CMClock/CMTime but does not reexport the CoreMedia
// cross-clock conversion function. Keep the ABI declaration private to this adapter.
unsafe extern "C" {
    fn CMSyncConvertTime(time: CMTime, from: *const c_void, to: *const c_void) -> CMTime;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(value: i64, scale: i32, epoch: i64) -> CMTime {
        CMTime {
            value,
            timescale: scale,
            flags: 1,
            epoch,
        }
    }

    #[test]
    fn relative_milliseconds_uses_checked_rational_floor() {
        assert_eq!(
            checked_relative_milliseconds(t(3, 2, 4), t(7, 2, 4)),
            Ok(2000)
        );
        assert_eq!(
            checked_relative_milliseconds(t(1, 3, 0), t(2, 3, 0)),
            Ok(333)
        );
        assert_eq!(
            checked_relative_milliseconds(t(0, 1, 0), t(1, 10_000, 0)),
            Ok(0)
        );
    }

    #[test]
    fn relative_milliseconds_rejects_invalid_mismatched_and_negative_times() {
        assert_eq!(
            checked_relative_milliseconds(CMTime::INVALID, t(1, 1, 0)),
            Err(ClockError::InvalidTime)
        );
        assert_eq!(
            checked_relative_milliseconds(t(0, 0, 0), t(1, 1, 0)),
            Err(ClockError::InvalidTimescale)
        );
        assert_eq!(
            checked_relative_milliseconds(t(0, 1, 1), t(1, 1, 2)),
            Err(ClockError::EpochMismatch)
        );
        assert_eq!(
            checked_relative_milliseconds(t(2, 1, 0), t(1, 1, 0)),
            Err(ClockError::BeforeAdmission)
        );
    }

    #[test]
    fn relative_milliseconds_rejects_values_outside_public_range() {
        assert_eq!(
            checked_relative_milliseconds(t(0, 1, 0), t(MAX_CAPTURE_MILLISECONDS as i64, 1000, 0)),
            Ok(MAX_CAPTURE_MILLISECONDS)
        );
        assert_eq!(
            checked_relative_milliseconds(
                t(0, 1, 0),
                t(MAX_CAPTURE_MILLISECONDS as i64 + 1, 1000, 0)
            ),
            Err(ClockError::OutsidePublicRange)
        );
        assert_eq!(
            checked_relative_milliseconds(t(0, 1, 0), t(i64::MAX, 1, 0)),
            Err(ClockError::OutsidePublicRange)
        );
        assert!(checked_relative_milliseconds(t(0, 1, 0), t(1, 1, 0)).is_ok());
    }

    #[test]
    fn rational_millisecond_scaling_checks_overflow() {
        assert_eq!(
            checked_floor_milliseconds(i128::MAX, 1),
            Err(ClockError::ArithmeticOverflow)
        );
    }

    #[test]
    fn invalid_and_non_numeric_flags_are_refused_in_either_operand() {
        // CoreMedia flags: valid=1, rounded=2, +infinity=4, -infinity=8,
        // indefinite=16. A rounded finite value remains numeric.
        for flags in [0, 2, 1 | 4, 1 | 8, 1 | 16] {
            let invalid = CMTime {
                flags,
                ..t(1, 1, 0)
            };
            assert_eq!(
                checked_relative_milliseconds(invalid, t(2, 1, 0)),
                Err(ClockError::InvalidTime)
            );
            assert_eq!(
                checked_relative_milliseconds(t(0, 1, 0), invalid),
                Err(ClockError::InvalidTime)
            );
        }
        assert_eq!(
            checked_relative_milliseconds(
                t(0, 1, 0),
                CMTime {
                    flags: 3,
                    ..t(1, 1, 0)
                }
            ),
            Ok(1000)
        );
    }

    #[test]
    fn coremedia_host_clock_conversion_preserves_an_observed_timestamp() {
        // Actual Darwin FFI/link check, without screen capture. This verifies
        // the conversion ABI and host-clock identity, not SCK or movie origin.
        let host = CMClock::host_time_clock();
        let admission = CaptureAdmissionTime::now().unwrap();
        let observed = host.time();
        let mapped = map_sample_to_capture_clock(observed, &host, &host, admission).unwrap();
        assert_eq!(mapped.source_pts, ClockTimestamp::from(observed));
        assert_eq!(mapped.host_pts, ClockTimestamp::from(observed));
        assert!(
            mapped.capture_relative_ms
                <= checked_relative_milliseconds(admission.0, host.time()).unwrap()
        );
    }
}
