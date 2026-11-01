//! Shared bounded data, deliberately separate from host-owned readiness receipts.

use std::{fmt, str::FromStr};

use getrandom::fill;
use serde::{Deserialize, Deserializer, Serialize, de::Error};
use serde_json::from_slice;
use tokio::time::Instant;
use url::Url;
use uuid::{Builder, Uuid};

use crate::{
    probe::{ProbeError, REPORT_BYTES},
    protocol::{Architecture, AttemptId},
};

/// Fixed admission-independent diagnostic route; no caller-selected path.
pub const GATEWAY_PROBE_PATH: &str = "/_rustack/runtime-probe";

/// Non-secret process identity; not authentication or a readiness capability.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[non_exhaustive]
pub struct GatewayInstanceId(Uuid);
impl fmt::Debug for GatewayInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GatewayInstanceId(<redacted>)")
    }
}
impl GatewayInstanceId {
    /// Generate a fresh identity with OS randomness, without a panic/fallback.
    ///
    /// This can block on OS entropy. Use a blocking owner or call
    /// [`Self::generate_owned`] from async startup.
    /// # Errors
    /// Returns a static entropy failure if the OS source is unavailable.
    /// ```
    /// let id=rustack_lambda_runtime::probe::GatewayInstanceId::generate()?;
    /// # Ok::<(),rustack_lambda_runtime::probe::ProbeError>(())
    /// ```
    pub fn generate() -> Result<Self, ProbeError> {
        let mut bytes = [0; 16];
        fill(&mut bytes).map_err(|_| ProbeError::Entropy)?;
        Ok(Self(Builder::from_random_bytes(bytes).into_uuid()))
    }
    /// Generate identity in bounded retained blocking work under one deadline.
    /// # Errors
    /// Rejects nonpositive/over60s budgets, queue expiry, entropy or worker failure.
    /// Abandoned observers never publish a late identity or release a running slot.
    /// ```no_run
    /// # async fn id()->Result<(),rustack_lambda_runtime::probe::ProbeError>{
    /// let id=rustack_lambda_runtime::probe::GatewayInstanceId::generate_owned(tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn generate_owned(deadline: Instant) -> Result<Self, ProbeError> {
        super::entropy::generate(deadline).await
    }
}
impl FromStr for GatewayInstanceId {
    type Err = ProbeError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        if raw.len() != 36 {
            return Err(ProbeError::Configuration);
        }
        let id = Uuid::parse_str(raw).map_err(|_| ProbeError::Configuration)?;
        if id.is_nil() || id.to_string() != raw {
            return Err(ProbeError::Configuration);
        }
        Ok(Self(id))
    }
}
impl TryFrom<String> for GatewayInstanceId {
    type Error = ProbeError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}
impl From<GatewayInstanceId> for String {
    fn from(id: GatewayInstanceId) -> Self {
        id.0.to_string()
    }
}
/// Validated literal-root operator endpoint; parsing performs no network IO.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[non_exhaustive]
pub struct GatewayEndpoint(String);
impl fmt::Debug for GatewayEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GatewayEndpoint(<redacted>)")
    }
}
impl FromStr for GatewayEndpoint {
    type Err = ProbeError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        if raw.is_empty()
            || raw.len() > 2048
            || raw.chars().any(char::is_control)
            || raw.bytes().any(|byte| byte.is_ascii_whitespace())
            || raw.contains('\\')
        {
            return Err(ProbeError::Configuration);
        }
        let rest = raw
            .split_once("://")
            .map(|(_, rest)| rest)
            .ok_or(ProbeError::Configuration)?;
        let authority = rest
            .split(['/', '?', '#'])
            .next()
            .ok_or(ProbeError::Configuration)?;
        let tail = rest
            .strip_prefix(authority)
            .ok_or(ProbeError::Configuration)?;
        if authority.contains('@') || !matches!(tail, "" | "/") {
            return Err(ProbeError::Configuration);
        }
        let url = Url::parse(raw).map_err(|_| ProbeError::Configuration)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.port() == Some(0)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || url.as_str().len() > 2048
        {
            return Err(ProbeError::Configuration);
        }
        Ok(Self(url.into()))
    }
}
impl TryFrom<String> for GatewayEndpoint {
    type Error = ProbeError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}
