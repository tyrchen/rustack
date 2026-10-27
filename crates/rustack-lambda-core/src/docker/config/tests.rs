use std::{collections::BTreeMap, time::Duration};

use rstest::rstest;
use rustack_core::settings::{RuntimeBudgets, ValidatedSettings};

use super::{DockerGatewayEndpoint, DockerRuntimeSettings, RegistryAuthFile, cpu};

fn settings(extra: &[(&str, &str)]) -> Result<DockerRuntimeSettings, crate::docker::DockerError> {
    let mut values = BTreeMap::from([
        ("LAMBDA_DOCKER_RELAY_IMAGE", "private-relay:v1"),
        ("LAMBDA_DOCKER_NETWORK", "none"),
    ]);
    values.extend(extra.iter().copied());
    DockerRuntimeSettings::from_lookup(
        |key| values.get(key).map(|value| (*value).into()),
        &RuntimeBudgets::default(),
        Duration::from_secs(600),
    )
}
#[rstest]
#[case("0.1", 100_000_000)]
#[case("1", 1_000_000_000)]
#[case("1.234567891", 1_234_567_891)]
#[case("16.000000000", 16_000_000_000)]
fn test_should_convert_cpu_decimal_without_rounding(#[case] raw: &str, #[case] expected: u64) {
    assert_eq!(cpu(raw).unwrap(), expected);
}
#[rstest]
#[case("")]
#[case("0")]
#[case("0.099999999")]
#[case("16.000000001")]
#[case("1.1234567891")]
#[case("1e0")]
#[case("NaN")]
#[case("+1")]
#[case("-1")]
#[case("1.")]
#[case(".5")]
#[case(" 1")]
#[case("1..1")]
#[case("999999999999")]
fn test_should_reject_invalid_or_out_of_range_cpu(#[case] raw: &str) {
    assert!(cpu(raw).is_err());
}
#[rstest]
#[case("http://localhost:4566")]
#[case("https://gateway.example")]
#[case("http://[::1]:4566/")]
fn test_should_validate_operator_endpoint_without_network_io(#[case] raw: &str) {
    let endpoint: DockerGatewayEndpoint = raw.parse().unwrap();
    assert!(!format!("{endpoint:?}").contains("localhost"));
}
#[rstest]
#[case("http://user:password@localhost:4566")]
#[case("http://@localhost:4566")]
#[case("http://:password@localhost:4566")]
#[case("http://localhost:0")]
#[case("http://localhost/path")]
#[case("http://localhost/?")]
#[case("http://localhost/#")]
#[case("ftp://localhost/")]
#[case("http:localhost")]
#[case("http://localhost\\")]
#[case("http://localhost\n")]
#[case(" http://localhost")]
#[case("http://localhost ")]
#[case("http://localhost/a/..")]
#[case("http://localhost/%2e")]
fn test_should_reject_ambiguous_userinfo_or_nonroot_endpoints(#[case] raw: &str) {
    assert!(raw.parse::<DockerGatewayEndpoint>().is_err());
}
#[test]
fn test_should_bound_auth_paths_and_endpoint_bytes() {
    for raw in [
        "relative.yaml",
        "~/auth.yaml",
        "/private/../auth.yaml",
        "/",
        "/private/auth.yaml/",
        "/private/secret\0.yaml",
    ] {
        assert!(raw.parse::<RegistryAuthFile>().is_err());
    }
    assert!(
        format!("/{}", "a".repeat(4096))
            .parse::<RegistryAuthFile>()
            .is_err()
    );
    assert!(
        format!("http://{}.example/", "a".repeat(2048))
            .parse::<DockerGatewayEndpoint>()
            .is_err()
    );
    let path: RegistryAuthFile = "/private/auth.yaml".parse().unwrap();
    assert!(!format!("{path:?}").contains("auth.yaml"));
}
#[test]
fn test_should_require_explicit_runtime_sources_and_validate_every_operator_limit() {
    assert!(
        DockerRuntimeSettings::from_lookup(
            |_| None,
            &RuntimeBudgets::default(),
            Duration::from_secs(600)
        )
        .is_err()
    );
    for (key, value) in [
        ("LAMBDA_DOCKER_SOCKET", "tcp://localhost:2375"),
        ("LAMBDA_DOCKER_RELAY_IMAGE", "bad image"),
        ("LAMBDA_DOCKER_NETWORK", "host"),
        ("LAMBDA_DOCKER_NETWORK", "bridge"),
        ("LAMBDA_DOCKER_UID", "0"),
        ("LAMBDA_DOCKER_GID", "2147483648"),
        ("LAMBDA_DOCKER_UID", "+1"),
        ("LAMBDA_DOCKER_CPU", "17"),
        ("LAMBDA_DOCKER_LOG_RING_BYTES", "4095"),
        ("LAMBDA_DOCKER_LOG_RING_BYTES", "1048577"),
        ("LAMBDA_DOCKER_CROSS_ARCHITECTURE", "yes"),
        ("LAMBDA_DOCKER_ZIP_IMAGE", "bad image"),
        ("LAMBDA_DOCKER_GATEWAY_ENDPOINT", "http://@localhost"),
    ] {
        assert!(settings(&[(key, value)]).is_err(), "{key}");
    }
}
#[test]
fn test_should_capture_settings_from_existing_shared_yaml_and_redact_runtime_values() {
    let shared = ValidatedSettings::parse(
        "environment:\n  LAMBDA_DOCKER_RELAY_IMAGE: private-relay:v1\n  LAMBDA_DOCKER_NETWORK: \
         none\n  LAMBDA_DOCKER_CPU: '0.125'\n  LAMBDA_DOCKER_CROSS_ARCHITECTURE: yes\nbudgets:\n  \
         lambdaDockerPullSeconds: 600",
        &BTreeMap::new(),
    )
    .unwrap();
    let runtime = DockerRuntimeSettings::from_lookup(
        |key| shared.get(key).map(str::to_owned),
        shared.budgets(),
        Duration::from_secs(600),
    )
    .unwrap();
    assert!(runtime.cross_architecture());
    assert_eq!(runtime.budgets().lambda_docker_pull_seconds, 600);
    assert_eq!(runtime.log_ring_bytes(), 262_144);
    assert_eq!(runtime.relay_image().as_str(), "private-relay:v1");
    assert!(runtime.zip_image().is_none());
    assert!(runtime.gateway_endpoint().is_none());
    assert!(!format!("{runtime:?}").contains("private-relay"));
}
#[test]
fn test_should_require_exact_if_missing_allowlist_without_ignored_settings() {
    for values in [
        vec![("LAMBDA_DOCKER_REGISTRIES", "registry.example")],
        vec![("LAMBDA_DOCKER_REGISTRY_AUTH_FILE", "/private/auth.yaml")],
        vec![("LAMBDA_DOCKER_PULL_POLICY", "always")],
        vec![("LAMBDA_DOCKER_PULL_POLICY", "ifMissing")],
        vec![
            ("LAMBDA_DOCKER_PULL_POLICY", "ifMissing"),
            ("LAMBDA_DOCKER_REGISTRIES", ""),
        ],
        vec![
            ("LAMBDA_DOCKER_PULL_POLICY", "ifMissing"),
            (
                "LAMBDA_DOCKER_REGISTRIES",
                "registry.example,registry.example",
            ),
        ],
        vec![
            ("LAMBDA_DOCKER_PULL_POLICY", "ifMissing"),
            (
                "LAMBDA_DOCKER_REGISTRIES",
                "registry.example, other.example",
            ),
        ],
    ] {
        assert!(settings(&values).is_err());
    }
    assert!(
        settings(&[
            ("LAMBDA_DOCKER_PULL_POLICY", "ifMissing"),
            (
                "LAMBDA_DOCKER_REGISTRIES",
                "registry.example,public.ecr.aws"
            )
        ])
        .is_ok()
    );
}

