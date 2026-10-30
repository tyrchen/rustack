//! Shared immutable base policy for startup and retained staging.

use tokio::time::Instant;

use crate::{
    artifact::{Architecture, PinnedImage},
    docker::{DockerError, DockerPort},
};

pub(in crate::docker) async fn validate_base(
    base: &PinnedImage,
    docker: &DockerPort,
    deadline: Instant,
) -> Result<(), DockerError> {
    let metadata = base.resolved().metadata();
    let reference = metadata.image_id.as_str().parse()?;
    let inspected = docker.inspect_image(&reference, deadline).await?;
    let config = inspected.config.as_ref().ok_or(DockerError::Protocol)?;
    let labels = config.labels.as_ref();
    if inspected.id.as_deref() != Some(metadata.image_id.as_str())
        || inspected.os.as_deref() != Some("linux")
        || inspected.architecture.as_deref()
            != Some(match metadata.architecture {
                Architecture::Arm64 => "arm64",
                Architecture::X86_64 => "amd64",
            })
        || config
            .volumes
            .as_ref()
            .is_some_and(|volumes| !volumes.is_empty())
        || labels.is_some_and(|labels| {
            labels.len() > 56
                || labels.iter().any(|(key, value)| {
                    key.is_empty()
                        || key.len() > 128
                        || value.len() > 256
                        || key.starts_with("rustack.lambda.")
                        || !key
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                        || value.chars().any(char::is_control)
                })
        })
    {
        return Err(DockerError::Quarantined);
    }
    Ok(())
}
