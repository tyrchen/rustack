//! Release profile configuration copied into the existing exact data volume.

use std::{fs::Permissions, os::unix::fs::PermissionsExt, time::Duration};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    time::{Instant, timeout_at},
};

use super::{
    super::{cli::arguments, records},
    Owner, Resources,
    data::{DataOperation, DataOutput},
};

const CONFIGURATION: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/lambda-fixtures/server-config.yaml"
));

impl Owner {
    pub(super) async fn install_public_configuration(&mut self, cutoff: Instant) -> Result<()> {
        ensure!(
            (1..=4096).contains(&CONFIGURATION.len()),
            "public configuration byte bound"
        );
        self.inspect_server(cutoff).await?;
        let Resources::Server { id, .. } = &self.resources else {
            anyhow::bail!("public configuration without exact server");
        };
        let id = id.clone();
        self.write("qualification.yaml", CONFIGURATION, cutoff)
            .await?;
        let path = self.path.join("qualification.yaml");
        let at = cutoff.min(Instant::now() + Duration::from_secs(10));
        timeout_at(
            at,
            fs::set_permissions(&path, Permissions::from_mode(0o644)),
        )
        .await??;
        ensure!(
            timeout_at(at, fs::metadata(&path))
                .await??
                .permissions()
                .mode()
                & 0o777
                == 0o644,
            "public configuration host mode differs"
        );
        let source = path
            .to_str()
            .context("public configuration path encoding")?;
        let DataOutput::Configuration(observed) = self
            .transfer_data(DataOperation::Configuration(source), cutoff)
            .await?
        else {
            anyhow::bail!("incorrect data-volume configuration operation");
        };
        self.inspect_server(cutoff).await?;
        records::configuration(&observed, CONFIGURATION)?;
        self.write(
            "configuration.json",
            &serde_json::to_vec(&json!({
                "schemaVersion":1,"id":id,"engine":self.expected.engine.to_string(),
                "name":"qualification.yaml","bytes":CONFIGURATION.len(),"uid":65532,"gid":65532,"mode":420,
                "sha256Base64":STANDARD.encode(Sha256::digest(CONFIGURATION))
            }))?,
            cutoff,
        )
        .await?;
        self.uncertain = false;
        Ok(())
    }
}

pub(super) fn copy_arguments(source: &str, target: &str) -> Vec<String> {
    // Moby's explicit CopyUIDGID branch binds extraction to Config.User.
    // The default branch can preserve the host archive's UID/GID instead.
    arguments(&["cp", "--archive", source, target])
}

#[cfg(test)]
mod tests {
    use super::copy_arguments;

    #[test]
    fn test_should_request_explicit_container_user_ownership_for_public_copy() {
        let source = "/private/controlled root/qualification.yaml";
        let target = "controlled-id:/data/qualification.yaml";
        assert_eq!(
            copy_arguments(source, target),
            ["cp", "--archive", source, target],
        );
    }
}
