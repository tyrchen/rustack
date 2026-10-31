use rstest::rstest;

use super::*;

fn credentials() -> LocalSdkCredentials {
    ("local-sdk-key".to_owned(), "private-sdk-value".to_owned())
        .try_into()
        .unwrap()
}
fn endpoint() -> DockerGatewayEndpoint {
    "http://127.0.0.1:4566".parse().unwrap()
}
#[rstest]
#[case("", "value")]
#[case("key", "")]
#[case("bad/key", "value")]
#[case("bad key", "value")]
#[case("é", "value")]
#[case("key", "bad value")]
#[case("key", "bad\nvalue")]
#[case("key", "bad\0value")]
#[case("key", "é")]
fn test_should_reject_invalid_sdk_credentials(#[case] access: &str, #[case] secret: &str) {
    assert!(LocalSdkCredentials::try_from((access.into(), secret.into())).is_err());
}
#[test]
fn test_should_bound_pair_and_never_fabricate_a_partial_sdk_pair() {
    assert!(LocalSdkCredentials::parse(None, None).unwrap().is_none());
    assert!(LocalSdkCredentials::parse(Some("key".into()), None).is_err());
    assert!(LocalSdkCredentials::parse(None, Some("secret".into())).is_err());
    for (access, secret, accepted) in [
        ("k".repeat(128), "s".repeat(256), true),
        ("k".repeat(129), "s".into(), false),
        ("k".into(), "s".repeat(257), false),
    ] {
        assert_eq!(
            LocalSdkCredentials::try_from((access, secret)).is_ok(),
            accepted
        );
    }
    let value = credentials();
    assert_eq!(format!("{value:?}"), "LocalSdkCredentials(<redacted>)");
}
#[test]
fn test_should_allocate_checked_nonzero_context_ids_without_reuse_or_wrap() {
    let counter = AtomicU64::new(1);
    assert_eq!(next_identity(&counter).unwrap().get(), 1);
    assert_eq!(next_identity(&counter).unwrap().get(), 2);
    for invalid in [0, u64::MAX] {
        let counter = AtomicU64::new(invalid);
        assert!(next_identity(&counter).is_err());
        assert_eq!(counter.load(Ordering::Relaxed), invalid);
    }
}
#[test]
fn test_should_override_sdk_values_and_retain_all_explicit_unset_markers() {
    let mut entries = vec![
        "BUSINESS=preserved".into(),
        "AWS_ACCESS_KEY_ID=poison".into(),
        "AWS_SECRET_ACCESS_KEY=poison".into(),
        "AWS_ENDPOINT_URL=http://127.0.0.1:9".into(),
        "AWS_ENDPOINT_URL_S3=http://127.0.0.1:9".into(),
        "AWS_ENDPOINT_URL_DYNAMODB=http://127.0.0.1:9".into(),
        "AWS_IGNORE_CONFIGURED_ENDPOINT_URLS=true".into(),
    ];
    entries.extend(UNSET.map(|key| format!("{key}=poison")));
    let result = environment(&credentials(), &endpoint(), entries).unwrap();
    for key in UNSET
        .into_iter()
        .chain(["AWS_ENDPOINT_URL_S3", "AWS_ENDPOINT_URL_DYNAMODB"])
    {
        assert_eq!(
            result
                .iter()
                .filter(|entry| entry.split('=').next() == Some(key))
                .collect::<Vec<_>>(),
            vec![key]
        );
    }
    for expected in [
        "BUSINESS=preserved",
        "AWS_ACCESS_KEY_ID=local-sdk-key",
        "AWS_SECRET_ACCESS_KEY=private-sdk-value",
        "AWS_ENDPOINT_URL=http://127.0.0.1:4566/",
        "AWS_IGNORE_CONFIGURED_ENDPOINT_URLS=false",
        "AWS_EC2_METADATA_DISABLED=true",
        "AWS_CONFIG_FILE=/dev/null",
        "AWS_SHARED_CREDENTIALS_FILE=/dev/null",
    ] {
        assert!(result.iter().any(|entry| entry == expected), "{expected}");
    }
    assert!(result.windows(2).all(|pair| pair.first() < pair.get(1)));
    assert!(result.iter().all(|entry| !entry.contains("poison")));
}
#[test]
fn test_should_preserve_maximum_valid_user_entry_and_bound_final_sdk_environment() {
    let maximum = format!("ZZ={}", "v".repeat(4094));
    let result = environment(&credentials(), &endpoint(), vec![maximum.clone()]).unwrap();
    assert!(result.contains(&maximum));
    for entries in [
        vec!["x".repeat(MAX_ENV_ENTRY_BYTES + 1)],
        (0..21)
            .map(|i| format!("KEY_{i}={}", "v".repeat(4090)))
            .collect(),
        (0..298).map(|i| format!("KEY_{i}=v")).collect(),
        vec!["AA=bad\0value".into()],
    ] {
        assert!(environment(&credentials(), &endpoint(), entries).is_err());
    }
    let result = environment(
        &credentials(),
        &endpoint(),
        (0..297).map(|i| format!("KEY_{i}=v")).collect(),
    )
    .unwrap();
    assert_eq!(result.len(), MAX_ENV_ENTRIES);
}
