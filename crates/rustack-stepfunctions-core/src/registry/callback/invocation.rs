//! Unique real-invocation reporting and bounded metadata, separate from cleanup.

use std::{fmt, num::NonZeroU64, sync::Arc};

use serde_json::json;
use tokio::{sync::watch, time::Instant};
use uuid::{Uuid, Variant};

use super::{CallbackError, Control};
use crate::{AslError, FrameCallId, Payload, TaskFailure, TenantScope, registry::domain};

/// Safe synchronous invocation metadata; never stores the function business body.
#[derive(Clone)]
#[non_exhaustive]
pub struct CallbackInvocationMetadata {
    request_id: Uuid,
    version: Version,
}
#[derive(Debug, Clone, Copy)]
enum Version {
    Latest,
    Published(NonZeroU64),
}
impl fmt::Debug for CallbackInvocationMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackInvocationMetadata(<redacted>)")
    }
}
impl CallbackInvocationMetadata {
    /// Capture an actual version4 request ID and actual executed version.
    ///
    /// # Errors
    /// Rejects non-v4 IDs and versions other than `$LATEST` or canonical positive
    /// u64 decimal versions. It never substitutes invented metadata.
    ///
    /// ```
    /// # fn capture(id:uuid::Uuid){let metadata=rustack_stepfunctions_core::CallbackInvocationMetadata::new(id,"$LATEST");}
    /// ```
    pub fn new(request_id: Uuid, executed_version: &str) -> Result<Self, CallbackError> {
        if request_id.get_version_num() != 4 || request_id.get_variant() != Variant::RFC4122 {
            return Err(CallbackError::Validation);
        }
        let version = if executed_version == "$LATEST" {
            Version::Latest
        } else {
            if executed_version.is_empty()
                || executed_version.len() > 20
                || executed_version.starts_with('0')
                || !executed_version.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(CallbackError::Validation);
            }
            Version::Published(
                executed_version
                    .parse()
                    .map_err(|_| CallbackError::Validation)?,
            )
        };
        Ok(Self {
            request_id,
            version,
        })
    }
    /// Read the actual invocation identity.
    ///
    /// ```
    /// # fn inspect(m:&rustack_stepfunctions_core::CallbackInvocationMetadata){let id=m.request_id();}
    /// ```
    #[must_use]
    pub fn request_id(&self) -> Uuid {
        self.request_id
    }
    /// Read the actual canonical executed version.
    ///
    /// ```
    /// # fn inspect(m:&rustack_stepfunctions_core::CallbackInvocationMetadata){let version=m.executed_version();}
    /// ```
    #[must_use]
    pub fn executed_version(&self) -> String {
        match self.version {
            Version::Latest => "$LATEST".into(),
            Version::Published(version) => version.to_string(),
        }
    }
    /// Materialize the bounded metadata-only optimized invocation envelope.
    ///
    /// # Errors
    /// Preserves typed bounded JSON/encoding failure without a business Payload.
    ///
    /// ```
    /// # fn encode(m:&rustack_stepfunctions_core::CallbackInvocationMetadata){let output=m.payload();}
    /// ```
    pub fn payload(&self) -> Result<Payload, AslError> {
        Payload::from_value(
            &json!({"StatusCode":200,"ExecutedVersion":self.executed_version(),"SdkHttpMetadata":{"HttpStatusCode":200},"SdkResponseMetadata":{"RequestId":self.request_id.to_string()}}),
        )
    }
}
/// One immutable actual return observation, independent of external callback result.
#[derive(Clone)]
#[non_exhaustive]
pub struct CallbackInvocation {
    pub(super) outcome: Result<Option<CallbackInvocationMetadata>, TaskFailure>,
    pub(super) observed: Instant,
}
impl fmt::Debug for CallbackInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackInvocation(<redacted>)")
    }
}
impl CallbackInvocation {
    /// Read the captured monotonic actual-return observation.
    ///
    /// ```
    /// # fn inspect(r:&rustack_stepfunctions_core::CallbackInvocation){let observed=r.observed();}
    /// ```
    #[must_use]
    pub fn observed(&self) -> Instant {
        self.observed
    }
    /// Whether the synchronous invocation succeeded, regardless of callback state.
    ///
    /// ```
    /// # fn inspect(r:&rustack_stepfunctions_core::CallbackInvocation){let success=r.succeeded();}
    /// ```
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.outcome.is_ok()
    }
    /// Read actual metadata; direct foundation unit reports may omit it.
    ///
    /// ```
    /// # fn inspect(r:&rustack_stepfunctions_core::CallbackInvocation){let metadata=r.metadata();}
    /// ```
    #[must_use]
    pub fn metadata(&self) -> Option<&CallbackInvocationMetadata> {
        self.outcome.as_ref().ok().and_then(Option::as_ref)
    }
    /// Read a bounded synchronous invocation failure, if any.
    ///
    /// ```
    /// # fn inspect(r:&rustack_stepfunctions_core::CallbackInvocation){let failure=r.failure();}
    /// ```
    #[must_use]
    pub fn failure(&self) -> Option<&TaskFailure> {
        self.outcome.as_ref().err()
    }
}
#[derive(Debug)]
pub(super) struct InvocationIdentity {
    pub(super) scope: TenantScope,
    pub(super) call: FrameCallId,
    pub(super) deadline: Instant,
    pub(super) issued: Instant,
}
/// Noncloneable completion capability transferred to the real invocation owner.
#[derive(Debug)]
#[non_exhaustive]
pub struct CallbackInvocationReporter {
    identity: InvocationIdentity,
    control: Arc<Control>,
    invocation: watch::Sender<Option<Arc<CallbackInvocation>>>,
    pub(super) reported: bool,
}
impl CallbackInvocationReporter {
    pub(super) fn new(
        identity: InvocationIdentity,
        control: Arc<Control>,
        invocation: watch::Sender<Option<Arc<CallbackInvocation>>>,
    ) -> Self {
        Self {
            identity,
            control,
            invocation,
            reported: false,
        }
    }
    pub(crate) fn bind(
        &self,
        scope: &TenantScope,
        call: FrameCallId,
    ) -> Result<Instant, CallbackError> {
        self.check_live()?;
        if !domain::same_scope(&self.identity.scope, scope) || self.identity.call != call {
            return Err(CallbackError::Validation);
        }
        Ok(self.identity.deadline)
    }
    /// Refresh report/deadline/revocation control before real invocation issuance.
    ///
    /// # Errors
    /// Rejects duplicate progress and expired/revoked ownership.
    ///
    /// ```
    /// # fn check(r:&rustack_stepfunctions_core::CallbackInvocationReporter){let result=r.check_live();}
    /// ```
    pub fn check_live(&self) -> Result<(), CallbackError> {
        if self.reported {
            return Err(CallbackError::Progress);
        }
        if self.control.revoked() || Instant::now() >= self.identity.deadline {
            return Err(CallbackError::TaskTimedOut);
        }
        Ok(())
    }
    /// Publish one captured actual return before physical cleanup is observed.
    ///
    /// # Errors
    /// Rejects duplicate/closed/expired reports and observations before token
    /// registration or in the future. Invalid reports never reactivate a token.
    ///
    /// ```
    /// # fn report(r:&mut rustack_stepfunctions_core::CallbackInvocationReporter,m:rustack_stepfunctions_core::CallbackInvocationMetadata){let result=r.complete_at(Ok(m),tokio::time::Instant::now());}
    /// ```
    pub fn complete_at(
        &mut self,
        result: Result<CallbackInvocationMetadata, TaskFailure>,
        observed: Instant,
    ) -> Result<(), CallbackError> {
        self.publish(result.map(Some), observed)
    }
    pub(super) fn complete_unit(
        &mut self,
        result: Result<(), TaskFailure>,
    ) -> Result<(), CallbackError> {
        self.publish(result.map(|()| None), Instant::now())
    }
    fn publish(
        &mut self,
        outcome: Result<Option<CallbackInvocationMetadata>, TaskFailure>,
        observed: Instant,
    ) -> Result<(), CallbackError> {
        self.check_live()?;
        let now = Instant::now();
        if observed < self.identity.issued || observed > now {
            return Err(CallbackError::Validation);
        }
        // The post-validation fence also covers time spent validating metadata.
        if now >= self.identity.deadline || self.control.revoked() {
            return Err(CallbackError::TaskTimedOut);
        }
        self.reported = true;
        self.invocation
            .send_replace(Some(Arc::new(CallbackInvocation { outcome, observed })));
        self.control.wake.notify_one();
        Ok(())
    }
}
impl Drop for CallbackInvocationReporter {
    fn drop(&mut self) {
        if !self.reported {
            self.control.revoke();
        }
    }
}