impl From<GatewayEndpoint> for String {
    fn from(endpoint: GatewayEndpoint) -> Self {
        endpoint.0
    }
}
impl GatewayEndpoint {
    /// Canonical operator origin, excluding any caller request path.
    /// ```
    /// let endpoint:rustack_lambda_runtime::probe::GatewayEndpoint="http://localhost:4566".parse()?;
    /// assert_eq!(endpoint.as_str(),"http://localhost:4566/");
    /// # Ok::<(),rustack_lambda_runtime::probe::ProbeError>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
fn version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    let version = u8::deserialize(deserializer)?;
    if version != 1 {
        return Err(D::Error::custom("invalid probe schema"));
    }
    Ok(version)
}
/// Intentional public diagnostic data, never a readiness receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct GatewayReply {
    #[serde(deserialize_with = "version")]
    version: u8,
    instance_id: GatewayInstanceId,
}
impl GatewayReply {
    /// Reply for the exact installed process identity.
    /// ```
    /// let reply=rustack_lambda_runtime::probe::GatewayReply::new(rustack_lambda_runtime::probe::GatewayInstanceId::generate()?);
    /// # Ok::<(),rustack_lambda_runtime::probe::ProbeError>(())
    /// ```
    #[must_use]
    pub fn new(instance_id: GatewayInstanceId) -> Self {
        Self {
            version: 1,
            instance_id,
        }
    }
    /// Observed independent process identity.
    /// ```
    /// let id=rustack_lambda_runtime::probe::GatewayInstanceId::generate()?;
    /// assert_eq!(rustack_lambda_runtime::probe::GatewayReply::new(id).instance_id(),id);
    /// # Ok::<(),rustack_lambda_runtime::probe::ProbeError>(())
    /// ```
    #[must_use]
    pub fn instance_id(&self) -> GatewayInstanceId {
        self.instance_id
    }
}
/// Private Runtime API challenge; expected identity is never sent in gateway GET.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct EndpointChallenge {
    #[serde(deserialize_with = "version")]
    version: u8,
    endpoint: GatewayEndpoint,
    instance_id: GatewayInstanceId,
}
impl EndpointChallenge {
    /// Capture already validated origin and expected instance.
    /// ```
    /// let challenge=rustack_lambda_runtime::probe::EndpointChallenge::new("http://localhost:4566".parse()?,rustack_lambda_runtime::probe::GatewayInstanceId::generate()?);
    /// # Ok::<(),rustack_lambda_runtime::probe::ProbeError>(())
    /// ```
    #[must_use]
    pub fn new(endpoint: GatewayEndpoint, instance_id: GatewayInstanceId) -> Self {
        Self {
            version: 1,
            endpoint,
            instance_id,
        }
    }
    pub(in crate::probe) fn decode(bytes: &[u8]) -> Result<Self, ProbeError> {
        if bytes.len() > REPORT_BYTES {
            return Err(ProbeError::Protocol);
        }
        from_slice(bytes).map_err(|_| ProbeError::Protocol)
    }
    pub(in crate::probe) fn endpoint(&self) -> &GatewayEndpoint {
        &self.endpoint
    }
    pub(in crate::probe) fn expected(&self) -> GatewayInstanceId {
        self.instance_id
    }
}
/// Untrusted report data; only the retained host owner can issue endpoint proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct EndpointReport {
    #[serde(deserialize_with = "version")]
    version: u8,
    attempt: AttemptId,
    architecture: Architecture,
    instance_id: GatewayInstanceId,
}
impl EndpointReport {
    pub(in crate::probe) fn observed(
        attempt: AttemptId,
        architecture: Architecture,
        instance_id: GatewayInstanceId,
    ) -> Self {
        Self {
            version: 1,
            attempt,
            architecture,
            instance_id,
        }
    }
    /// Consumed attempt to match against the host's actual invocation.
    /// ```no_run
    /// # fn inspect(r:&rustack_lambda_runtime::probe::EndpointReport){let attempt=r.attempt();}
    /// ```
    #[must_use]
    pub fn attempt(&self) -> AttemptId {
        self.attempt
    }
    /// Actual compiled probe architecture.
    /// ```no_run
    /// # fn inspect(r:&rustack_lambda_runtime::probe::EndpointReport){let architecture=r.architecture();}
    /// ```
    #[must_use]
    pub fn architecture(&self) -> Architecture {
        self.architecture
    }
    /// Independently observed gateway instance identity.
    /// ```no_run
    /// # fn inspect(r:&rustack_lambda_runtime::probe::EndpointReport){let id=r.instance_id();}
    /// ```
    #[must_use]
    pub fn instance_id(&self) -> GatewayInstanceId {
        self.instance_id
    }
}

#[cfg(test)]
mod tests;