#[test]
fn test_should_bound_registry_csv_before_collecting_and_accept_full_documented_capacity() {
    let hosts = (0..32)
        .map(|i| {
            format!(
                "{i:02}{}.{}.{}.{}",
                "a".repeat(61),
                "b".repeat(63),
                "c".repeat(63),
                "d".repeat(61)
            )
        })
        .collect::<Vec<_>>();
    assert!(hosts.iter().all(|host| host.len() == 253));
    let csv = hosts.join(",");
    assert_eq!(csv.len(), 8127);
    let shared = ValidatedSettings::parse(
        "",
        &BTreeMap::from([("LAMBDA_DOCKER_REGISTRIES".into(), csv.clone())]),
    )
    .unwrap();
    assert_eq!(shared.get("LAMBDA_DOCKER_REGISTRIES"), Some(csv.as_str()));
    assert!(
        settings(&[
            ("LAMBDA_DOCKER_PULL_POLICY", "ifMissing"),
            ("LAMBDA_DOCKER_REGISTRIES", &csv)
        ])
        .is_ok()
    );
    let csv = (0..33)
        .map(|i| format!("r{i}.example"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(
        settings(&[
            ("LAMBDA_DOCKER_PULL_POLICY", "ifMissing"),
            ("LAMBDA_DOCKER_REGISTRIES", &csv)
        ])
        .is_err()
    );
}
