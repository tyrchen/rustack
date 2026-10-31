//! Actual default SDK provider chain on a qualified local gateway.

use std::{env, ffi::OsString, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::{
    image_sdk,
    process::{self, Profile, Server},
};

pub(super) async fn exercise(wrong: bool) -> Result<()> {
    ensure!(
        cfg!(all(feature = "s3", feature = "dynamodb")),
        "Docker handler SDK resource requires explicit s3,dynamodb source features"
    );
    let profile = if wrong {
        Profile::SdkWrongCredentials
    } else {
        Profile::Sdk
    };
    image_sdk::exercise(
        profile,
        async |server: &mut Server| prepare(server, wrong).await,
        |observed| validate(observed, wrong),
    )
    .await
}
async fn prepare(server: &mut Server, wrong: bool) -> Result<Vec<OsString>> {
    let image = env::var("RUSTACK_HANDLER_SDK_IMAGE").context("explicit handler SDK image")?;
    server.track_image(&image).await?;
    server.ready().await?;
    let (owner, engine) = server.observation_identity().await?;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/workflows/handler_sdk_image.py");
    Ok(vec![
        script.into_os_string(),
        OsString::from(&server.endpoint),
        OsString::from(image),
        OsString::from(if process::architecture()? == "arm64" {
            "arm64"
        } else {
            "x86_64"
        }),
        env::var_os("RUSTACK_RUNTIME_DOCKER_SOCKET").context("SDK metadata socket")?,
        OsString::from(owner),
        OsString::from(engine),
        OsString::from(if wrong { "wrong" } else { "positive" }),
        env::var_os("RUSTACK_RUNTIME_GATEWAY_HOST").context("qualified gateway host")?,
    ])
}
fn validate(observed: &Value, wrong: bool) -> Result<()> {
    let cases = if wrong {
        json!([
            "explicit-invalid-s3-authorization",
            "explicit-invalid-dynamodb-authorization",
            "no-ambient-fallback"
        ])
    } else {
        json!([
            "quiet-body-abort",
            "local-s3-dynamodb-cold-warm",
            "authoritative-SDK-env",
            "metadata-secret-omission"
        ])
    };
    ensure!(
        observed["schemaVersion"] == 1
            && observed["mode"] == if wrong { "wrong" } else { "positive" }
            && observed["versions"] == json!({"s3":"3.1105.0","dynamodb":"3.1105.0"})
            && observed["cases"] == cases,
        "handler SDK semantic inventory differs"
    );
    eprintln!("actual handler SDK versions: {}", observed["versions"]);
    Ok(())
}
