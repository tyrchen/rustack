//! Explicit registry policy, secret credentials and retained progress requests.

pub(super) mod progress;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU16,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use base64::{Engine, engine::general_purpose::URL_SAFE};
use bytes::Bytes;
use http::{Method, StatusCode};
use secrecy::{ExposeSecret, SecretString, zeroize::Zeroize};
use serde_json::to_vec;
use tokio::{sync::watch, time::Instant};

use super::{DockerError, DockerPort, Operation, RawReply, RawRequest};
use crate::artifact::{Architecture, ImageReference};

/// Canonical exact registry host/port; debug omits operator infrastructure.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct RegistryHost(String);
impl fmt::Debug for RegistryHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryHost(<redacted>)")
    }
}
impl FromStr for RegistryHost {
    type Err = DockerError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.len() > 253
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
        {
            return Err(DockerError::Configuration);
        }
        let canonical = value.to_ascii_lowercase();
        let canonical = if canonical == "index.docker.io" {
            "docker.io".to_owned()
        } else {
            canonical
        };
        let port = if canonical.starts_with('[') {
            canonical
                .split_once(']')
                .and_then(|(_, tail)| tail.strip_prefix(':'))
        } else {
            canonical.split_once(':').map(|(_, port)| port)
        };
        if port.is_some_and(|port| port.parse::<NonZeroU16>().is_err()) {
            return Err(DockerError::Configuration);
        }
        if !canonical.starts_with('[') {
            let name = canonical
                .split_once(':')
                .map_or(canonical.as_str(), |(host, _)| host);
            if name.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || !label
                        .bytes()
                        .next()
                        .is_some_and(|b| b.is_ascii_alphanumeric())
                    || !label
                        .bytes()
                        .last()
                        .is_some_and(|b| b.is_ascii_alphanumeric())
            }) {
                return Err(DockerError::Configuration);
            }
        }
        let reference: ImageReference = format!("{canonical}/a")
            .parse()
            .map_err(|_| DockerError::Configuration)?;
        if reference.registry() != Some(canonical.as_str()) {
            return Err(DockerError::Configuration);
        }
        Ok(Self(canonical))
    }
}
impl TryFrom<&str> for RegistryHost {
    type Error = DockerError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl RegistryHost {
    /// Canonical host, excluding any scheme/path.
    /// ```
    /// let host: rustack_lambda_core::docker::RegistryHost="registry.example:5000".parse()?;
    /// assert_eq!(host.as_str(),"registry.example:5000");
    /// # Ok::<(),rustack_lambda_core::docker::DockerError>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

enum Credential {
    Basic(SecretString, SecretString),
    Token(SecretString),
}
/// Validated mutually exclusive registry credentials, never serializable publicly.
#[non_exhaustive]
pub struct RegistryCredential(Credential);
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}
impl fmt::Debug for RegistryCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryCredential(<redacted>)")
    }
}
fn secret(value: &SecretString, max: usize) -> Result<(), DockerError> {
    let value = value.expose_secret();
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(DockerError::Configuration);
    }
    Ok(())
}
impl RegistryCredential {
    /// Validate explicit basic credentials without trimming or discovery.
    /// # Errors
    /// Rejects empty, overlong or control-bearing secrets.
    /// ```
    /// let credentials=rustack_lambda_core::docker::RegistryCredential::basic("user".into(),"password".into())?;
    /// # Ok::<(),rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn basic(username: SecretString, password: SecretString) -> Result<Self, DockerError> {
        secret(&username, 256)?;
        secret(&password, 4096)?;
        Ok(Self(Credential::Basic(username, password)))
    }
    /// Validate an explicit identity token.
    /// # Errors
    /// Rejects empty, overlong or control-bearing tokens.
    /// ```
    /// let credentials=rustack_lambda_core::docker::RegistryCredential::identity_token("token".into())?;
    /// # Ok::<(),rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn identity_token(token: SecretString) -> Result<Self, DockerError> {
        secret(&token, 8192)?;
        Ok(Self(Credential::Token(token)))
    }
    fn bytes(&self) -> usize {
        match &self.0 {
            Credential::Basic(user, password) => user
                .expose_secret()
                .len()
                .saturating_add(password.expose_secret().len()),
            Credential::Token(token) => token.expose_secret().len(),
        }
    }
    fn header(&self, host: &RegistryHost) -> Result<SecretString, DockerError> {
        let mut fields = BTreeMap::from([("serveraddress", host.as_str())]);
        match &self.0 {
            Credential::Basic(user, password) => {
                fields.insert("username", user.expose_secret());
                fields.insert("password", password.expose_secret());
            }
            Credential::Token(token) => {
                fields.insert("identitytoken", token.expose_secret());
            }
        }
        let mut bytes = to_vec(&fields).map_err(|_| DockerError::Schema)?;
        let encoded: SecretString = URL_SAFE.encode(&bytes).into();
        bytes.zeroize();
        if encoded.expose_secret().len() > 32 * 1024 {
            return Err(DockerError::TooLarge);
        }
        Ok(encoded)
    }
}

