//! Checked wall/monotonic deadline conversion and local tracing metadata.

use std::{
    num::NonZeroU64,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use thiserror::Error;
use tokio::time::Instant;
use uuid::Uuid;

use crate::protocol::TraceId;

/// Clock/metadata failures contain no invocation contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum MetadataError {
    /// Supplied deadline has already elapsed.
    #[error("runtime deadline expired")]
    Expired,
    /// Clock/range/conversion cannot represent the metadata.
    #[error("invalid runtime clock metadata")]
    Invalid,
}

/// Convert a future monotonic deadline into checked Unix milliseconds.
///
/// # Errors
/// Rejects expired deadlines, a pre-epoch wall clock or arithmetic overflow.
///
/// ```
/// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
/// assert!(rustack_lambda_runtime::metadata::deadline_unix_ms(deadline).is_ok());
/// ```
pub fn deadline_unix_ms(deadline: Instant) -> Result<NonZeroU64, MetadataError> {
    to_wall(deadline, Instant::now(), SystemTime::now())
}

fn to_wall(deadline: Instant, now: Instant, wall: SystemTime) -> Result<NonZeroU64, MetadataError> {
    let remaining = deadline
        .checked_duration_since(now)
        .filter(|remaining| !remaining.is_zero())
        .ok_or(MetadataError::Expired)?;
    let epoch = wall
        .duration_since(UNIX_EPOCH)
        .map_err(|_| MetadataError::Invalid)?;
    let total = epoch.checked_add(remaining).ok_or(MetadataError::Invalid)?;
    NonZeroU64::new(u64::try_from(total.as_millis()).map_err(|_| MetadataError::Invalid)?)
        .ok_or(MetadataError::Invalid)
}

/// Translate an incoming wall deadline into a bounded monotonic deadline.
///
/// # Errors
/// Rejects expired deadlines, invalid clocks and more than one hour remaining.
/// Lambda's≤900s handler plus bounded startup fits within this local ceiling.
///
/// ```
/// # use rustack_lambda_runtime::metadata::{deadline_unix_ms, deadline_from_unix_ms};
/// let value = deadline_unix_ms(tokio::time::Instant::now() + std::time::Duration::from_secs(5))?;
/// assert!(deadline_from_unix_ms(value).is_ok());
/// # Ok::<(), rustack_lambda_runtime::metadata::MetadataError>(())
/// ```
pub fn deadline_from_unix_ms(value: NonZeroU64) -> Result<Instant, MetadataError> {
    from_wall(value, Instant::now(), SystemTime::now())
}

fn from_wall(value: NonZeroU64, now: Instant, wall: SystemTime) -> Result<Instant, MetadataError> {
    let epoch = wall
        .duration_since(UNIX_EPOCH)
        .map_err(|_| MetadataError::Invalid)?;
    let remaining = Duration::from_millis(value.get())
        .checked_sub(epoch)
        .filter(|remaining| !remaining.is_zero())
        .ok_or(MetadataError::Expired)?;
    if remaining > Duration::from_hours(1) {
        return Err(MetadataError::Invalid);
    }
    now.checked_add(remaining).ok_or(MetadataError::Invalid)
}

/// Generate an X-Ray-shaped local trace, without claiming upstream sampling.
///
/// # Errors
/// Returns Invalid if the wall clock/validated header cannot be represented.
///
/// ```
/// assert!(rustack_lambda_runtime::metadata::local_trace_id()?.as_ref().ends_with("Sampled=0"));
/// # Ok::<(), rustack_lambda_runtime::metadata::MetadataError>(())
/// ```
pub fn local_trace_id() -> Result<TraceId, MetadataError> {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| MetadataError::Invalid)?
        .as_secs()
        % (1_u64 << 32);
    let root = Uuid::new_v4().simple().to_string();
    let parent = Uuid::new_v4().simple().to_string();
    let root = root.get(..24).ok_or(MetadataError::Invalid)?;
    let parent = parent.get(..16).ok_or(MetadataError::Invalid)?;
    format!("Root=1-{epoch:08x}-{root};Parent={parent};Sampled=0")
        .parse()
        .map_err(|_| MetadataError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_convert_checked_fractional_deadlines_and_reject_invalid_clocks() {
        let now = Instant::now();
        let wall = UNIX_EPOCH + Duration::from_millis(1000);
        let deadline = now + Duration::from_micros(2500);
        assert_eq!(to_wall(deadline, now, wall).unwrap().get(), 1002);
        assert_eq!(
            from_wall(NonZeroU64::new(1002).unwrap(), now, wall).unwrap(),
            now + Duration::from_millis(2)
        );
        assert_eq!(to_wall(now, now, wall), Err(MetadataError::Expired));
        assert_eq!(
            to_wall(deadline, now, UNIX_EPOCH - Duration::from_secs(1)),
            Err(MetadataError::Invalid)
        );
        assert_eq!(
            from_wall(NonZeroU64::new(1000).unwrap(), now, wall),
            Err(MetadataError::Expired)
        );
        assert_eq!(
            from_wall(NonZeroU64::new(u64::MAX).unwrap(), now, wall),
            Err(MetadataError::Invalid)
        );
    }
}
