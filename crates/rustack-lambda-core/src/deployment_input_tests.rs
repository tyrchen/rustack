use base64::{Engine, engine::general_purpose::STANDARD};
use rstest::rstest;
use rustack_lambda_model::input::{
    CreateFunctionInput, UpdateFunctionCodeInput, UpdateFunctionConfigurationInput,
};
use serde_json::json;
use tokio::sync::oneshot;

use super::*;

#[test]
fn test_should_allow_large_zip_only_at_the_selected_deployment_path() {
    let zip = "A".repeat(2 * 1024 * 1024);
    let code = serde_json::to_vec(&json!({"ZipFile":zip,"DryRun":true})).unwrap();
    let parsed: UpdateFunctionCodeInput = parse(&code, Profile::Code).unwrap();
    assert_eq!(parsed.zip_file.as_deref(), Some(zip.as_str()));
    let create =
        serde_json::to_vec(&json!({"FunctionName":"worker","Role":"role","Code":{"ZipFile":zip}}))
            .unwrap();
    let parsed: CreateFunctionInput = parse(&create, Profile::Create).unwrap();
    assert_eq!(parsed.code.zip_file.as_deref(), Some(zip.as_str()));
    assert!(parse::<UpdateFunctionCodeInput>(&code, Profile::Configuration).is_err());
    let wrong = serde_json::to_vec(&json!({"Environment":{"Variables":{"ZipFile":zip}}})).unwrap();
    assert!(parse::<UpdateFunctionConfigurationInput>(&wrong, Profile::Configuration).is_err());
}

#[rstest]
#[case(br#"{"ZipFile":"AA==","ZipFile":"AA=="}"#)]
#[case(br#"{"ZipFile":"AA==","\u005aipFile":"AA=="}"#)]
#[case(br#"{"DryRun":false,"DryRun":true}"#)]
#[case(br#"{"Architectures":[],"DryRun":"true"}"#)]
#[case(br#"{"DryRun":true,"SourceKMSKeyArn":"private-marker"}"#)]
#[case(br#"{"ZipFile":true}"#)]
#[case(br#"{"ZipFile":"unfinished}"#)]
fn test_should_reject_ambiguous_unknown_or_malformed_code_bodies(#[case] bytes: &[u8]) {
    let error = parse::<UpdateFunctionCodeInput>(bytes, Profile::Code).unwrap_err();
    assert_eq!(error.message, "Invalid or unsupported deployment request");
    assert!(!format!("{error:?}").contains("private-marker"));
}

#[test]
fn test_should_recognize_escaped_code_keys_and_reject_nested_duplicate_environment_keys() {
    let input = br#"{"FunctionName":"worker","Role":"role","\u0043ode":{"\u005aipFile":"AA=="}}"#;
    assert_eq!(
        parse::<CreateFunctionInput>(input, Profile::Create)
            .unwrap()
            .code
            .zip_file
            .as_deref(),
        Some("AA==")
    );
    let duplicate = br#"{"Environment":{"Variables":{"TOKEN":"first","\u0054OKEN":"second"}}}"#;
    assert!(parse::<UpdateFunctionConfigurationInput>(duplicate, Profile::Configuration).is_err());
}

#[test]
fn test_should_reject_oversized_keys_strings_depth_and_metadata_before_dispatch() {
    let body = serde_json::to_vec(&json!({"Description":"x".repeat(STRING_BYTES+1)})).unwrap();
    assert!(parse::<UpdateFunctionConfigurationInput>(&body, Profile::Configuration).is_err());
    let key = format!("{{\"{}\":0}}", "x".repeat(KEY_BYTES * 6 + 1));
    assert!(lexical(key.as_bytes()).is_err());
    let nested = format!("{}0{}", "[".repeat(DEPTH + 1), "]".repeat(DEPTH + 1));
    assert!(lexical(nested.as_bytes()).is_err());
    let members = format!("[{}0]", "0,".repeat(MEMBERS * 2 + 1));
    assert!(lexical(members.as_bytes()).is_err());
    let metadata = format!("{{{}\"DryRun\":true}}", " ".repeat(META_BYTES));
    assert!(parse::<UpdateFunctionCodeInput>(metadata.as_bytes(), Profile::Code).is_err());
}

