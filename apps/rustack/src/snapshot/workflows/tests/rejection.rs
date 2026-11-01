//! Valid-envelope corruption cannot publish a partial workflow graph.

use rstest::rstest;
use rustack_stepfunctions_core::RegistryHistories;

use super::*;
use crate::snapshot::{
    archive::{
        ArchiveSection, SECTION_STATE_CBOR, pack_data_archive, to_cbor, unpack_data_archive,
        write_archive,
    },
    read_manifest, write_manifest,
};

#[rstest]
#[case("schema")]
#[case("profile")]
#[case("hash")]
#[case("missing-data")]
#[case("orphan-data")]
#[case("unknown-field")]
#[case("quota")]
#[case("missing-file")]
#[tokio::test]
async fn test_should_reject_invalid_archive_before_publishing_any_workflow(
    #[case] corruption: &str,
) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = config(&directory)?;
    let mut source = runtime(RuntimeBudgets::default())?;
    let mut budgets = RuntimeBudgets::default();
    if corruption == "quota" {
        budgets.workflow_registry_records = 1;
    }
    let mut target = runtime(budgets)?;
    let result = async {
        let first = start(&source, 0).await?;
        let (status, reply) = request(
            source.api.clone(),
            "StartExecution",
            json!({"stateMachineArn":first.arn().machine().as_str(),"name":"second","input":"{}"}),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "{reply}");
        source.owner.quiesce(deadline()).await?;
        config
            .save(&providers(source.archive.clone()), "source")
            .await?;
        let service = config.snapshot_dir().join("services/stepfunctions");
        let meta_path = service.join("meta.ss.zst");
        let data_path = service.join("data.ss.zst");
        let sections = read_archive(&meta_path, ArchiveKind::ServiceMeta).await?;
        let mut metadata = serde_json::to_value(from_cbor::<RegistryMetadata>(
            &sections.first().context("missing metadata")?.bytes,
        )?)?;
        let history_dir = directory.path().join("mutated-history");
        unpack_data_archive(
            &data_path,
            &history_dir,
            Some(1024 * 1024),
            Some(HISTORY_FILE),
        )
        .await?;
        let mut histories = serde_json::to_value(from_cbor::<RegistryHistories>(
            &fs::read(history_dir.join(HISTORY_FILE)).await?,
        )?)?;
        match corruption {
            "schema" => metadata["schemaVersion"] = json!(2),
            "profile" => metadata["profileVersion"] = json!(2),
            "hash" => metadata["revisions"][0]["sourceHash"] = json!(vec![0; 32]),
            "missing-data" => histories["histories"] = json!([]),
            "orphan-data" => {
                histories["histories"][0]["arn"] =
                    json!("arn:aws:states:us-east-1:000000000000:execution:Machine:orphan");
            }
            "unknown-field" => metadata["private-archive-marker"] = json!("private-archive-marker"),
            "quota" => {}
            "missing-file" => {
                let mut manifest =
                    read_manifest(&config.snapshot_dir().join(MANIFEST_FILE)).await?;
                let service = manifest
                    .services
                    .get_mut("stepfunctions")
                    .context("missing service entry")?;
                service.data_file = None;
                service.data = None;
                write_manifest(&config.snapshot_dir().join(MANIFEST_FILE), &manifest).await?;
            }
            _ => anyhow::bail!("unknown corruption"),
        }
        write_archive(
            &meta_path,
            ArchiveKind::ServiceMeta,
            vec![ArchiveSection::new(
                SECTION_STATE_CBOR,
                to_cbor(&metadata)?,
                1,
            )],
        )
        .await?;
        fs::write(history_dir.join(HISTORY_FILE), to_cbor(&histories)?).await?;
        pack_data_archive(&history_dir, &data_path).await?;
        let before = fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?;
        let error = config
            .load(&providers(target.archive.clone()))
            .await
            .err()
            .context("accepted invalid workflow archive")?;
        assert!(!format!("{error:#?}").contains("private-archive-marker"));
        let (status, machines) =
            request(target.api.clone(), "ListStateMachines", json!({})).await?;
        assert_eq!(status, StatusCode::OK, "{machines}");
        assert_eq!(machines.get("stateMachines"), Some(&json!([])));
        assert_eq!(
            fs::read(config.snapshot_dir().join(MANIFEST_FILE)).await?,
            before
        );
        Ok(())
    }
    .await;
    let one = source.owner.shutdown(deadline()).await;
    let two = target.owner.shutdown(deadline()).await;
    one?;
    two?;
    result
}
#[tokio::test]
async fn test_should_load_old_generation_without_workflow_service_as_empty() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = config(&directory)?;
    config.save(&RuntimeProviders::default(), "old").await?;
    let mut target = runtime(RuntimeBudgets::default())?;
    let result = async {
        config.load(&providers(target.archive.clone())).await?;
        let (status, machines) =
            request(target.api.clone(), "ListStateMachines", json!({})).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(machines.get("stateMachines"), Some(&json!([])));
        Ok(())
    }
    .await;
    target.owner.shutdown(deadline()).await?;
    result
}

