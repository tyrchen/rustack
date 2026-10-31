//! Managed source bodies remain separate from bounded ordinary metadata.

use std::{
    io::{Cursor, Write},
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::time::Instant;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use super::{
    DockerArchiveProfile,
    validation::{export_docker, validate_docker},
    zip as zip_source,
};
use crate::{
    artifact::Architecture,
    error::LambdaServiceError,
    identity::FunctionResourceId,
    provider::{
        LambdaSnapshot, RustackLambda,
        provided_zip_tests::{fixture, input, package},
    },
    storage::PolicyStatement,
};

fn profile() -> DockerArchiveProfile {
    DockerArchiveProfile::ProvidedZip(Architecture::Arm64)
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
async fn source(large: bool) -> (tempfile::TempDir, RustackLambda, LambdaSnapshot) {
    let (root, provider) = fixture();
    let mut input = input();
    input.publish = Some(true);
    if large {
        let bytes = STANDARD.decode(package(true)).unwrap();
        let mut writer = ZipWriter::new_append(Cursor::new(bytes)).unwrap();
        writer
            .start_file(
                "payload",
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(&vec![42; 200 * 1024]).unwrap();
        input.code.zip_file = Some(STANDARD.encode(writer.finish().unwrap().into_inner()));
    }
    provider.create_function(input).await.unwrap();
    let snapshot = provider.export_snapshot();
    (root, provider, snapshot)
}
async fn validated(snapshot: LambdaSnapshot) -> Result<LambdaSnapshot, LambdaServiceError> {
    validate_docker(snapshot, deadline(), "us-east-1", "000000000000", profile()).await
}
#[tokio::test]
async fn test_should_round_trip_borrowed_large_original_bodies_without_relaxing_metadata() {
    let (_root, provider, original) = source(true).await;
    let expected = serde_json::to_value(&original).unwrap();
    assert!(
        original
            .functions
            .functions
            .first()
            .unwrap()
            .latest
            .zip_bytes_base64
            .as_ref()
            .unwrap()
            .len()
            > 128 * 1024
    );
    let saved = export_docker(&provider, deadline(), profile())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&saved).unwrap(), expected);
    assert_eq!(
        serde_json::to_value(validated(saved).await.unwrap()).unwrap(),
        expected
    );
    assert!(
        export_docker(&provider, deadline(), DockerArchiveProfile::ImageOnly)
            .await
            .is_err()
    );
    provider.shutdown_before(deadline()).await.unwrap();
}
#[rstest]
#[case::description(false)]
#[case::policy_lookalike(true)]
#[tokio::test]
async fn test_should_reject_large_ancillary_text_while_zip_bodies_keep_their_own_allowance(
    #[case] policy: bool,
) {
    let (_root, provider, mut snapshot) = source(true).await;
    let record = snapshot.functions.functions.first_mut().unwrap();
    if policy {
        record.policy.statements.push(PolicyStatement {
            sid: "lookalike".into(),
            effect: "Allow".into(),
            principal: "*".into(),
            action: "lambda:InvokeFunction".into(),
            resource: "*".into(),
            condition: Some(json!({"zipBytesBase64":"x".repeat(128 * 1024 + 1)})),
        });
    } else {
        record.latest.description = "x".repeat(128 * 1024 + 1);
    }
    assert!(matches!(
        validated(snapshot).await,
        Err(LambdaServiceError::InvalidParameter { .. })
    ));
    provider.shutdown_before(deadline()).await.unwrap();
}
#[rstest]
#[case::name(false)]
#[case::resource(true)]
#[tokio::test]
async fn test_should_reject_duplicate_function_scope_before_any_staging(#[case] identity: bool) {
    let (_root, provider, mut snapshot) = source(false).await;
    let mut duplicate = snapshot.functions.functions.first().unwrap().clone();
    if identity {
        duplicate.name = "another".into();
        duplicate.arn = "arn:aws:lambda:us-east-1:000000000000:function:another".into();
    } else {
        duplicate.resource_id = FunctionResourceId::new();
    }
    snapshot.functions.functions.push(duplicate);
    assert!(matches!(
        validated(snapshot).await,
        Err(LambdaServiceError::InvalidParameter { .. })
    ));
    provider.shutdown_before(deadline()).await.unwrap();
}
#[rstest]
#[case("runtime")]
#[case("handler")]
#[case("architecture")]
#[case("size")]
#[case("hash")]
#[case("inactive")]
#[case("mixed")]
#[case("missing")]
#[tokio::test]
async fn test_should_reject_invalid_saved_zip_configuration_and_identity_without_effects(
    #[case] changed: &str,
) {
    let (_root, provider, mut snapshot) = source(false).await;
    let row = &mut snapshot.functions.functions.first_mut().unwrap().latest;
    match changed {
        "runtime" => row.runtime = Some("provided.al2".into()),
        "handler" => row.handler = Some("bad handler".into()),
        "architecture" => row.architectures = vec!["x86_64".into()],
        "size" => row.code_size += 1,
        "hash" => row.code_sha256 = "AA==".into(),
        "inactive" => row.state = "Failed".into(),
        "mixed" => row.package_type = "Image".into(),
        _ => row.zip_bytes_base64 = None,
    }
    let old = serde_json::to_value(provider.export_snapshot()).unwrap();
    assert!(matches!(
        validated(snapshot).await,
        Err(LambdaServiceError::InvalidParameter { .. })
    ));
    assert_eq!(
        serde_json::to_value(provider.export_snapshot()).unwrap(),
        old
    );
    provider.shutdown_before(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_count_repeated_revision_source_bytes_at_exact_aggregate_boundary() {
    let (_root, provider, mut snapshot) = source(false).await;
    let expected: Value = serde_json::to_value(&snapshot).unwrap();
    let row = snapshot.functions.functions.first().unwrap();
    let bytes = usize::try_from(row.latest.code_size)
        .unwrap()
        .checked_mul(2)
        .unwrap();
    provider.store.check_snapshot_zip_bytes(bytes).unwrap();
    assert!(provider.store.check_snapshot_zip_bytes(bytes - 1).is_err());
    assert!(zip_source::detach_with_limit(&mut snapshot.clone(), profile(), bytes - 1).is_err());
    let bodies = zip_source::detach_with_limit(&mut snapshot, profile(), bytes).unwrap();
    assert_eq!(bodies.len(), 2);
    zip_source::reattach(&mut snapshot, bodies).unwrap();
    assert_eq!(serde_json::to_value(snapshot).unwrap(), expected);
    provider.shutdown_before(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_preflight_global_metadata_nodes_across_individually_legal_live_records() {
    let (_root, provider, _) = source(false).await;
    let original = provider.store.get("provided-control").unwrap().unwrap();
    for index in 0..16 {
        let mut row = original.clone();
        row.name = format!("metadata-{index}");
        row.arn = format!("arn:aws:lambda:us-east-1:000000000000:function:metadata-{index}");
        row.resource_id = FunctionResourceId::new();
        row.policy.statements.push(PolicyStatement {
            sid: "nodes".into(),
            effect: "Allow".into(),
            principal: "*".into(),
            action: "lambda:InvokeFunction".into(),
            resource: "*".into(),
            condition: Some(Value::Array(vec![Value::Null; 5000])),
        });
        provider.store.insert(row).unwrap();
    }
    assert!(matches!(
        export_docker(&provider, deadline(), profile()).await,
        Err(LambdaServiceError::InvalidParameter { .. })
    ));
    provider.shutdown_before(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_reject_oversized_live_ancillary_text_before_full_dto_construction() {
    let (_root, provider, _) = source(false).await;
    provider
        .store
        .try_update_if_revision("provided-control", None, |record| {
            record.latest.description = "x".repeat(20 * 1024 * 1024);
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        export_docker(&provider, deadline(), profile()).await,
        Err(LambdaServiceError::InvalidParameter { .. })
    ));
    provider.shutdown_before(deadline()).await.unwrap();
}
