//! Immutable operator scope and request provenance; no serialized trust claims.

use std::{fmt, num::NonZeroU64, sync::Arc, time::Duration};

use rustack_auth::{AuthMode, CredentialProvider};
use rustack_core::http::BodyBudget;
use tokio::time::Instant;
use uuid::Uuid;

use crate::HttpError;

/// Validated partition/region/account configured by the runtime, not the caller.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RequestScope {
    partition: String,
    region: String,
    account: String,
}
impl fmt::Debug for RequestScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RequestScope(<redacted>)")
    }
}
impl RequestScope {
    /// Validate an operator scope for shared HTTP/core use.
    ///
    /// # Errors
    /// Rejects unsupported partitions, invalid region identifiers or non12-digit accounts.
    ///
    /// ```
    /// let scope = rustack_stepfunctions_http::RequestScope::new("aws".into(), "us-east-1".into(), "000000000000".into())?;
    /// assert_eq!(scope.region(), "us-east-1");
    /// # Ok::<(), rustack_stepfunctions_http::HttpError>(())
    /// ```
    pub fn new(partition: String, region: String, account: String) -> Result<Self, HttpError> {
        if !matches!(partition.as_str(), "aws" | "aws-cn" | "aws-us-gov")
            || !(1..=64).contains(&region.len())
            || !region
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_lowercase)
            || !region
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
            || !region
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || account.len() != 12
            || !account.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(HttpError::Configuration);
        }
        Ok(Self {
            partition,
            region,
            account,
        })
    }
    /// Configured AWS partition.
    #[must_use]
    pub fn partition(&self) -> &str {
        &self.partition
    }
    /// Configured AWS region.
    #[must_use]
    pub fn region(&self) -> &str {
        &self.region
    }
    /// Configured account identifier.
    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }
}

/// Validated, explicitly selected authentication/body/deadline configuration.
#[derive(Clone)]
#[non_exhaustive]
pub struct StepFunctionsHttpConfig {
    pub(crate) scope: Arc<RequestScope>,
    pub(crate) skip: bool,
    pub(crate) credentials: Option<Arc<dyn CredentialProvider>>,
    pub(crate) body: BodyBudget,
    pub(crate) total: Duration,
}
impl fmt::Debug for StepFunctionsHttpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StepFunctionsHttpConfig(<redacted>)")
    }
}
impl StepFunctionsHttpConfig {
    /// Construct explicit local-development or strict authentication policy.
    ///
    /// # Errors
    /// Strict mode requires a credential provider; scope and budgets must be valid.
    ///
    /// ```
    /// use rustack_stepfunctions_http::{RequestScope, StepFunctionsHttpConfig};
    /// let scope = RequestScope::new("aws".into(), "us-east-1".into(), "000000000000".into())?;
    /// let config = StepFunctionsHttpConfig::new(scope, true, None)?;
    /// # Ok::<(), rustack_stepfunctions_http::HttpError>(())
    /// ```
    pub fn new(
        scope: RequestScope,
        skip_signature_validation: bool,
        credentials: Option<Arc<dyn CredentialProvider>>,
    ) -> Result<Self, HttpError> {
        AuthMode::resolve(skip_signature_validation, credentials.as_deref())
            .map_err(|_| HttpError::Configuration)?;
        let cap = NonZeroU64::new(2 * 1024 * 1024).ok_or(HttpError::Configuration)?;
        Ok(Self {
            scope: Arc::new(scope),
            skip: skip_signature_validation,
            credentials,
            body: BodyBudget::control().capped(cap),
            total: Duration::from_secs(rustack_core::settings::budgets().request_seconds),
        })
    }
    /// Apply validated operator budgets without relaxing the2MiB request limit.
    ///
    /// # Errors
    /// Total request duration must be positive and at most one hour.
    ///
    /// ```
    /// # fn budget(config: rustack_stepfunctions_http::StepFunctionsHttpConfig) -> Result<(), rustack_stepfunctions_http::HttpError> {
    /// let config = config.with_budgets(rustack_core::http::BodyBudget::control(), std::time::Duration::from_secs(30))?;
    /// # Ok(()) }
    /// ```
    pub fn with_budgets(mut self, body: BodyBudget, total: Duration) -> Result<Self, HttpError> {
        if total.is_zero() || total > Duration::from_secs(3600) {
            return Err(HttpError::Configuration);
        }
        self.body = body.capped(NonZeroU64::new(2 * 1024 * 1024).ok_or(HttpError::Configuration)?);
        self.total = total;
        Ok(self)
    }
}

/// Per-call deadline/provenance. Dropping this value does not cancel accepted work.
#[derive(Clone)]
#[non_exhaustive]
pub struct RequestContext {
    pub(crate) scope: Arc<RequestScope>,
    pub(crate) id: Uuid,
    pub(crate) deadline: Instant,
    pub(crate) authenticated: bool,
}
impl fmt::Debug for RequestContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestContext")
            .field("request_id", &self.id)
            .field("authenticated", &self.authenticated)
            .finish_non_exhaustive()
    }
}
impl RequestContext {
    /// Trusted operator scope shared with the core adapter.
    #[must_use]
    pub fn scope(&self) -> &RequestScope {
        &self.scope
    }
    /// Generated request correlation ID.
    #[must_use]
    pub fn request_id(&self) -> Uuid {
        self.id
    }
    /// Absolute monotonic caller deadline, not the workflow's lifetime.
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    /// Whether this request passed strict signature verification.
    #[must_use]
    pub fn authenticated(&self) -> bool {
        self.authenticated
    }
}
