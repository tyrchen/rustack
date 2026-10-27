//! Exact recorded identity checks before any code-resource deletion.

use std::collections::BTreeMap;

use uuid::Uuid;

use super::{CodeRecord, LoaderState};
use crate::docker::{
    DockerError, DockerKey,
    models::{ContainerInspectResponse, ContainerStateStatusEnum, Volume, VolumeScopeEnum},
};

impl CodeRecord {
    pub(in crate::docker) fn volume_key(&self) -> Result<DockerKey, DockerError> {
        format!("rustack-code-{}", self.environment).parse()
    }
    pub(in crate::docker) fn loader_key(&self) -> Result<DockerKey, DockerError> {
        match self.loader {
            LoaderState::Known(id) => hex::encode(id).parse(),
            _ => format!("rustack-code-loader-{}", self.environment).parse(),
        }
    }
    pub(in crate::docker) fn labels(&self, owner: Uuid, loader: bool) -> BTreeMap<String, String> {
        [
            ("ownerId", owner.to_string()),
            ("runId", self.run.to_string()),
            ("environmentId", self.environment.to_string()),
            ("functionResourceId", self.resource.to_string()),
            ("revisionId", self.revision.to_string()),
            ("codeSha256", hex::encode(self.zip)),
            ("baseFingerprint", hex::encode(self.fingerprint)),
            ("role", if loader { "code-loader" } else { "code" }.into()),
        ]
        .into_iter()
        .map(|(key, value)| (format!("rustack.lambda.{key}"), value))
        .collect()
    }
    pub(in crate::docker) fn owned_volume(
        &self,
        owner: Uuid,
        volume: &Volume,
    ) -> Result<(), DockerError> {
        let labels = self.labels(owner, false);
        if volume.name != self.volume_key()?.as_str()
            || volume.driver != "local"
            || volume.scope != Some(VolumeScopeEnum::LOCAL)
            || !volume.options.is_empty()
            || volume.labels.len() != labels.len()
            || !labels
                .iter()
                .all(|(key, value)| volume.labels.get(key) == Some(value))
        {
            return Err(DockerError::Quarantined);
        }
        Ok(())
    }
    pub(in crate::docker) fn owned_loader(
        &self,
        owner: Uuid,
        inspected: &ContainerInspectResponse,
    ) -> Result<[u8; 32], DockerError> {
        let config = inspected.config.as_ref().ok_or(DockerError::Protocol)?;
        let labels = config.labels.as_ref().ok_or(DockerError::Quarantined)?;
        let expected = self.labels(owner, true);
        let image = format!("sha256:{}", hex::encode(self.base));
        let id = inspected.id.as_deref().ok_or(DockerError::Protocol)?;
        let mut decoded = [0; 32];
        if id.len() != 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DockerError::Quarantined);
        }
        hex::decode_to_slice(id, &mut decoded).map_err(|_| DockerError::Quarantined)?;
        let state = inspected.state.as_ref().ok_or(DockerError::Protocol)?;
        let mounts = inspected
            .mounts
            .as_deref()
            .ok_or(DockerError::Quarantined)?;
        let [mount] = mounts else {
            return Err(DockerError::Quarantined);
        };
        if matches!(self.loader, LoaderState::Known(known) if known != decoded)
            || inspected.name.as_deref()
                != Some(format!("/rustack-code-loader-{}", self.environment).as_str())
            || config.image.as_deref() != Some(image.as_str())
            || inspected.image.as_deref() != Some(image.as_str())
            || labels.len() > 64
            || labels.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 128
                    || value.len() > 256
                    || !key
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
                    || value.chars().any(char::is_control)
                    || (key.starts_with("rustack.lambda.") && expected.get(key) != Some(value))
            })
            || !expected
                .iter()
                .all(|(key, value)| labels.get(key) == Some(value))
            || state.running != Some(false)
            || state.status != Some(ContainerStateStatusEnum::CREATED)
            || !matches!(
                state.started_at.as_deref(),
                None | Some("" | "0001-01-01T00:00:00Z" | "0001-01-01T00:00:00.000000000Z")
            )
            || mount.typ.as_deref() != Some("volume")
            || mount.driver.as_deref() != Some("local")
            || mount.name.as_deref() != Some(self.volume_key()?.as_str())
            || mount.destination.as_deref() != Some("/var/task")
            || mount.rw != Some(true)
        {
            return Err(DockerError::Quarantined);
        }
        Ok(decoded)
    }
}
