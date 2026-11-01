//! Exact unstarted volume helper; the actual server keeps its read-only socket.

#[path = "data_policy.rs"]
mod policy;

use std::path::Path;

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::docker::models::ContainerInspectResponse;
use serde_json::json;
use tokio::time::Instant;
use uuid::Uuid;

use super::{
    super::{
        cli::{self, arguments},
        policy::{Expected, volume},
        records::{self, Identity},
    },
    INSPECT, Owner, configuration, write_private,
};

#[derive(Debug)]
pub(super) enum DataOperation<'a> {
    Identity,
    Configuration(&'a str),
}

#[derive(Debug)]
pub(super) enum DataOutput {
    Identity([Vec<u8>; 2]),
    Configuration(Vec<u8>),
}

impl Owner {
    pub(super) async fn transfer_data(
        &mut self,
        operation: DataOperation<'_>,
        cutoff: Instant,
    ) -> Result<DataOutput> {
        ensure!(!self.uncertain, "uncertain parent before data transfer");
        let data = self.inspect_volume(cutoff).await?;
        volume(&data, &self.expected)?;
        let transfer = Uuid::new_v4().simple().to_string();
        let name = format!("rustack-transfer-{}-{transfer}", self.expected.nonce);
        let writing = matches!(operation, DataOperation::Configuration(_));
        let args = create_arguments(&name, &self.expected);
        self.write(&format!("data-{transfer}-intent.json"), &serde_json::to_vec(&json!({
            "schemaVersion":1,"name":name,"engine":self.expected.engine.to_string(),
            "parentNonce":self.expected.nonce,"source":self.expected.source,"volume":self.expected.volume,
            "configurationWrite":writing,"argv":args
        }))?, cutoff).await?;
        self.uncertain = true;
        let reply = self.cli.run(&args, cutoff).await?;
        let text = std::str::from_utf8(&reply)?
            .strip_suffix('\n')
            .context("data helper create newline")?;
        let id = cli::container_id(text)?;
        self.write(&format!("data-{transfer}-created.json"), &serde_json::to_vec(&json!({
            "id":id,"name":name,"engine":self.expected.engine.to_string(),"parentNonce":self.expected.nonce
        }))?, cutoff).await?;
        let result = async {
            self.inspect_data(&id, &name, cutoff).await?;
            match operation {
                DataOperation::Identity => {
                    let mut archives = [Vec::new(), Vec::new()];
                    for (slot, filename) in archives.iter_mut().zip(["owner.v1", "daemon.v1"]) {
                        let source = format!("{id}:/data/state/.rustack-lambda-docker/{filename}");
                        *slot = self
                            .cli
                            .run(&arguments(&["cp", &source, "-"]), cutoff)
                            .await?;
                        self.write(&format!("data-{transfer}-{filename}.tar"), slot, cutoff)
                            .await?;
                    }
                    Ok(DataOutput::Identity(archives))
                }
                DataOperation::Configuration(source) => {
                    let target = format!("{id}:/data/qualification.yaml");
                    drop(
                        self.cli
                            .run(&configuration::copy_arguments(source, &target), cutoff)
                            .await?,
                    );
                    let archive = self
                        .cli
                        .run(&arguments(&["cp", &target, "-"]), cutoff)
                        .await?;
                    self.write(
                        &format!("data-{transfer}-configuration.tar"),
                        &archive,
                        cutoff,
                    )
                    .await?;
                    Ok(DataOutput::Configuration(archive))
                }
            }
        }
        .await;
        self.inspect_data(&id, &name, cutoff).await?;
        let removed = self.cli.run(&arguments(&["rm", &id]), cutoff).await?;
        ensure!(
            removed == format!("{id}\n").as_bytes(),
            "data helper removal reply differs"
        );
        let remaining = self
            .cli
            .run(
                &arguments(&[
                    "container",
                    "ls",
                    "--all",
                    "--no-trunc",
                    "--filter",
                    &format!("id={id}"),
                    "--format",
                    "{{json .ID}}",
                ]),
                cutoff,
            )
            .await?;
        ensure!(remaining.is_empty(), "data helper removal unconfirmed");
        self.write(
            &format!("data-{transfer}-removed.json"),
            &serde_json::to_vec(&json!({
                "id":id,"engine":self.expected.engine.to_string(),"neverStarted":true,"absent":true
            }))?,
            cutoff,
        )
        .await?;
        result
    }

    async fn inspect_data(&self, id: &str, name: &str, cutoff: Instant) -> Result<()> {
        let view: ContainerInspectResponse = self
            .cli
            .json(
                &arguments(&["container", "inspect", "--format", INSPECT, id]),
                cutoff,
            )
            .await?;
        policy::inspect(&view, &self.expected, id, name)
    }
}

