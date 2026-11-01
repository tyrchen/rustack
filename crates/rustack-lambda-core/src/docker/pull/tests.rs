use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE};
use http::StatusCode;
use rstest::rstest;
use secrecy::ExposeSecret;
use serde_json::{Value, from_slice};

use super::{ImagePullPolicy, RegistryCredential, RegistryCredentials, RegistryHost};
use crate::docker::DockerError;

mod preparation;
mod real;
mod wire;

#[rstest]
#[case("")]
#[case("https://registry.example")]
#[case("registry.example/path")]
#[case("user@registry.example")]
#[case("*.example")]
#[case("registry")]
#[case("registry.example:0")]
#[case("registry.example:65536")]
#[case("registry.example\n")]
#[case("-registry.example")]
#[case("registry-.example")]
#[case("registry.-example")]
fn test_should_reject_unsafe_ambiguous_registry_hosts(#[case] host: &str) {
    assert!(host.parse::<RegistryHost>().is_err());
}
#[rstest]
#[case("registry.example")]
#[case("localhost:5000")]
#[case("[::1]:5000")]
fn test_should_validate_and_redact_exact_registry_hosts(#[case] host: &str) {
    let parsed: RegistryHost = host.parse().unwrap();
    assert_eq!(parsed.as_str(), host);
    assert!(!format!("{parsed:?}").contains(host));
}
#[test]
fn test_should_canonicalize_dns_case_and_only_documented_default_hub_alias() {
    assert_eq!(
        "INDEX.DOCKER.IO".parse::<RegistryHost>().unwrap(),
        "docker.io".parse().unwrap()
    );
    assert_eq!(
        "REGISTRY.example:5000"
            .parse::<RegistryHost>()
            .unwrap()
            .as_str(),
        "registry.example:5000"
    );
    assert!("a".repeat(254).parse::<RegistryHost>().is_err());
}
#[test]
fn test_should_bound_and_redact_credentials_with_padded_docker_auth_encoding() {
    let host = "registry.example".parse().unwrap();
    let credential =
        RegistryCredential::basic("PRIVATE-USER".into(), "PRIVATE-PASSWORD".into()).unwrap();
    let header = credential.header(&host).unwrap();
    let json: Value = from_slice(&URL_SAFE.decode(header.expose_secret()).unwrap()).unwrap();
    assert_eq!(json["username"], "PRIVATE-USER");
    assert_eq!(json["password"], "PRIVATE-PASSWORD");
    assert_eq!(json["serveraddress"], "registry.example");
    assert!(!format!("{credential:?} {header:?}").contains("PRIVATE"));
    assert!(RegistryCredential::basic("u".repeat(257).into(), "p".into()).is_err());
    assert!(RegistryCredential::basic("u".into(), "p".repeat(4097).into()).is_err());
    assert!(RegistryCredential::identity_token("t".repeat(8193).into()).is_err());
    assert!(RegistryCredential::identity_token("t\n".into()).is_err());
    assert!(RegistryCredential::basic("".into(), "p".into()).is_err());
    let token = RegistryCredential::identity_token("t".repeat(8192).into()).unwrap();
    assert!(token.header(&host).unwrap().expose_secret().len() <= 32 * 1024);
}
fn policy(host: &str) -> ImagePullPolicy {
    ImagePullPolicy::if_missing(
        vec![host.parse().unwrap()],
        RegistryCredentials::default(),
        Duration::from_secs(10),
    )
    .unwrap()
}
#[test]
fn test_should_require_exact_host_port_allowlist_and_never_pull_internal_aliases() {
    let configured = policy("registry.example:5000");
    assert!(
        configured
            .authorize(&"registry.example:5000/team/fn:v1".parse().unwrap())
            .is_ok()
    );
    for source in [
        "registry.example:5001/team/fn:v1",
        "registry.example.evil/team/fn:v1",
        "evilregistry.example/team/fn:v1",
        "worker:v1",
    ] {
        assert!(matches!(
            configured.authorize(&source.parse().unwrap()),
            Err(DockerError::PullDenied)
        ));
    }
    let hub = policy("docker.io");
    assert!(hub.authorize(&"worker:v1".parse().unwrap()).is_ok());
    for source in [
        "rustack-internal/lambda-owner/run:pin",
        "docker.io/rustack-internal/run:pin",
        "index.docker.io/rustack-internal/run:pin",
    ] {
        assert!(matches!(
            hub.authorize(&source.parse().unwrap()),
            Err(DockerError::PullDenied)
        ));
    }
    assert!(matches!(
        ImagePullPolicy::default().authorize(&"worker:v1".parse().unwrap()),
        Err(DockerError::Status(StatusCode::NOT_FOUND))
    ));
}
#[test]
fn test_should_reject_duplicate_unapproved_or_excessive_policy_records_and_budgets() {
    let host: RegistryHost = "registry.example".parse().unwrap();
    assert!(
        ImagePullPolicy::if_missing(
            vec![],
            RegistryCredentials::default(),
            Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(
        ImagePullPolicy::if_missing(
            vec![host.clone(), host.clone()],
            RegistryCredentials::default(),
            Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(
        ImagePullPolicy::if_missing(
            vec![host.clone(); 33],
            RegistryCredentials::default(),
            Duration::from_secs(1)
        )
        .is_err()
    );
    for timeout in [
        Duration::ZERO,
        Duration::from_millis(1500),
        Duration::from_secs(601),
    ] {
        assert!(
            ImagePullPolicy::if_missing(
                vec![host.clone()],
                RegistryCredentials::default(),
                timeout
            )
            .is_err()
        );
    }
    let make = || RegistryCredential::identity_token("secret".into()).unwrap();
    assert!(
        RegistryCredentials::try_from(vec![(host.clone(), make()), (host.clone(), make())])
            .is_err()
    );
    let map = RegistryCredentials::try_from(vec![(host, make())]).unwrap();
    assert!(
        ImagePullPolicy::if_missing(
            vec!["other.example".parse().unwrap()],
            map,
            Duration::from_secs(1)
        )
        .is_err()
    );
    let records: Vec<(RegistryHost, RegistryCredential)> = (0..9)
        .map(|i| {
            (
                format!("registry{i}.example").parse().unwrap(),
                RegistryCredential::identity_token("x".repeat(8192).into()).unwrap(),
            )
        })
        .collect();
    assert!(RegistryCredentials::try_from(records).is_err());
}
