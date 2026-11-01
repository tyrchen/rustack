//! Explicit local handler credentials, qualified affiliation and bounded environment.
//!
//! Only private startup qualification issues contexts. Context identity separates
//! warm plans; the retained proof and exact scope, not that identity, authorize use.

use std::{
    collections::BTreeMap,
    fmt,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use secrecy::{ExposeSecret, SecretString};

use super::{
    ContainerPolicy, DirectoryLease, DockerError, DockerExecutorConfig, DockerGatewayEndpoint,
    DockerReadiness, EnvironmentPort,
};
use crate::artifact::PinnedImage;

const MAX_AUGMENTATION_BYTES: usize = 4096;
const MAX_ENV_ENTRIES: usize = 320;
const MAX_ENV_BYTES: usize = 80 * 1024;
const MAX_ENV_ENTRY_BYTES: usize = 4097;
const UNSET: [&str; 16] = [
    "AWS_PROFILE",
    "AWS_DEFAULT_PROFILE",
    "AWS_SDK_LOAD_CONFIG",
    "AWS_SESSION_TOKEN",
    "AWS_SECURITY_TOKEN",
    "AWS_ACCESS_KEY",
    "AWS_SECRET_KEY",
    "AWS_WEB_IDENTITY_TOKEN_FILE",
    "AWS_ROLE_ARN",
    "AWS_ROLE_SESSION_NAME",
    "AWS_CONTAINER_CREDENTIALS_FULL_URI",
    "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    "AWS_CONTAINER_AUTHORIZATION_TOKEN",
    "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE",
    "AWS_EC2_METADATA_SERVICE_ENDPOINT",
    "AWS_EC2_METADATA_SERVICE_ENDPOINT_MODE",
];
static NEXT_CONTEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(super) struct LocalSdkCredentials {
    access: SecretString,
    secret: SecretString,
}
impl fmt::Debug for LocalSdkCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalSdkCredentials(<redacted>)")
    }
}
impl TryFrom<(String, String)> for LocalSdkCredentials {
    type Error = DockerError;
    fn try_from((access, secret): (String, String)) -> Result<Self, Self::Error> {
        if !(1..=128).contains(&access.len())
            || !access
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || !(1..=256).contains(&secret.len())
            || !secret.bytes().all(|b| (33..=126).contains(&b))
        {
            return Err(DockerError::Configuration);
        }
        Ok(Self {
            access: access.into(),
            secret: secret.into(),
        })
    }
}
impl LocalSdkCredentials {
    pub(super) fn parse(
        access: Option<String>,
        secret: Option<String>,
    ) -> Result<Option<Self>, DockerError> {
        match (access, secret) {
            (None, None) => Ok(None),
            (Some(access), Some(secret)) => (access, secret).try_into().map(Some),
            _ => Err(DockerError::Configuration),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct SdkAffiliation {
    directory: DirectoryLease,
    relay: PinnedImage,
    policy: ContainerPolicy,
    environments: EnvironmentPort,
    readiness: Arc<dyn DockerReadiness>,
}
impl SdkAffiliation {
    pub(super) fn new(
        directory: DirectoryLease,
        relay: PinnedImage,
        policy: ContainerPolicy,
        environments: EnvironmentPort,
        readiness: Arc<dyn DockerReadiness>,
    ) -> Self {
        Self {
            directory,
            relay,
            policy,
            environments,
            readiness,
        }
    }
    pub(super) fn live(&self) -> Result<(), DockerError> {
        self.readiness.check().map_err(|_| DockerError::Stopped)
    }
    fn matches(&self, config: &DockerExecutorConfig) -> bool {
        let expected = self.relay.resolved().metadata();
        let actual = config.relay.resolved().metadata();
        Arc::ptr_eq(&self.readiness, &config.readiness)
            && self.directory.same_instance(&config.directory)
            && self.environments.same_service(&config.environments)
            && self.policy == config.policy
            && expected.image_id == actual.image_id
            && expected.inspection_hash == actual.inspection_hash
            && self.relay.is_live()
            && config.relay.is_live()
            && self.readiness.check().is_ok()
    }
}

/// Opaque qualified local SDK environment; credentials never enter saved revisions.
///
/// Only the private startup issuer can construct this context from actual proof.
/// Clones retain the same nonsecret warm identity and live qualification holder.
///
/// ```compile_fail
/// let _ = serde_json::from_str::<rustack_lambda_core::docker::DockerSdkEnvironment>("{}");
/// ```
#[derive(Clone)]
#[non_exhaustive]
pub struct DockerSdkEnvironment {
    credentials: LocalSdkCredentials,
    endpoint: DockerGatewayEndpoint,
    affiliation: SdkAffiliation,
    identity: NonZeroU64,
}
impl fmt::Debug for DockerSdkEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DockerSdkEnvironment(<redacted>)")
    }
}
impl DockerSdkEnvironment {
    pub(super) fn new(
        credentials: LocalSdkCredentials,
        endpoint: DockerGatewayEndpoint,
        affiliation: SdkAffiliation,
    ) -> Result<Self, DockerError> {
        affiliation
            .readiness
            .check()
            .map_err(|_| DockerError::Stopped)?;
        Ok(Self {
            credentials,
            endpoint,
            affiliation,
            identity: next_identity(&NEXT_CONTEXT)?,
        })
    }
    pub(super) fn matches(&self, config: &DockerExecutorConfig) -> bool {
        self.affiliation.matches(config)
    }
    pub(super) fn liveness(&self) -> SdkAffiliation {
        self.affiliation.clone()
    }
    pub(super) fn identity(&self) -> NonZeroU64 {
        self.identity
    }
    pub(super) fn environment(&self, entries: Vec<String>) -> Result<Vec<String>, DockerError> {
        environment(&self.credentials, &self.endpoint, entries)
    }
}

fn next_identity(counter: &AtomicU64) -> Result<NonZeroU64, DockerError> {
    let value = counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            NonZeroU64::new(value).and_then(|_| value.checked_add(1))
        })
        .map_err(|_| DockerError::Configuration)?;
    NonZeroU64::new(value).ok_or(DockerError::Configuration)
}

