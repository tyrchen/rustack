//! Typed bounded kernel observations; qualification belongs to the host owner.

#[cfg(unix)]
mod client;
#[cfg(unix)]
mod collect;
#[cfg(any(unix, test))]
mod parse;

use std::{fmt, io, sync::Arc};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use typed_builder::TypedBuilder;

use crate::protocol::{Architecture, AttemptId};

/// Maximum encoded resource report/challenge response size.
pub const REPORT_BYTES: usize = 4096;

/// Static probe errors never display kernel text or filesystem paths.
#[derive(Clone, Error)]
#[non_exhaustive]
pub enum ProbeError {
    /// Unsupported platform, arguments or private API configuration.
    #[error("invalid runtime probe configuration")]
    Configuration,
    /// Incomplete, duplicate, malformed or unsupported kernel observations.
    #[error("invalid runtime probe observation")]
    Observation,
    /// The private API protocol/challenge did not match.
    #[error("invalid runtime probe protocol")]
    Protocol,
    /// Absolute collection/request budget expired.
    #[error("runtime probe deadline exceeded")]
    Deadline,
    /// Fixed local kernel/API IO failed.
    #[error("runtime probe IO failed")]
    Io(#[source] Arc<io::Error>),
    /// Private Runtime API HTTP failed.
    #[error("runtime probe HTTP failed")]
    Http(#[source] Arc<hyper::Error>),
    /// Typed report serialization failed.
    #[error("runtime probe report failed")]
    Report(#[source] Arc<serde_json::Error>),
}
impl fmt::Debug for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl From<io::Error> for ProbeError {
    fn from(error: io::Error) -> Self {
        Self::Io(Arc::new(error))
    }
}
impl From<hyper::Error> for ProbeError {
    fn from(error: hyper::Error) -> Self {
        Self::Http(Arc::new(error))
    }
}

/// Observed container-local temporary filesystem, without mount source text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TypedBuilder)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct TemporaryReport {
    /// Kernel-reported tmpfs size in bytes.
    pub bytes: u64,
    /// Actual permission/special mode bits.
    pub mode: u32,
    /// The mount filesystem is tmpfs.
    pub tmpfs: bool,
    /// The effective mount disallows setuid transitions.
    pub nosuid: bool,
    /// The effective mount disallows device interpretation.
    pub nodev: bool,
}

/// Finite/unlimited normalized controller and process observations.
///
/// A deserialized report is untrusted data, never a readiness capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TypedBuilder)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct ResourceReport {
    /// Report schema, currently1.
    pub version: u8,
    /// Consumed Runtime API attempt identity.
    pub attempt: AttemptId,
    /// Actual compiled Linux architecture.
    pub architecture: Architecture,
    /// Required controller memberships are rooted in this private namespace.
    pub cgroup_root: bool,
    /// Observed memory controller value; textual unlimited becomes None. Numeric
    /// v1 unlimited sentinels retain their numeric value and cannot qualify.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_max: Option<u64>,
    /// Observed extra swap cap; textual unlimited becomes None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_max: Option<u64>,
    /// Process/thread hard cap; None means unlimited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pids_max: Option<u64>,
    /// CPU time quota in microseconds; None means unlimited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_quota: Option<u64>,
    /// CPU scheduling period in microseconds.
    pub cpu_period: u64,
    /// Effective UID observed from proc status.
    pub euid: u32,
    /// Effective GID observed from proc status.
    pub egid: u32,
    /// Effective Linux capability bitmask.
    pub cap_effective: u64,
    /// Actual `no_new_privs` flag.
    pub no_new_privileges: bool,
    /// Actual seccomp mode (2 means filter mode).
    pub seccomp_mode: u8,
    /// Root's effective mount flags include read-only.
    pub root_readonly: bool,
    /// Actual /tmp mount observation.
    pub temporary: TemporaryReport,
}

/// Run one private Linux probe challenge, then wait for host-owned retirement.
///
/// # Errors
/// Rejects non-Linux execution, any endpoint other than fixed loopback9001,
/// malformed challenges, expired budgets and failed kernel/API observations.
///
/// ```no_run
/// # #[cfg(unix)] async fn probe() {
/// let outcome = rustack_lambda_runtime::probe::run().await;
/// # }
/// ```
#[cfg(unix)]
pub async fn run() -> Result<(), ProbeError> {
    client::run().await
}
