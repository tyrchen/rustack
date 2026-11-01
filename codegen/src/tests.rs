//! Generator protocol-policy tests independent of generated runtime fixtures.

use serde_json::json;

use crate::{
    codegen::generate_all,
    config::{CustomError, ServiceConfig, ServiceConfigFile},
    model::SmithyModel,
    shapes::resolve_model,
};

fn config(protocol: &str, strict: bool) -> ServiceConfig {
    let text = format!(
        r#"
[service]
name="fixture"
display_name="Fixture"
rust_prefix="Fixture"
namespace="fixture"
protocol="{protocol}"
[protocol]
serde_rename="camelCase"
emit_serde_derives=true
[operations]
all=["Run"]
[output]
file_layout="flat"
strict_model={strict}
"#
    );
    let file: ServiceConfigFile = toml::from_str(&text).unwrap();
    ServiceConfig::from_file(file).unwrap()
}
fn model(target: Option<&str>, member: Option<&str>, collection: bool) -> SmithyModel {
    let mut value = json!({"smithy":"2.0","shapes":{
        "fixture#Run":{"type":"operation","input":{"target":"fixture#RunRequest"}},
        "fixture#RunRequest":{"type":"structure","members":{
            "required":{"target":"fixture#Time","traits":{"smithy.api#required":{}}},
            "optional":{"target":"fixture#Time"},
            "plain":{"target":"smithy.api#String"}}},
        "fixture#Time":{"type":"timestamp","traits":{}}
    }});
    if let Some(format) = target {
        value["shapes"]["fixture#Time"]["traits"]["smithy.api#timestampFormat"] = json!(format);
    }
    if let Some(format) = member {
        value["shapes"]["fixture#RunRequest"]["members"]["required"]["traits"]
            ["smithy.api#timestampFormat"] = json!(format);
    }
    if collection {
        value["shapes"]["fixture#Times"] =
            json!({"type":"list","member":{"target":"fixture#Time"}});
        value["shapes"]["fixture#RunRequest"]["members"]["times"] =
            json!({"target":"fixture#Times"});
    }
    serde_json::from_value(value).unwrap()
}
#[test]
fn test_should_emit_numeric_codecs_only_for_json_protocol_defaults() {
    for protocol in [
        "awsJson1_0",
        "awsJson1_1",
        "restJson1",
        "awsQuery",
        "restXml",
    ] {
        let config = config(protocol, true);
        let resolved = resolve_model(&model(None, None, false), &config).unwrap();
        let files = generate_all(&resolved, &config).unwrap();
        let numeric = protocol.starts_with("awsJson");
        assert_eq!(files.contains_key("epoch_seconds.rs"), numeric);
        assert_eq!(
            files["input.rs"].contains("with = \"crate::epoch_seconds"),
            numeric
        );
        if numeric {
            assert!(files["input.rs"].contains("default, with = \"crate::epoch_seconds::option\""));
        }
    }
}
#[test]
fn test_should_resolve_member_override_before_shape_before_protocol_default() {
    let config = config("awsJson1_0", true);
    for (target, member, expected) in [
        (Some("date-time"), Some("epoch-seconds"), "epoch-seconds"),
        (Some("epoch-seconds"), Some("date-time"), "date-time"),
        (Some("date-time"), None, "date-time"),
        (None, None, ""),
    ] {
        let resolved = resolve_model(&model(target, member, false), &config).unwrap();
        let field = resolved.input_structs["RunInput"]
            .iter()
            .find(|field| field.smithy_name == "required")
            .unwrap();
        assert_eq!(field.timestamp_format.as_deref(), Some(expected));
        let files = generate_all(&resolved, &config).unwrap();
        let required = files["input.rs"].split("pub required:").next().unwrap();
        assert_eq!(
            required
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap()
                .contains("with = \"crate::epoch_seconds\""),
            expected != "date-time"
        );
    }
}
#[test]
fn test_should_reject_unsupported_explicit_formats_and_unadapted_collections() {
    let config = config("awsJson1_0", true);
    for model in [
        model(None, Some("http-date"), false),
        model(None, None, true),
    ] {
        let resolved = resolve_model(&model, &config).unwrap();
        assert!(generate_all(&resolved, &config).is_err());
    }
}
#[test]
fn test_should_keep_strict_models_redacted_documented_and_without_blanket_ignores() {
    let config = config("awsJson1_0", true);
    let resolved = resolve_model(&model(None, None, false), &config).unwrap();
    let files = generate_all(&resolved, &config).unwrap();
    assert!(files["input.rs"].contains("#[serde(deny_unknown_fields)]"));
    assert!(files["input.rs"].contains("#[non_exhaustive]"));
    assert!(files["input.rs"].contains("impl std::fmt::Debug for RunInput"));
    assert!(files["input.rs"].contains("/// The `required` wire member."));
    assert!(files.values().all(|file| !file.contains("#[allow(")
        && !file.contains("#![allow(")
        && !file.contains("```ignore")));
    assert_eq!(files, generate_all(&resolved, &config).unwrap());
}

#[test]
fn test_should_emit_one_redacted_debug_without_serde_derives() {
    for protocol in ["restXml", "awsJson1_0", "awsJson1_1"] {
        let mut config = config(protocol, true);
        config.emit_serde_derives = false;
        let resolved = resolve_model(&model(None, None, false), &config).unwrap();
        let files = generate_all(&resolved, &config).unwrap();
        assert!(files["input.rs"].contains("#[derive(Clone, Default)]"));
        assert!(!files["input.rs"].contains("derive(Debug"));
        assert_eq!(
            files["input.rs"]
                .matches("impl std::fmt::Debug for RunInput")
                .count(),
            1
        );
        assert!(!files.contains_key("epoch_seconds.rs"));
        assert!(!files["lib.rs"].contains("mod epoch_seconds"));
    }
}

#[test]
fn test_should_preserve_payment_required_and_request_timeout_error_statuses() {
    let mut config = config("awsJson1_0", true);
    for (name, status) in [("Quota", 402), ("Timeout", 408)] {
        config.custom_errors.insert(
            name.into(),
            CustomError {
                status,
                message: "static".into(),
            },
        );
    }
    let resolved = resolve_model(&model(None, None, false), &config).unwrap();
    let files = generate_all(&resolved, &config).unwrap();
    assert!(files["error.rs"].contains("Self::Quota => http::StatusCode::PAYMENT_REQUIRED"));
    assert!(files["error.rs"].contains("Self::Timeout => http::StatusCode::REQUEST_TIMEOUT"));
}
