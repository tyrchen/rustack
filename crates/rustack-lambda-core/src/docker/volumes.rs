//! Local volumes and typed code upload; callers retain mutation recovery.

use std::collections::BTreeMap;

use bytes::Bytes;
use http::{Method, StatusCode};
use tokio::time::Instant;

use super::{
    DockerCodeArchive, DockerError, DockerKey, DockerPort, Operation, OwnerFilter, RawRequest,
    decode, encode, escaped,
    models::{Volume, VolumeCreateRequest, VolumeListResponse, VolumeScopeEnum},
    unit,
};

impl DockerPort {
    /// Upload checked code to fixed `/var/task`, retaining buffer admission.
    ///
    /// Requires an existing writable code volume. Creates/starts no container.
    ///
    /// # Errors
    /// Returns normal wire failures. Cancellation/error does not prove no writes.
    ///
    /// ```no_run
    /// # async fn upload(p: &rustack_lambda_core::docker::DockerPort, c: &rustack_lambda_core::docker::DockerKey, a: &rustack_lambda_core::docker::DockerCodeArchive) {
    /// let result = p.upload_code(c, a, tokio::time::Instant::now() + std::time::Duration::from_secs(30)).await;
    /// # }
    /// ```
    pub async fn upload_code(
        &self,
        container: &DockerKey,
        code: &DockerCodeArchive,
        deadline: Instant,
    ) -> Result<(), DockerError> {
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("path", "/var/task")
            .append_pair("noOverwriteDirNonDir", "true")
            .append_pair("copyUIDGID", "false")
            .finish();
        unit(
            self.raw
                .request(
                    RawRequest {
                        archive: Some(code.clone()),
                        pull: None,
                        binding: None,
                        mutation: None,
                        operation: Operation::UploadCode,
                        method: Method::PUT,
                        target: format!(
                            "/v{}/containers/{}/archive?{query}",
                            self.version,
                            escaped(container.as_str())
                        ),
                        body: code.bytes().clone(),
                        expected: StatusCode::OK,
                    },
                    deadline,
                )
                .await?,
            deadline,
        )
    }

