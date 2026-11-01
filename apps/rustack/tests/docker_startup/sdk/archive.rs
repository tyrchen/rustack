//! Actual restored definitions, immutable aliases, cold computation and old history.

use super::*;

#[derive(Debug)]
pub(crate) struct ArchiveCheckpoint {
    function: String,
    hash: String,
    revision: String,
    old_request: String,
    machine: String,
    execution: String,
    history: String,
    output: String,
}
fn states(endpoint: &str) -> States {
    States::from_conf(
        aws_sdk_sfn::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(endpoint)
            .credentials_provider(credentials())
            .retry_config(RetryConfig::disabled())
            .timeout_config(timeouts())
            .build(),
    )
}
async fn terminal(client: &States, execution: &str) -> Result<String> {
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
                "saved image workflow failed"
            );
            let output = result.output().context("image workflow output")?.to_owned();
            let body: Value = serde_json::from_str(&output)?;
            ensure!(
                body["computed"] == 49,
                "restored image task did not compute"
            );
            return Ok(output);
        }
        ensure!(
            Instant::now() < at,
            "image archive workflow observer expired"
        );
        sleep(Duration::from_millis(25)).await;
    }
}
async fn history(client: &States, execution: &str) -> Result<String> {
    let result = client
        .get_execution_history()
        .execution_arn(execution)
        .send()
        .await?;
    ensure!(
        (5..=32).contains(&result.events().len()) && result.next_token().is_none(),
        "image archive history bounds"
    );
    let value = format!("{:?}", result.events());
    ensure!(value.len() <= 65_536, "image archive history byte cap");
    Ok(value)
}
pub(crate) async fn checkpoint(endpoint: &str) -> Result<ArchiveCheckpoint> {
    let client = lambda(endpoint);
    let listed = client.list_functions().send().await?;
    ensure!(
        listed.functions().len() <= 8 && listed.next_marker().is_none(),
        "archive fixture function bounds"
    );
    let function = listed
        .functions()
        .iter()
        .filter_map(|function| function.function_name())
        .find(|name| {
            name.starts_with("DockerStartup")
                && !name.ends_with("OtherArch")
                && !name.ends_with("NoRic")
        })
        .context("actual primary saved image")?
        .to_owned();
    let config = client
        .get_function_configuration()
        .function_name(&function)
        .send()
        .await?;
    let hash = config.code_sha256().context("saved image hash")?.to_owned();
    let revision = config.revision_id().context("saved revision")?.to_owned();
    let arn = config.function_arn().context("saved function ARN")?;
    let workflows = states(endpoint);
    let machine = workflows.create_state_machine().name(format!("ArchiveImage{}", Uuid::new_v4().simple()))
        .role_arn("arn:aws:iam::000000000000:role/Docker")
        .definition(json!({"StartAt":"Compute","States":{"Compute":{"Type":"Task","Resource":"arn:aws:states:::lambda:invoke","Parameters":{"FunctionName":format!("{arn}:live"),"Payload":{"value":42}},"OutputPath":"$.Payload","End":true}}}).to_string())
        .send().await?.state_machine_arn().to_owned();
    let execution = workflows
        .start_execution()
        .state_machine_arn(&machine)
        .send()
        .await?
        .execution_arn()
        .to_owned();
    let output = terminal(&workflows, &execution).await?;
    let history = history(&workflows, &execution).await?;
    let (actual, error) = invoke(&client, &function, json!({"value":3})).await?;
    ensure!(
        error.is_none() && actual["computed"] == 10,
        "pre-save computation"
    );
    let old_request = actual["requestId"]
        .as_str()
        .context("pre-save request")?
        .to_owned();
    Ok(ArchiveCheckpoint {
        function,
        hash,
        revision,
        old_request,
        machine,
        execution,
        history,
        output,
    })
}
pub(crate) async fn verify(endpoint: &str, saved: &ArchiveCheckpoint) -> Result<()> {
    let client = lambda(endpoint);
    let config = client
        .get_function_configuration()
        .function_name(&saved.function)
        .send()
        .await?;
    ensure!(
        config.code_sha256() == Some(saved.hash.as_str())
            && config.revision_id() == Some(saved.revision.as_str()),
        "restored image identity/configuration changed"
    );
    let workflows = states(endpoint);
    ensure!(
        history(&workflows, &saved.execution).await? == saved.history,
        "restored history changed or replayed"
    );
    ensure!(
        terminal(&workflows, &saved.execution).await? == saved.output,
        "saved terminal output changed"
    );
    let (latest, error) = invoke(&client, &saved.function, json!({"value":100})).await?;
    ensure!(
        error.is_none()
            && latest["computed"] == 107
            && latest["counter"] == 1
            && latest["requestId"].as_str() != Some(saved.old_request.as_str()),
        "restored latest did not execute cold without replay"
    );
    let alias = client
        .invoke()
        .function_name(&saved.function)
        .qualifier("live")
        .payload(Blob::new(br#"{"value":50}"#.as_slice()))
        .send()
        .await?;
    let value: Value =
        serde_json::from_slice(alias.payload().context("restored alias payload")?.as_ref())?;
    ensure!(
        alias.function_error().is_none()
            && alias.executed_version() == Some("1")
            && value["computed"] == 57
            && value["counter"] == 1,
        "restored immutable alias failed cold computation"
    );
    let next = workflows
        .start_execution()
        .state_machine_arn(&saved.machine)
        .send()
        .await?;
    terminal(&workflows, next.execution_arn()).await?;
    ensure!(
        history(&workflows, &saved.execution).await? == saved.history,
        "new work rewrote restored history"
    );
    Ok(())
}