#[rstest]
#[case("profile")]
#[case("hash")]
#[case("scope")]
#[tokio::test]
async fn test_should_retain_both_generations_until_domain_import_succeeds(
    #[case] corruption: &str,
) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = config(&directory)?;
    let mut source = runtime(RuntimeBudgets::default())?;
    let mut target = runtime(RuntimeBudgets::default())?;
    let result = async {
        start(&source, 0).await?;
        source.owner.quiesce(deadline()).await?;
        let providers = providers(source.archive.clone());
        config.save(&providers, "previous").await?;
        let previous = directory.path().join(".workflows.previous");
        fs::rename(config.snapshot_dir(), &previous).await?;
        config.save(&providers, "current").await?;
        // Recreate the post-publish/pre-retirement crash window with two
        // physically valid generations, before corrupting only domain data.
        fs::create_dir_all(previous.join("services/stepfunctions")).await?;
        let files = [
            MANIFEST_FILE,
            "services/stepfunctions/meta.ss.zst",
            "services/stepfunctions/data.ss.zst",
        ];
        let mut original = Vec::new();
        for relative in files {
            let bytes = fs::read(config.snapshot_dir().join(relative)).await?;
            fs::write(previous.join(relative), &bytes).await?;
            original.push(bytes);
        }
        let meta_path = config.snapshot_dir().join(files[1]);
        let sections = read_archive(&meta_path, ArchiveKind::ServiceMeta).await?;
        let mut metadata = serde_json::to_value(from_cbor::<RegistryMetadata>(
            &sections.first().context("missing metadata")?.bytes,
        )?)?;
        match corruption {
            "profile" => metadata["profileVersion"] = json!(2),
            "hash" => metadata["revisions"][0]["sourceHash"] = json!(vec![0; 32]),
            "scope" => metadata["scope"]["account"] = json!("111111111111"),
            _ => anyhow::bail!("unknown corruption"),
        }
        write_archive(
            &meta_path,
            ArchiveKind::ServiceMeta,
            vec![ArchiveSection::new(
                SECTION_STATE_CBOR,
                to_cbor(&metadata)?,
                1,
            )],
        )
        .await?;
        let corrupted = fs::read(&meta_path).await?;
        let target_providers = super::providers(target.archive.clone());
        assert!(config.load(&target_providers).await.is_err());
        for (relative, bytes) in files.into_iter().zip(&original) {
            assert_eq!(&fs::read(previous.join(relative)).await?, bytes);
        }
        assert_eq!(fs::read(&meta_path).await?, corrupted);
        let (status, machines) =
            request(target.api.clone(), "ListStateMachines", json!({})).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(machines.get("stateMachines"), Some(&json!([])));
        fs::write(
            &meta_path,
            original.get(1).context("missing original metadata")?,
        )
        .await?;
        target.owner.shutdown(deadline()).await?;
        target = runtime(RuntimeBudgets::default())?;
        config
            .load(&super::providers(target.archive.clone()))
            .await?;
        assert!(!fs::try_exists(&previous).await?);
        let (status, machines) =
            request(target.api.clone(), "ListStateMachines", json!({})).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            machines
                .get("stateMachines")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        Ok(())
    }
    .await;
    let one = source.owner.shutdown(deadline()).await;
    let two = target.owner.shutdown(deadline()).await;
    one?;
    two?;
    result
}