fn environment(
    credentials: &LocalSdkCredentials,
    endpoint: &DockerGatewayEndpoint,
    entries: Vec<String>,
) -> Result<Vec<String>, DockerError> {
    if entries.len() > MAX_ENV_ENTRIES {
        return Err(DockerError::Configuration);
    }
    let mut values = BTreeMap::new();
    let mut input_bytes = 0usize;
    for entry in entries {
        input_bytes = add_bytes(input_bytes, entry.len(), MAX_ENV_BYTES)?;
        if entry.len() > MAX_ENV_ENTRY_BYTES || entry.contains('\0') {
            return Err(DockerError::Configuration);
        }
        let (key, value) = entry
            .split_once('=')
            .map_or((entry.as_str(), None), |(k, v)| (k, Some(v.to_owned())));
        values.insert(key.to_owned(), value);
    }
    for (key, value) in &mut values {
        if key.starts_with("AWS_ENDPOINT_URL_") || UNSET.contains(&key.as_str()) {
            *value = None;
        }
    }
    let fixed = [
        ("AWS_ACCESS_KEY_ID", credentials.access.expose_secret()),
        ("AWS_SECRET_ACCESS_KEY", credentials.secret.expose_secret()),
        ("AWS_ENDPOINT_URL", endpoint.as_str()),
        ("AWS_IGNORE_CONFIGURED_ENDPOINT_URLS", "false"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
        ("AWS_CONFIG_FILE", "/dev/null"),
        ("AWS_SHARED_CREDENTIALS_FILE", "/dev/null"),
    ];
    let mut augmentation = 0usize;
    for key in UNSET {
        augmentation = add_bytes(augmentation, key.len(), MAX_AUGMENTATION_BYTES)?;
        values.insert(key.to_owned(), None);
    }
    for (key, value) in fixed {
        let length = key
            .len()
            .checked_add(1)
            .and_then(|n| n.checked_add(value.len()))
            .ok_or(DockerError::Configuration)?;
        augmentation = add_bytes(augmentation, length, MAX_AUGMENTATION_BYTES)?;
        values.insert(key.to_owned(), Some(value.to_owned()));
    }
    if values.len() > MAX_ENV_ENTRIES {
        return Err(DockerError::Configuration);
    }
    let mut output = Vec::with_capacity(values.len());
    let mut bytes = 0usize;
    for (key, value) in values {
        let length = match &value {
            Some(value) => key
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_add(value.len()))
                .ok_or(DockerError::Configuration)?,
            None => key.len(),
        };
        if length > MAX_ENV_ENTRY_BYTES {
            return Err(DockerError::Configuration);
        }
        bytes = add_bytes(bytes, length, MAX_ENV_BYTES)?;
        output.push(match value {
            Some(value) => format!("{key}={value}"),
            None => key,
        });
    }
    Ok(output)
}
fn add_bytes(current: usize, extra: usize, maximum: usize) -> Result<usize, DockerError> {
    current
        .checked_add(extra)
        .filter(|value| *value <= maximum)
        .ok_or(DockerError::Configuration)
}

#[cfg(test)]
mod tests;
