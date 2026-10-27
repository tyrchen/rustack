//! Explicit live, completion-relative and imported retention lifetimes.

use std::time::Duration;

use tokio::time::Instant;

use super::RegistryError;

#[derive(Debug, Clone, Copy)]
pub(super) enum Retention {
    Running,
    ClosedAt { at: Instant, ttl: Duration },
    Until(Instant),
}
impl Retention {
    pub(super) fn running(self) -> bool {
        matches!(self, Self::Running)
    }
    pub(super) fn expired(self, now: Instant) -> bool {
        match self {
            Self::Running => false,
            Self::ClosedAt { at, ttl } => now.saturating_duration_since(at) >= ttl,
            Self::Until(deadline) => now >= deadline,
        }
    }
    pub(super) fn remaining(self, now: Instant) -> Result<Duration, RegistryError> {
        match self {
            Self::Running => Err(RegistryError::Unavailable),
            Self::ClosedAt { at, ttl } => Ok(ttl.saturating_sub(
                now.checked_duration_since(at)
                    .ok_or(RegistryError::Validation)?,
            )),
            Self::Until(deadline) => Ok(deadline.saturating_duration_since(now)),
        }
    }
}