/// Bounded unique registry credential records; YAML/file loading is caller-owned.
#[derive(Default)]
#[non_exhaustive]
pub struct RegistryCredentials(BTreeMap<RegistryHost, RegistryCredential>);
impl fmt::Debug for RegistryCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryCredentials(<redacted>)")
    }
}
impl TryFrom<Vec<(RegistryHost, RegistryCredential)>> for RegistryCredentials {
    type Error = DockerError;
    fn try_from(records: Vec<(RegistryHost, RegistryCredential)>) -> Result<Self, Self::Error> {
        if records.len() > 32 {
            return Err(DockerError::Configuration);
        }
        let mut total = 0_usize;
        let mut map = BTreeMap::new();
        for (host, credential) in records {
            total = total
                .checked_add(host.0.len())
                .and_then(|n| n.checked_add(credential.bytes()))
                .ok_or(DockerError::TooLarge)?;
            if total > 64 * 1024 || map.insert(host, credential).is_some() {
                return Err(DockerError::Configuration);
            }
        }
        Ok(Self(map))
    }
}

#[derive(Debug)]
struct Approved {
    hosts: BTreeSet<RegistryHost>,
    credentials: RegistryCredentials,
    timeout: Duration,
}
/// Default-never policy for explicitly configured deployment preparation.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ImagePullPolicy(Option<Arc<Approved>>);
impl ImagePullPolicy {
    /// Validate exact approved registries, matching credentials and total budget.
    /// # Errors
    /// Rejects empty/duplicate/over32 hosts, unapproved credentials and budgets
    /// outside1..600 whole seconds.
    /// ```
    /// let policy=rustack_lambda_core::docker::ImagePullPolicy::if_missing(
    ///     vec!["registry.example".parse()?],Default::default(),std::time::Duration::from_secs(300))?;
    /// # Ok::<(),rustack_lambda_core::docker::DockerError>(())
    /// ```
    pub fn if_missing(
        hosts: Vec<RegistryHost>,
        credentials: RegistryCredentials,
        timeout: Duration,
    ) -> Result<Self, DockerError> {
        if hosts.is_empty()
            || hosts.len() > 32
            || timeout.subsec_nanos() != 0
            || !(1..=600).contains(&timeout.as_secs())
        {
            return Err(DockerError::Configuration);
        }
        let len = hosts.len();
        let hosts: BTreeSet<_> = hosts.into_iter().collect();
        if hosts.len() != len || credentials.0.keys().any(|host| !hosts.contains(host)) {
            return Err(DockerError::Configuration);
        }
        Ok(Self(Some(Arc::new(Approved {
            hosts,
            credentials,
            timeout,
        }))))
    }
    fn authorize(
        &self,
        reference: &ImageReference,
    ) -> Result<(&Approved, RegistryHost), DockerError> {
        let policy = self
            .0
            .as_deref()
            .ok_or(DockerError::Status(StatusCode::NOT_FOUND))?;
        let host: RegistryHost = reference.registry().unwrap_or("docker.io").parse()?;
        if !policy.hosts.contains(&host) {
            return Err(DockerError::PullDenied);
        }
        let repository = reference.repository();
        let local = reference
            .registry()
            .and_then(|registry| repository.strip_prefix(registry))
            .and_then(|path| path.strip_prefix('/'))
            .unwrap_or(repository);
        if host.as_str() == "docker.io" && local.starts_with("rustack-internal/") {
            return Err(DockerError::PullDenied);
        }
        Ok((policy, host))
    }
    pub(super) async fn pull(
        &self,
        docker: &DockerPort,
        reference: &ImageReference,
        architecture: Architecture,
        cancel: watch::Receiver<()>,
        stop: watch::Receiver<bool>,
    ) -> Result<(), DockerError> {
        let (policy, host) = self.authorize(reference)?;
        let auth = policy
            .credentials
            .0
            .get(&host)
            .map(|credential| credential.header(&host))
            .transpose()?;
        docker
            .pull_image(
                reference,
                architecture,
                Transfer { auth, cancel, stop },
                Instant::now() + policy.timeout,
            )
            .await
    }
}

pub(super) struct Transfer {
    pub(super) auth: Option<SecretString>,
    pub(super) cancel: watch::Receiver<()>,
    pub(super) stop: watch::Receiver<bool>,
}
impl fmt::Debug for Transfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Transfer(<redacted>)")
    }
}

impl DockerPort {
    async fn pull_image(
        &self,
        reference: &ImageReference,
        architecture: Architecture,
        transfer: Transfer,
        deadline: Instant,
    ) -> Result<(), DockerError> {
        if self.raw.binding.is_none() {
            return Err(DockerError::DaemonBinding);
        }
        let tag = reference
            .digest()
            .or_else(|| {
                reference
                    .as_str()
                    .strip_prefix(reference.repository())
                    .and_then(|suffix| suffix.strip_prefix(':'))
            })
            .unwrap_or("latest");
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("fromImage", reference.repository())
            .append_pair("tag", tag)
            .append_pair("platform", architecture.docker_platform())
            .finish();
        let result = self
            .raw
            .request_owned(
                RawRequest {
                    pull: Some(transfer),
                    binding: None,
                    mutation: None,
                    operation: Operation::Pull,
                    method: Method::POST,
                    target: format!("/v{}/images/create?{query}", self.version),
                    body: Bytes::new(),
                    expected: StatusCode::OK,
                },
                deadline,
            )
            .await?;
        match result {
            RawReply::Pulled => Ok(()),
            _ => Err(DockerError::Protocol),
        }
    }
}

#[cfg(test)]
mod tests;