#[rstest]
#[case("", true)]
#[case("AA==", true)]
#[case("AAA=", true)]
#[case("AAAA", true)]
#[case("AB==", false)]
#[case("AAB=", false)]
#[case("A===", false)]
#[case("AA=A", false)]
#[case("A", false)]
#[case("AAAA\n", false)]
fn test_should_validate_canonical_base64_without_decoding(#[case] data: &str, #[case] valid: bool) {
    assert_eq!(inline(data).is_ok(), valid);
}

#[test]
fn test_should_accept_exact_zip_size_and_reject_the_first_extra_decoded_byte() {
    let bytes = vec![0u8; crate::artifact::MAX_ZIP_BYTES];
    let encoded = STANDARD.encode(bytes);
    assert!(inline(&encoded).is_ok());
    let excess = "A".repeat(MAX_ZIP_BASE64_BYTES);
    assert!(inline(&excess).is_err());
}

#[tokio::test]
async fn test_should_retain_a_parser_permit_after_its_response_waiter_is_dropped() {
    static POOL: Semaphore = Semaphore::const_new(1);
    let (begun, started) = oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let waiter = tokio::spawn(run(&POOL, move || {
        begun.send(()).unwrap();
        blocked.recv().unwrap();
        Ok(())
    }));
    started.await.unwrap();
    waiter.abort();
    let _ = waiter.await;
    assert!(matches!(
        run(&POOL, || Ok(())).await.unwrap_err().code,
        LambdaErrorCode::TooManyRequestsException
    ));
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while POOL.available_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(run(&POOL, || Ok(())).await.is_ok());
}

#[tokio::test]
async fn test_should_surface_a_pure_parser_worker_panic_without_losing_capacity() {
    static POOL: Semaphore = Semaphore::const_new(1);
    let error = run::<()>(&POOL, || panic!("injected pure parser panic"))
        .await
        .unwrap_err();
    assert_eq!(error.code, LambdaErrorCode::ServiceException);
    assert_eq!(POOL.available_permits(), 1);
}

#[tokio::test]
async fn test_should_apply_strict_nested_deployment_dtos() {
    for body in [
        json!({"ImageConfig":{"Unsupported":true}}),
        json!({"Environment":{"Unknown":"private-marker"}}),
        json!({"VpcConfig":{"Unsupported":true}}),
        json!({"LoggingConfig":{"Unsupported":true}}),
    ] {
        assert!(
            parse::<UpdateFunctionConfigurationInput>(
                &serde_json::to_vec(&body).unwrap(),
                Profile::Configuration
            )
            .is_err()
        );
    }
    let invalid =
        json!({"FunctionName":"worker","Role":"role","Code":{"Unknown":"private-marker"}});
    assert!(
        parse::<CreateFunctionInput>(&serde_json::to_vec(&invalid).unwrap(), Profile::Create)
            .is_err()
    );
}

#[tokio::test]
async fn test_should_keep_capacity_with_a_completed_parse_result_until_it_is_consumed() {
    static POOL: Semaphore = Semaphore::const_new(1);
    let completed = run(&POOL, || Ok(7)).await.unwrap();
    assert_eq!(completed.0, 7);
    assert_eq!(POOL.available_permits(), 0);
    assert_eq!(
        run(&POOL, || Ok(())).await.unwrap_err().code,
        LambdaErrorCode::TooManyRequestsException
    );
    drop(completed);
    assert_eq!(POOL.available_permits(), 1);
}

#[tokio::test]
async fn test_should_reuse_admission_for_all_concurrent_inline_validators() {
    static POOL: Semaphore = Semaphore::const_new(4);
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let permit = POOL.try_acquire().unwrap();
        tasks.spawn(validate_inline("AA==".into(), Some(permit)));
    }
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap().unwrap());
    }
    assert_eq!(POOL.available_permits(), 0);
    drop(results);
    assert_eq!(POOL.available_permits(), 4);
}