pub(super) async fn commit_identity(
    uncertain: &mut bool,
    slot: &mut Option<Identity>,
    archives: Result<[Vec<u8>; 2]>,
    engine: Uuid,
    directory: &Path,
    cutoff: Instant,
) -> Result<()> {
    let [owner, daemon] = archives?;
    ensure!(*uncertain, "identity commit without retained transfer");
    let identity = records::identity(&owner, &daemon, engine)?;
    write_private(
        directory,
        "identity.json",
        &serde_json::to_vec(&json!({
            "owner":identity.owner.to_string(),"engine":identity.engine.to_string()
        }))?,
        cutoff,
    )
    .await?;
    *slot = Some(identity);
    *uncertain = false;
    Ok(())
}

fn create_arguments(name: &str, expected: &Expected) -> Vec<String> {
    let mut args = arguments(&[
        "create",
        "--pull=never",
        "--name",
        name,
        "--user",
        "65532:65532",
        "--read-only",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges:true",
        "--cgroupns",
        "private",
        "--network",
        "none",
        "--memory",
        "32m",
        "--memory-swap",
        "32m",
        "--cpus",
        "0.1",
        "--pids-limit",
        "16",
        "--restart",
        "no",
        "--no-healthcheck",
        "--mount",
        &format!(
            "type=volume,source={},target=/data,volume-nocopy",
            expected.volume
        ),
    ]);
    for (key, value) in expected.labels("data-transfer") {
        args.extend(["--label".into(), format!("{key}={value}")]);
    }
    args.push(expected.source.clone());
    args
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sha2::{Digest, Sha256};
    use tar::{Builder, Header};
    use tempfile::tempdir;
    use tokio::fs;

    use super::*;

    fn archives(engine: Uuid) -> Result<[Vec<u8>; 2]> {
        let owner = Uuid::from_u128(3);
        let mut result = [Vec::new(), Vec::new()];
        for (index, (name, magic)) in [("owner.v1", b"RSDKOWN2"), ("daemon.v1", b"RSDKDMD1")]
            .into_iter()
            .enumerate()
        {
            let mut data = magic.to_vec();
            data.extend_from_slice(owner.as_bytes());
            if index == 1 {
                data.extend_from_slice(engine.as_bytes());
            }
            let checksum = Sha256::digest(&data);
            data.extend_from_slice(&checksum);
            let mut header = Header::new_ustar();
            header.set_path(name)?;
            header.set_size(u64::try_from(data.len())?);
            header.set_uid(65532);
            header.set_gid(65532);
            header.set_mode(0o600);
            header.set_cksum();
            let mut builder = Builder::new(Vec::new());
            builder.append(&header, data.as_slice())?;
            *result.get_mut(index).context("controlled archive slot")? = builder.into_inner()?;
        }
        Ok(result)
    }

    #[rstest::rstest]
    #[case::copy(0)]
    #[case::affiliation(1)]
    #[case::receipt(2)]
    #[tokio::test]
    async fn test_should_retain_identity_uncertainty_until_archive_oracle_and_receipt_commit(
        #[case] failure: u8,
    ) -> Result<()> {
        let directory = tempdir()?;
        let engine = Uuid::from_u128(2);
        let mut uncertain = true;
        let mut identity = None;
        let input = match failure {
            0 => Err(anyhow::anyhow!("controlled archive copy failure")),
            1 => Ok(archives(Uuid::from_u128(9))?),
            _ => {
                fs::write(directory.path().join("identity.json"), b"original receipt").await?;
                Ok(archives(engine)?)
            }
        };
        assert!(
            commit_identity(
                &mut uncertain,
                &mut identity,
                input,
                engine,
                directory.path(),
                Instant::now() + Duration::from_secs(10)
            )
            .await
            .is_err()
        );
        assert!(uncertain);
        assert!(identity.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_clear_identity_uncertainty_only_after_the_durable_receipt() -> Result<()> {
        let directory = tempdir()?;
        let engine = Uuid::from_u128(2);
        let mut uncertain = true;
        let mut identity = None;
        commit_identity(
            &mut uncertain,
            &mut identity,
            Ok(archives(engine)?),
            engine,
            directory.path(),
            Instant::now() + Duration::from_secs(10),
        )
        .await?;
        assert!(!uncertain);
        assert_eq!(identity.context("committed identity")?.engine, engine);
        assert!(directory.path().join("identity.json").is_file());
        Ok(())
    }
}