    /// Create a named local volume without driver, bind or cluster options.
    ///
    /// Caller must first journal intent and retain it through uncertain creation.
    ///
    /// # Errors
    /// Returns invalid labels, mismatched reply metadata or normal wire failures.
    /// No error authorizes deletion/retry; a matching reply is not journal authority.
    ///
    /// ```no_run
    /// # async fn create(p: &rustack_lambda_core::docker::DockerPort, n: &rustack_lambda_core::docker::DockerKey) {
    /// let labels = std::collections::BTreeMap::new();
    /// let result = p.create_volume(n, &labels, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn create_volume(
        &self,
        name: &DockerKey,
        labels: &BTreeMap<String, String>,
        deadline: Instant,
    ) -> Result<Volume, DockerError> {
        validate_labels(
            labels
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )?;
        let body = VolumeCreateRequest {
            name: Some(name.as_str().to_owned()),
            driver: Some("local".into()),
            labels: Some(
                labels
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
            ..VolumeCreateRequest::default()
        };
        let volume: Volume = decode(
            self.call(
                Operation::CreateVolume,
                Method::POST,
                "/volumes/create".into(),
                encode(&body)?,
                StatusCode::CREATED,
                deadline,
            )
            .await?,
            deadline,
        )?;
        validate_volume(&volume)?;
        if volume.name != name.as_str()
            || volume.driver != "local"
            || volume.scope != Some(VolumeScopeEnum::LOCAL)
            || !volume.options.is_empty()
            || volume.labels.len() != labels.len()
            || !labels
                .iter()
                .all(|(key, value)| volume.labels.get(key) == Some(value))
        {
            return Err(DockerError::Protocol);
        }
        Ok(volume)
    }

    /// Inspect one volume through reserved cleanup admission.
    ///
    /// # Errors
    /// Returns invalid/bounded metadata or wire failures. Observation grants no
    /// destructive authority; the caller must verify its durable identity.
    ///
    /// ```no_run
    /// # async fn inspect(p: &rustack_lambda_core::docker::DockerPort, n: &rustack_lambda_core::docker::DockerKey) {
    /// let result = p.inspect_volume(n, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn inspect_volume(
        &self,
        name: &DockerKey,
        deadline: Instant,
    ) -> Result<Volume, DockerError> {
        let volume: Volume = decode(
            self.call(
                Operation::InspectVolume,
                Method::GET,
                format!("/volumes/{}", escaped(name.as_str())),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
            deadline,
        )?;
        validate_volume(&volume)?;
        if volume.name != name.as_str() {
            return Err(DockerError::Protocol);
        }
        Ok(volume)
    }

    /// List a complete owner-label volume scope, without a positive result limit.
    ///
    /// # Errors
    /// Rejects warnings, missing collections, over1024 rows, invalid metadata and
    /// wire failures; no incomplete response is returned as an absence proof.
    ///
    /// ```no_run
    /// # async fn list(p: &rustack_lambda_core::docker::DockerPort) -> Result<(), rustack_lambda_core::docker::DockerError> {
    /// let scope = rustack_lambda_core::docker::OwnerFilter::new(uuid::Uuid::new_v4(), None, None)?;
    /// let result = p.list_volumes(scope, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # Ok(()) }
    /// ```
    pub async fn list_volumes(
        &self,
        scope: OwnerFilter,
        deadline: Instant,
    ) -> Result<Vec<Volume>, DockerError> {
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("filters", &scope.encoded_labels()?)
            .finish();
        let response: VolumeListResponse = decode(
            self.call(
                Operation::ListVolumes,
                Method::GET,
                format!("/volumes?{query}"),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
            deadline,
        )?;
        if response
            .warnings
            .is_some_and(|warnings| !warnings.is_empty())
        {
            return Err(DockerError::Protocol);
        }
        let volumes = response.volumes.ok_or(DockerError::Protocol)?;
        if volumes.len() > 1024 {
            return Err(DockerError::TooLarge);
        }
        for volume in &volumes {
            validate_volume(volume)?;
        }
        Ok(volumes)
    }

    /// Remove a named volume with force disabled and reserved cleanup admission.
    ///
    /// # Errors
    /// Returns normal wire failures, including attached-volume conflict. Caller
    /// must prove ownership/consumer absence first, then inspect absence; neither
    ///404 nor this response settles a journal here.
    ///
    /// ```no_run
    /// # async fn remove(p: &rustack_lambda_core::docker::DockerPort, n: &rustack_lambda_core::docker::DockerKey) {
    /// let result = p.remove_volume(n, tokio::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn remove_volume(
        &self,
        name: &DockerKey,
        deadline: Instant,
    ) -> Result<(), DockerError> {
        unit(
            self.call(
                Operation::RemoveVolume,
                Method::DELETE,
                format!("/volumes/{}?force=false", escaped(name.as_str())),
                Bytes::new(),
                StatusCode::NO_CONTENT,
                deadline,
            )
            .await?,
            deadline,
        )
    }
}

fn validate_labels<'a>(
    labels: impl Iterator<Item = (&'a str, &'a str)>,
) -> Result<(), DockerError> {
    for (index, (key, value)) in labels.enumerate() {
        if index >= 16
            || key.is_empty()
            || key.len() > 128
            || value.len() > 256
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
            || value.chars().any(char::is_control)
        {
            return Err(DockerError::Configuration);
        }
    }
    Ok(())
}
fn validate_volume(volume: &Volume) -> Result<(), DockerError> {
    let _: DockerKey = volume.name.parse()?;
    if volume.driver.len() > 256 || volume.mountpoint.len() > 4096 || volume.options.len() > 16 {
        return Err(DockerError::TooLarge);
    }
    validate_labels(
        volume
            .labels
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    )
}

#[cfg(test)]
mod tests;
