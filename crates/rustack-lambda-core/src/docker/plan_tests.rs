use std::{collections::HashSet, sync::atomic::AtomicBool};

use tokio::{
    sync::watch,
    time::{Instant, timeout},
};

use super::*;
use crate::{
    artifact::{
        Architecture, ImageExecutionMetadata, ImageMetadata, ResolvedImage, ValidatedArtifact,
    },
    identity::FunctionResourceId,
};

fn directory() -> DirectoryLease {
    DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4())
}
pub(in crate::docker) fn policy() -> ContainerPolicy {
    ContainerPolicy::new(
        "none".parse().unwrap(),
        1000,
        1000,
        1_000_000_000,
        RelayBudgets::default(),
    )
    .unwrap()
}
pub(in crate::docker) fn image_with(user: &str, environment: Vec<String>) -> PinnedImage {
    let mut metadata = ResolvedImage::fixture(Architecture::Arm64)
        .metadata()
        .clone();
    metadata.execution = ImageExecutionMetadata::builder()
        .entrypoint(vec!["/bootstrap".into()])
        .command(vec!["handler".into()])
        .working_directory("/var/task".into())
        .environment(environment)
        .user(user.into())
        .build()
        .try_into()
        .unwrap();
    metadata.inspection_hash = ImageMetadata::execution_fingerprint(
        &metadata.image_id,
        metadata.selected_manifest.as_ref(),
        metadata.architecture,
        metadata.size_bytes,
        &metadata.execution,
    )
    .unwrap();
    PinnedImage::fixture(metadata.try_into().unwrap())
}
pub(in crate::docker) fn revision() -> ExecutionRevision {
    ExecutionRevision::builder()
        .resource_id(FunctionResourceId::new())
        .revision_id(Uuid::new_v4().to_string())
        .backend(ExecutorBackend::Docker)
        .function_name("policy-fixture".into())
        .qualifier("$LATEST".into())
        .runtime(None)
        .handler(None)
        .architecture(Architecture::Arm64)
        .artifact(DeploymentArtifact::Image(image_with(
            "root",
            vec!["BASE=value".into()],
        )))
        .image_config(None)
        .environment(HashMap::from([
            ("AWS_REGION".into(), "us-east-1".into()),
            ("AWS_DEFAULT_REGION".into(), "us-east-1".into()),
            ("TEST_VALUE".into(), "secret-value".into()),
        ]))
        .timeout(Duration::from_secs(3))
        .memory_mb(192)
        .ephemeral_storage_mb(512)
        .build()
}
fn plan(revision: ExecutionRevision) -> Result<EnvironmentPlan, DockerError> {
    EnvironmentPlan::new(
        directory(),
        image_with("0", vec![]),
        Arc::new(revision),
        policy(),
    )
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[test]
fn test_should_capture_fixed_images_identities_labels_and_complete_resource_policy() {
    let directory = directory();
    let owner = directory.owner_id();
    let run = directory.run_id();
    let revision = revision();
    let resource = revision.resource_id;
    let image = image(&revision)
        .unwrap()
        .resolved()
        .metadata()
        .image_id
        .clone();
    let plan = EnvironmentPlan::new(
        directory,
        image_with("0", vec![]),
        Arc::new(revision),
        policy(),
    )
    .unwrap();
    assert_ne!(plan.relay_key().as_str(), plan.function_key().as_str());
    assert_eq!(plan.memory_bytes(), 256 * MIB);
    for (body, role, memory, cpu, pids, tmp) in [
        (&plan.bodies.relay, "relay", 64 * MIB, 100_000_000, 64, MIB),
        (
            &plan.bodies.function,
            "function",
            192 * MIB,
            1_000_000_000,
            256,
            512 * MIB,
        ),
    ] {
        assert_eq!(body.image.as_deref(), Some(image.as_str()));
        assert_eq!(body.tty, Some(false));
        assert_eq!(body.user.as_deref(), Some("1000:1000"));
        assert_eq!(
            body.healthcheck.as_ref().unwrap().test.as_ref().unwrap(),
            &["NONE"]
        );
        let labels = body.labels.as_ref().unwrap();
        assert_eq!(labels["rustack.lambda.ownerId"], owner.to_string());
        assert_eq!(labels["rustack.lambda.runId"], run.to_string());
        assert_eq!(
            labels["rustack.lambda.environmentId"],
            plan.environment_id().to_string()
        );
        assert_eq!(
            labels["rustack.lambda.functionResourceId"],
            resource.to_string()
        );
        assert_eq!(labels["rustack.lambda.role"], role);
        let limits = body.host_config.as_ref().unwrap();
        assert_eq!(
            limits.cgroupns_mode,
            Some(HostConfigCgroupnsModeEnum::PRIVATE)
        );
        assert_eq!(limits.memory, Some(i64::try_from(memory).unwrap()));
        assert_eq!(limits.memory_swap, limits.memory);
        assert_eq!(limits.oom_kill_disable, Some(false));
        assert_eq!(limits.nano_cpus, Some(cpu));
        assert_eq!(limits.pids_limit, Some(pids));
        assert_eq!(limits.readonly_rootfs, Some(true));
        assert_eq!(limits.cap_drop.as_ref().unwrap(), &["ALL"]);
        assert_eq!(
            limits.security_opt.as_ref().unwrap(),
            &["no-new-privileges:true"]
        );
        assert_eq!(limits.privileged, Some(false));
        assert_eq!(limits.auto_remove, Some(false));
        assert_eq!(limits.publish_all_ports, Some(false));
        assert_eq!(
            limits.restart_policy.as_ref().unwrap().name,
            Some(RestartPolicyNameEnum::NO)
        );
        assert_eq!(
            limits.log_config.as_ref().unwrap().typ.as_deref(),
            Some("none")
        );
        assert_eq!(
            limits.tmpfs.as_ref().unwrap()["/tmp"],
            format!("rw,nosuid,nodev,size={tmp},mode=1777")
        );
        assert!(
            limits.binds.is_none() && limits.devices.is_none() && limits.port_bindings.is_none()
        );
    }
    assert_eq!(plan.bodies.relay.attach_stdin, Some(true));
    assert_eq!(plan.bodies.relay.open_stdin, Some(true));
    assert_eq!(plan.bodies.relay.stdin_once, Some(true));
    assert_eq!(plan.bodies.function.attach_stdin, Some(false));
    assert_eq!(
        plan.bodies
            .function
            .host_config
            .as_ref()
            .unwrap()
            .network_mode
            .as_deref(),
        Some(format!("container:{}", plan.relay_key().as_str()).as_str())
    );
    assert!(!format!("{plan:?}").contains("secret-value"));
}

#[test]
fn test_should_preserve_explicit_image_overrides_and_authoritative_environment_order() {
    let mut captured = revision();
    captured.qualifier = "27".into();
    captured.artifact = DeploymentArtifact::Image(image_with(
        "1234:4321",
        vec![
            "TEST_VALUE=old".into(),
            "AWS_LAMBDA_RUNTIME_API=outside:9001".into(),
            "DELETED".into(),
        ],
    ));
    captured.image_config = Some(ImageConfig {
        entry_point: Some(vec![]),
        command: Some(vec!["/actual".into(), "$(not-a-shell)".into()]),
        working_directory: Some("/app".into()),
    });
    let plan = plan(captured).unwrap();
    let body = &plan.bodies.function;
    assert_eq!(body.user.as_deref(), Some("1234:4321"));
    assert_eq!(body.entrypoint.as_ref().unwrap(), &[""]);
    assert_eq!(body.cmd.as_ref().unwrap(), &["/actual", "$(not-a-shell)"]);
    assert_eq!(body.working_dir.as_deref(), Some("/app"));
    let env = body.env.as_ref().unwrap();
    assert!(env.iter().any(|entry| entry == "DELETED"));
    let keys: HashSet<_> = env
        .iter()
        .map(|entry| entry.split('=').next().unwrap())
        .collect();
    assert_eq!(keys.len(), env.len());
    assert_eq!(
        env.iter()
            .find(|entry| entry.starts_with("AWS_LAMBDA_RUNTIME_API="))
            .unwrap(),
        "AWS_LAMBDA_RUNTIME_API=127.0.0.1:9001"
    );
    assert!(
        !env.iter()
            .any(|entry| entry.contains("outside:9001") || entry == "TEST_VALUE=old")
    );
    assert!(env.contains(&"TEST_VALUE=secret-value".into()));
    assert!(env.contains(&"AWS_LAMBDA_FUNCTION_VERSION=27".into()));
    assert!(env.contains(&"AWS_LAMBDA_FUNCTION_MEMORY_SIZE=192".into()));
    assert!(env.contains(&"AWS_LAMBDA_FUNCTION_TIMEOUT=3".into()));
    assert!(env.contains(&"LAMBDA_TASK_ROOT=/var/task".into()));
    let relay = plan.bodies.relay.env.as_ref().unwrap();
    assert_eq!(relay.len(), 4);
    assert!(!relay.iter().any(|entry| entry.contains("secret-value")));
}

#[rstest::rstest]
#[case::absent("", "1000:1000")]
#[case::root("root", "1000:1000")]
#[case::numeric_root("0:4321", "1000:1000")]
#[case::padded_root("0000", "1000:1000")]
#[case::numeric("1234", "1234")]
#[case::named("nobody:nogroup", "nobody:nogroup")]
#[case::named_numeric_group("app_user:0", "app_user:0")]
fn test_should_select_nonroot_container_users(#[case] user: &str, #[case] expected: &str) {
    assert_eq!(image_user(user, "1000:1000").unwrap(), expected);
}
#[rstest::rstest]
#[case(":1000")]
#[case("1000:")]
#[case("root:group:extra")]
#[case("+0")]
#[case("4294967295")]
#[case("account name")]
#[case("/root")]
fn test_should_reject_malformed_image_users(#[case] user: &str) {
    assert!(image_user(user, "1000:1000").is_err());
}

#[rstest::rstest]
#[case("host")]
#[case("bridge")]
#[case("default")]
#[case("container:other")]
#[case("")]
#[case("bad/network")]
fn test_should_reject_unsafe_network_selections(#[case] value: &str) {
    assert!(value.parse::<NetworkSelection>().is_err());
}
#[test]
fn test_should_accept_disconnected_and_named_operator_networks_without_claiming_probe_success() {
    for name in ["none", "rustack-test", "ab0123"] {
        assert!(name.parse::<NetworkSelection>().is_ok());
    }
    for (uid, gid, cpu) in [
        (0, 1000, 1_000_000_000),
        (1000, 0, 1_000_000_000),
        (u32::MAX, 1000, 1_000_000_000),
        (1000, 1000, 0),
        (1000, 1000, 99_999_999),
        (1000, 1000, 16_000_000_001),
    ] {
        assert!(
            ContainerPolicy::new(
                "none".parse().unwrap(),
                uid,
                gid,
                cpu,
                RelayBudgets::default()
            )
            .is_err()
        );
    }
    for cpu in [100_000_000, 16_000_000_000] {
        assert!(
            ContainerPolicy::new(
                "none".parse().unwrap(),
                1,
                2_147_483_647,
                cpu,
                RelayBudgets::default()
            )
            .is_ok()
        );
    }
    for durations in [
        (
            Duration::ZERO,
            Duration::from_secs(600),
            Duration::from_secs(10),
        ),
        (
            Duration::from_secs(61),
            Duration::from_secs(600),
            Duration::from_secs(10),
        ),
        (
            Duration::from_secs(5),
            Duration::from_secs(86401),
            Duration::from_secs(10),
        ),
        (
            Duration::from_secs(5),
            Duration::from_millis(1500),
            Duration::from_secs(10),
        ),
        (
            Duration::from_secs(5),
            Duration::from_secs(600),
            Duration::from_secs(61),
        ),
    ] {
        assert!(RelayBudgets::new(durations.0, durations.1, durations.2).is_err());
    }
}

#[rstest::rstest]
#[case("backend")]
#[case("zip")]
#[case("runtime")]
#[case("handler")]
#[case("nil_revision")]
#[case("revision_case")]
#[case("name")]
#[case("name_length")]
#[case("alias")]
#[case("zero_version")]
#[case("padded_version")]
#[case("overflow_version")]
#[case("timeout_zero")]
#[case("timeout_excess")]
#[case("timeout_fraction")]
#[case("memory_low")]
#[case("memory_high")]
#[case("tmp_low")]
#[case("tmp_high")]
#[case("architecture")]
#[case("region_missing")]
#[case("region_mismatch")]
#[case("region_invalid")]
#[case("reserved_env")]
#[case("environment_bytes")]
#[case("environment_nul")]
#[case("empty_argv")]
fn test_should_reject_invalid_captured_revisions_before_any_daemon_operation(
    #[case] scenario: &str,
) {
    let mut revision = revision();
    match scenario {
        "backend" => revision.backend = ExecutorBackend::Native,
        "zip" => revision.artifact = DeploymentArtifact::Zip(ValidatedArtifact::fixture()),
        "runtime" => revision.runtime = Some("provided.al2023".into()),
        "handler" => revision.handler = Some("bootstrap".into()),
        "nil_revision" => revision.revision_id = Uuid::nil().to_string(),
        "revision_case" => revision.revision_id = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA".into(),
        "name" => revision.function_name = "bad/name".into(),
        "name_length" => revision.function_name = "x".repeat(65),
        "alias" => revision.qualifier = "prod".into(),
        "zero_version" => revision.qualifier = "0".into(),
        "padded_version" => revision.qualifier = "01".into(),
        "overflow_version" => revision.qualifier = "18446744073709551616".into(),
        "timeout_zero" => revision.timeout = Duration::ZERO,
        "timeout_excess" => revision.timeout = Duration::from_secs(901),
        "timeout_fraction" => revision.timeout = Duration::from_millis(1500),
        "memory_low" => revision.memory_mb = 127,
        "memory_high" => revision.memory_mb = 10241,
        "tmp_low" => revision.ephemeral_storage_mb = 511,
        "tmp_high" => revision.ephemeral_storage_mb = 10241,
        "architecture" => revision.architecture = Architecture::X86_64,
        "region_missing" => {
            revision.environment.remove("AWS_REGION");
        }
        "region_mismatch" => {
            revision
                .environment
                .insert("AWS_DEFAULT_REGION".into(), "different".into());
        }
        "region_invalid" => {
            revision
                .environment
                .insert("AWS_REGION".into(), "outside\nregion".into());
        }
        "reserved_env" => {
            revision
                .environment
                .insert("AWS_LAMBDA_RUNTIME_API".into(), "outside:9001".into());
        }
        "environment_bytes" => {
            revision
                .environment
                .insert("TOO_LARGE".into(), "x".repeat(4096));
        }
        "environment_nul" => {
            revision.environment.insert("BAD_VALUE".into(), "\0".into());
        }
        "empty_argv" => {
            revision.image_config = Some(ImageConfig {
                entry_point: Some(vec![]),
                command: Some(vec![]),
                working_directory: None,
            });
        }
        _ => panic!("unknown fixture scenario"),
    }
    assert!(plan(revision).is_err(), "{scenario}");
}

#[tokio::test]
async fn test_should_retain_the_real_directory_until_allocation_plan_is_released() {
    let root = tempfile::tempdir().unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let plan = EnvironmentPlan::new(
        directory,
        image_with("", vec![]),
        Arc::new(revision()),
        policy(),
    )
    .unwrap();
    assert!(matches!(
        DirectoryLease::acquire(root.path().into(), deadline()).await,
        Err(super::super::DirectoryLeaseError::InUse)
    ));
    drop(plan);
    assert!(
        DirectoryLease::acquire(root.path().into(), deadline())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn test_should_submit_exact_captured_platform_and_refuse_revoked_availability_before_io() {
    let mut daemon = super::super::tests::Daemon::new();
    let mut wire = daemon.wire().await;
    let plan = Arc::new(plan(revision()).unwrap());
    let worker = {
        let plan = plan.clone();
        let port = wire.port();
        tokio::spawn(async move { plan.create_relay(&port, deadline()).await })
    };
    let request = daemon.next().await;
    assert!(request.head.contains("platform=linux%2Farm64"));
    request
        .reply(201, b"{\"Id\":\"created\",\"Warnings\":[]}")
        .await;
    assert_eq!(worker.await.unwrap().unwrap().id, "created");
    let (release, _) = watch::channel(());
    let (stop, stopping) = watch::channel(false);
    let revoked = PinnedImage::new(
        image_with("", vec![]).resolved().clone(),
        4096.try_into().unwrap(),
        release,
        Arc::new(AtomicBool::new(true)),
        stopping,
    );
    let mut revision = revision();
    revision.artifact = DeploymentArtifact::Image(revoked);
    let plan = EnvironmentPlan::new(
        directory(),
        image_with("", vec![]),
        Arc::new(revision),
        policy(),
    )
    .unwrap();
    stop.send_replace(true);
    assert!(
        plan.create_function(&wire.port(), deadline())
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(20), daemon.requests.recv())
            .await
            .is_err()
    );
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[test]
fn test_should_normalize_inherited_duplicates_empty_values_and_deletion_markers_once() {
    let mut revision = revision();
    revision.artifact = DeploymentArtifact::Image(image_with(
        "1000",
        vec![
            "INHERITED=first".into(),
            "INHERITED=last".into(),
            "REMOVED=old".into(),
            "REMOVED".into(),
            "EMPTY=".into(),
            "TEST_VALUE=old".into(),
            "TEST_VALUE".into(),
        ],
    ));
    let plan = plan(revision).unwrap();
    let env = plan.bodies.function.env.as_ref().unwrap();
    for (key, expected) in [
        ("INHERITED", "INHERITED=last"),
        ("REMOVED", "REMOVED"),
        ("EMPTY", "EMPTY="),
        ("TEST_VALUE", "TEST_VALUE=secret-value"),
    ] {
        let matching: Vec<_> = env
            .iter()
            .filter(|entry| entry.split('=').next() == Some(key))
            .collect();
        assert_eq!(matching, vec![expected]);
    }
}

#[tokio::test]
async fn test_should_retain_both_live_image_leases_through_allocation_plan_lifetime() {
    let image = image_with("", vec![]);
    let (relay_release, mut relay_released) = watch::channel(());
    let (function_release, mut function_released) = watch::channel(());
    let (_, stopped) = watch::channel(false);
    let make = |release| {
        PinnedImage::new(
            image.resolved().clone(),
            4096.try_into().unwrap(),
            release,
            Arc::new(AtomicBool::new(true)),
            stopped.clone(),
        )
    };
    let relay = make(relay_release);
    let function = make(function_release);
    let mut revision = revision();
    revision.artifact = DeploymentArtifact::Image(function.clone());
    let plan =
        EnvironmentPlan::new(directory(), relay.clone(), Arc::new(revision), policy()).unwrap();
    drop(relay);
    drop(function);
    assert!(!relay_released.has_changed().unwrap());
    assert!(!function_released.has_changed().unwrap());
    drop(plan);
    let _ = timeout(Duration::from_secs(1), relay_released.changed())
        .await
        .unwrap();
    let _ = timeout(Duration::from_secs(1), function_released.changed())
        .await
        .unwrap();
    assert!(relay_released.has_changed().is_err());
    assert!(function_released.has_changed().is_err());
}
