use rustack_lambda_runtime::{
    probe::TemporaryReport, protocol::Architecture as RuntimeArchitecture,
};
use serde_json::{Value, json};

use super::*;
use crate::docker::{
    DockerWire, EnvironmentConfig, EnvironmentManager, RelayBudgets,
    plan::tests::{image_with, policy},
    tests::Daemon,
};

pub(super) fn report(attempt: AttemptId) -> ResourceReport {
    ResourceReport::builder()
        .version(1)
        .attempt(attempt)
        .architecture(RuntimeArchitecture::Arm64)
        .cgroup_root(true)
        .memory_max(Some(128 * 1024 * 1024))
        .swap_max(Some(0))
        .pids_max(Some(256))
        .cpu_quota(Some(100_000))
        .cpu_period(100_000)
        .euid(1000)
        .egid(1000)
        .cap_effective(0)
        .no_new_privileges(true)
        .seccomp_mode(2)
        .root_readonly(true)
        .temporary(
            TemporaryReport::builder()
                .bytes(512 * 1024 * 1024)
                .mode(0o1777)
                .tmpfs(true)
                .nosuid(true)
                .nodev(true)
                .build(),
        )
        .build()
}
#[rstest::rstest]
#[case::version("version",json!(2),ProbeFailure::Identity)]
#[case::namespace("cgroupRoot",json!(false),ProbeFailure::Namespace)]
#[case::architecture("architecture",json!("x86_64"),ProbeFailure::Identity)]
#[case::memory("memoryMax",json!(134_217_729),ProbeFailure::Memory)]
#[case::unlimited_memory("memoryMax", Value::Null, ProbeFailure::Memory)]
#[case::swap("swapMax",json!(1),ProbeFailure::Swap)]
#[case::pids("pidsMax",json!(257),ProbeFailure::Pids)]
#[case::cpu("cpuQuota", Value::Null, ProbeFailure::Cpu)]
#[case::period("cpuPeriod",json!(0),ProbeFailure::Cpu)]
#[case::overflow("cpuPeriod",json!(u64::MAX),ProbeFailure::Cpu)]
#[case::root_uid("euid",json!(0),ProbeFailure::User)]
#[case::root_gid("egid",json!(0),ProbeFailure::User)]
#[case::capabilities("capEffective",json!(1),ProbeFailure::Capabilities)]
#[case::privileges("noNewPrivileges",json!(false),ProbeFailure::Privilege)]
#[case::seccomp("seccompMode",json!(0),ProbeFailure::Seccomp)]
#[case::rootfs("rootReadonly",json!(false),ProbeFailure::Root)]
#[case::tmp_size("temporary",json!({"bytes":1,"mode":1023,"tmpfs":true,"nosuid":true,"nodev":true}),ProbeFailure::Temporary)]
#[case::tmp_flags("temporary",json!({"bytes":536_870_912,"mode":1023,"tmpfs":true,"nosuid":false,"nodev":true}),ProbeFailure::Temporary)]
#[test]
fn test_should_reject_each_actual_policy_mismatch(
    #[case] field: &str,
    #[case] value: Value,
    #[case] failure: ProbeFailure,
) {
    let attempt = AttemptId::default();
    let mut report = serde_json::to_value(report(attempt)).unwrap();
    report[field] = value;
    let decoded = decode_report(&serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(
        verify(&decoded, attempt, Architecture::Arm64, &policy()),
        Err(failure)
    );
}
#[test]
fn test_should_verify_exact_attempt_and_checked_cpu_rounding_and_bound_reports() {
    let attempt = AttemptId::default();
    let mut observed = report(attempt);
    verify(&observed, attempt, Architecture::Arm64, &policy()).unwrap();
    assert_eq!(
        verify(
            &observed,
            AttemptId::default(),
            Architecture::Arm64,
            &policy()
        ),
        Err(ProbeFailure::Identity)
    );
    let policy = ContainerPolicy::new(
        "none".parse().unwrap(),
        1000,
        1000,
        333_333_333,
        RelayBudgets::default(),
    )
    .unwrap();
    observed.cpu_quota = Some(33_333);
    verify(&observed, attempt, Architecture::Arm64, &policy).unwrap();
    assert!(decode_report(&vec![b' '; REPORT_BYTES + 1]).is_err());
    let mut raw = serde_json::to_value(&observed).unwrap();
    raw["unknown"] = json!(true);
    assert!(decode_report(&serde_json::to_vec(&raw).unwrap()).is_err());
    assert!(decode_report(br#"{"version":1,"version":1}"#).is_err());
    assert!(ProbeSettings::new(Duration::ZERO, Duration::from_secs(1)).is_err());
    assert!(ProbeSettings::new(Duration::from_secs(1), Duration::from_millis(1500)).is_err());
}
#[rstest::rstest]
#[case::rejected(false)]
#[case::abandoned(true)]
#[tokio::test]
async fn test_should_settle_rejected_probe_creation_and_retain_abandoned_waiter_cleanup(
    #[case] abandoned: bool,
) {
    let mut daemon = Daemon::new();
    let mut wire: DockerWire = daemon.wire().await;
    let directory = DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4());
    let mut environments =
        EnvironmentManager::start(wire.port(), directory.clone(), EnvironmentConfig::default())
            .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let network = "none"
        .parse::<super::super::NetworkSelection>()
        .unwrap()
        .qualify(&wire.port(), deadline)
        .await
        .unwrap();
    let config = EnvironmentProbeConfig::builder()
        .directory(directory)
        .relay(image_with("1000", vec![]))
        .environments(environments.port())
        .policy(policy())
        .network(network)
        .build();
    let mut foreign = config.clone();
    foreign.directory = DirectoryLease::testing(Uuid::new_v4(), Uuid::new_v4());
    assert!(matches!(
        probe_environment(foreign, deadline).await,
        Err(EnvironmentProbeError::Configuration)
    ));
    let task = tokio::spawn(probe_environment(config, deadline));
    let creation = daemon.next().await;
    if abandoned {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(environments.port().status().allocated, 1);
        creation.reply(400, b"{}").await;
    } else {
        creation.reply(400, b"{}").await;
        assert!(matches!(
            task.await.unwrap(),
            Err(EnvironmentProbeError::Environment(_))
        ));
        assert_eq!(environments.port().status().allocated, 0);
    }
    assert!(daemon.requests.try_recv().is_err());
    environments.shutdown(deadline).await.unwrap();
    assert_eq!(environments.port().status().allocated, 0);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}
