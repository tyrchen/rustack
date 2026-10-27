//! Signed ordinary SDK calls through the actual production startup consumer.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use aws_credential_types::Credentials;
use aws_sdk_lambda::{
    Client,
    config::{BehaviorVersion, Region, retry::RetryConfig, timeout::TimeoutConfig},
    primitives::Blob,
    types::{Architecture, FunctionCode, ImageConfig, PackageType, Runtime, State},
};
use aws_sdk_sfn::{Client as States, error::ProvideErrorMetadata};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};
use uuid::Uuid;

fn timeouts() -> TimeoutConfig {
    TimeoutConfig::builder()
        .connect_timeout(Duration::from_secs(2))
        .read_timeout(Duration::from_secs(60))
        .operation_timeout(Duration::from_secs(60))
        .build()
}
fn credentials() -> Credentials {
    Credentials::new("local-key", "local-secret", None, None, "docker-fixture")
}
fn lambda(endpoint: &str) -> Client {
    Client::from_conf(
        aws_sdk_lambda::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(endpoint)
            .credentials_provider(credentials())
            .retry_config(RetryConfig::disabled())
            .timeout_config(timeouts())
            .build(),
    )
}
async fn active(client: &Client, name: &str, expected: &str) -> Result<()> {
    let at = Instant::now() + Duration::from_secs(120);
    loop {
        let config = client
            .get_function_configuration()
            .function_name(name)
            .send()
            .await?;
        let state = config.state().map(State::as_str);
        if state == Some(expected) {
            return Ok(());
        }
        ensure!(
            state != Some("Failed") && Instant::now() < at,
            "deployment did not reach expected state"
        );
        sleep(Duration::from_millis(25)).await;
    }
}
async fn invoke(client: &Client, name: &str, event: Value) -> Result<(Value, Option<String>)> {
    let result = client
        .invoke()
        .function_name(name)
        .payload(Blob::new(serde_json::to_vec(&event)?))
        .send()
        .await?;
    ensure!(result.status_code() == 200, "Invoke status");
    let value = serde_json::from_slice(result.payload().context("Invoke payload")?.as_ref())?;
    Ok((value, result.function_error().map(str::to_owned)))
}
pub(super) async fn exercise(
    endpoint: &str,
    image: &str,
    architecture: &str,
    workflow: bool,
    preserve: bool,
) -> Result<()> {
    let client = lambda(endpoint);
    let name = format!("DockerStartup{}", Uuid::new_v4().simple());
    let selected = if architecture == "arm64" {
        Architecture::Arm64
    } else {
        Architecture::X8664
    };
    let created = client
        .create_function()
        .function_name(&name)
        .role("arn:aws:iam::000000000000:role/Docker")
        .package_type(PackageType::Image)
        .architectures(selected.clone())
        .memory_size(128)
        .timeout(30)
        .code(FunctionCode::builder().image_uri(image).build())
        .send()
        .await?;
    let arn = created.function_arn().context("Image ARN")?.to_owned();
    active(&client, &name, "Active").await?;
    for (value, counter) in [(35, 1), (0, 2)] {
        let (output, error) = invoke(&client, &name, json!({"value":value})).await?;
        ensure!(
            error.is_none()
                && output["computed"] == value + 7
                && output["counter"] == counter
                && output["uid"] == 1000,
            "real computed/warm/user evidence differs"
        );
        ensure!(
            output["requestId"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "missing RIC request identity"
        );
    }
    let version = client.publish_version().function_name(&name).send().await?;
    let version = version.version().context("published version")?;
    client
        .create_alias()
        .function_name(&name)
        .name("live")
        .function_version(version)
        .send()
        .await?;
    let alias = client
        .invoke()
        .function_name(&name)
        .qualifier("live")
        .payload(Blob::new(br#"{"value":1}"#.as_slice()))
        .send()
        .await?;
    ensure!(
        alias.executed_version() == Some(version) && alias.function_error().is_none(),
        "published alias did not execute captured image"
    );
    let (_, failure) = invoke(&client, &name, json!({"fail":true,"value":1})).await?;
    ensure!(
        failure.as_deref() == Some("Unhandled"),
        "FunctionError was not distinct from HTTP success"
    );
    let (wrong_name, stalled) = failure_cases(&client, image, selected, &name).await?;
    if workflow {
        workflow_image(endpoint, &arn).await?;
    }
    if !preserve {
        client
            .delete_function()
            .function_name(&stalled)
            .send()
            .await?;
        client
            .delete_function()
            .function_name(&wrong_name)
            .send()
            .await?;
        client.delete_function().function_name(&name).send().await?;
    }
    Ok(())
}
async fn failure_cases(
    client: &Client,
    image: &str,
    selected: Architecture,
    name: &str,
) -> Result<(String, String)> {
    let zip = client
        .create_function()
        .function_name(format!("{name}Zip"))
        .role("arn:aws:iam::000000000000:role/Docker")
        .runtime(Runtime::Providedal2023)
        .handler("bootstrap")
        .code(
            FunctionCode::builder()
                .zip_file(Blob::new(b"unextracted".as_slice()))
                .build(),
        )
        .send()
        .await
        .expect_err("Docker ZIP must reject before extraction");
    ensure!(
        zip.as_service_error().and_then(|error| error.code())
            == Some("InvalidParameterValueException"),
        "wrong Docker ZIP error"
    );
    let wrong_name = format!("{name}OtherArch");
    client
        .create_function()
        .function_name(&wrong_name)
        .role("arn:aws:iam::000000000000:role/Docker")
        .package_type(PackageType::Image)
        .architectures(if selected == Architecture::Arm64 {
            Architecture::X8664
        } else {
            Architecture::Arm64
        })
        .code(FunctionCode::builder().image_uri(image).build())
        .send()
        .await?;
    active(client, &wrong_name, "Failed").await?;
    let stalled = format!("{name}NoRic");
    client
        .create_function()
        .function_name(&stalled)
        .role("arn:aws:iam::000000000000:role/Docker")
        .package_type(PackageType::Image)
        .architectures(selected)
        .code(FunctionCode::builder().image_uri(image).build())
        .image_config(
            ImageConfig::builder()
                .entry_point("/var/lang/bin/python3")
                .entry_point("-c")
                .entry_point("import time;time.sleep(60)")
                .set_command(Some(Vec::new()))
                .build(),
        )
        .send()
        .await?;
    active(client, &stalled, "Active").await?;
    let started = Instant::now();
    let result = client
        .invoke()
        .function_name(&stalled)
        .payload(Blob::new(b"{}".as_slice()))
        .send()
        .await;
    let error = result.expect_err("missing RIC must fail invocation");
    ensure!(
        started.elapsed() < Duration::from_secs(45)
            && error
                .as_service_error()
                .and_then(|error| error.message())
                .is_some_and(|message| message.contains("bootstrap initialization deadline")),
        "independent bootstrap timeout was ignored or misclassified"
    );
    Ok((wrong_name, stalled))
}
async fn workflow_image(endpoint: &str, function: &str) -> Result<()> {
    let client = States::from_conf(
        aws_sdk_sfn::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(endpoint)
            .credentials_provider(credentials())
            .retry_config(RetryConfig::disabled())
            .timeout_config(timeouts())
            .build(),
    );
    let machine=client.create_state_machine().name(format!("DockerWorkflow{}",Uuid::new_v4().simple())).role_arn("arn:aws:iam::000000000000:role/Docker")
        .definition(json!({"StartAt":"Compute","States":{"Compute":{"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","Parameters":{"FunctionName":function,"Payload.$":"$"},"OutputPath":"$.Payload","End":true}}}).to_string()).send().await?;
    let machine = machine.state_machine_arn().to_owned();
    let execution = client
        .start_execution()
        .state_machine_arn(&machine)
        .input(r#"{"value":35}"#)
        .send()
        .await?;
    let execution = execution.execution_arn();
    let at = Instant::now() + Duration::from_secs(120);
    loop {
        let result = client
            .describe_execution()
            .execution_arn(execution)
            .send()
            .await?;
        if result.status().as_str() != "RUNNING" {
            ensure!(
                result.status().as_str() == "SUCCEEDED",
                "image workflow failed: {:?}",
                result.error()
            );
            let output: Value = serde_json::from_str(result.output().context("workflow output")?)?;
            ensure!(
                output["computed"] == 42,
                "ASL did not compute through actual image"
            );
            break;
        }
        ensure!(Instant::now() < at, "image workflow observer expired");
        sleep(Duration::from_millis(25)).await;
    }
    client
        .delete_state_machine()
        .state_machine_arn(machine)
        .send()
        .await?;
    Ok(())
}

pub(super) async fn custom_compute(endpoint: &str, image: &str, architecture: &str) -> Result<()> {
    let client = lambda(endpoint);
    let name = format!("RustCustom{}", Uuid::new_v4().simple());
    client
        .create_function()
        .function_name(&name)
        .role("arn:aws:iam::000000000000:role/Docker")
        .package_type(PackageType::Image)
        .architectures(if architecture == "arm64" {
            Architecture::Arm64
        } else {
            Architecture::X8664
        })
        .memory_size(128)
        .timeout(30)
        .code(FunctionCode::builder().image_uri(image).build())
        .send()
        .await?;
    active(&client, &name, "Active").await?;
    let (output, error) = invoke(&client, &name, json!({"value":35})).await?;
    ensure!(
        error.is_none()
            && output["computed"] == 42
            && output["counter"] == 1
            && output["imageRevision"] == "A",
        "Rust SDK custom RIC did not compute"
    );
    client.delete_function().function_name(&name).send().await?;
    Ok(())
}
