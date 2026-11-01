//! Actual Native custom runtime using ordinary signed Step Functions SDK calls.
#![forbid(unsafe_code)]

use std::{env, ffi::OsString, process::ExitCode, time::Duration};
#[cfg(unix)]
use std::{os::unix::process::CommandExt, path::PathBuf};

use anyhow::{Context, Result, ensure};
use aws_credential_types::Credentials;
use aws_sdk_sfn::{
    Client,
    config::{BehaviorVersion, Region, retry::RetryConfig, timeout::TimeoutConfig},
    error::ProvideErrorMetadata,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, de::Error as _};
use serde_json::json;
#[cfg(unix)]
use tokio::process::Command;
use tokio::time::{Instant, sleep};
use url::Url;

mod wire;
use wire::Wire;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Mode {
    Early,
    Late,
    ExternalFailure,
    InvocationFailure,
    Retry,
    Heartbeat,
    AbsoluteTimeout,
    HeartbeatTimeout,
    Stop,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    #[serde(deserialize_with = "secret")]
    token: SecretString,
    execution_arn: String,
    mode: Mode,
    n: u32,
    retry: u32,
}
fn secret<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<SecretString, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(D::Error::custom("invalid callback credential"));
    }
    Ok(value.into())
}
#[derive(Debug)]
struct Sdk {
    client: Client,
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    if let Some(interpreter) = env::var_os("CALLBACK_PYTHON") {
        return python(interpreter).await;
    }
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
#[cfg(unix)]
async fn python(interpreter: OsString) -> ExitCode {
    let Some(root) = env::var_os("LAMBDA_TASK_ROOT") else {
        return ExitCode::FAILURE;
    };
    let script = PathBuf::from(root).join("callback.py");
    let path = PathBuf::from(&interpreter);
    if !path.is_absolute()
        || interpreter.len() > 4096
        || !matches!(tokio::time::timeout(Duration::from_secs(2),tokio::fs::metadata(&script)).await,Ok(Ok(metadata)) if metadata.is_file())
    {
        return ExitCode::FAILURE;
    }
    // exec preserves the exact PID already owned by Native's process supervisor.
    let _error = Command::new(path).arg(script).as_std_mut().exec();
    ExitCode::FAILURE
}
#[cfg(not(unix))]
async fn python(_interpreter: OsString) -> ExitCode {
    ExitCode::FAILURE
}
async fn run() -> Result<()> {
    let wire = Wire::new().await?;
    let sdk = Sdk::new()?;
    let mut previous: Option<SecretString> = None;
    loop {
        let (request, body) = wire.next().await?;
        let input: Input = serde_json::from_slice(&body).context("decode callback input")?;
        ensure!(
            input.n <= 1000
                && input.retry <= 1
                && (1..=256).contains(&input.execution_arn.len())
                && input.execution_arn.is_ascii()
                && input
                    .execution_arn
                    .starts_with("arn:aws:states:us-east-1:000000000000:execution:"),
            "invalid callback input"
        );
        if input.retry == 1 {
            let old = previous.take().context("missing previous attempt")?;
            ensure!(
                old.expose_secret() != input.token.expose_secret(),
                "reused callback token"
            );
            sdk.closed(&old).await?;
        }
        execute(&wire, &sdk, &request, &input).await?;
        previous = Some(input.token);
    }
}
impl Sdk {
    fn new() -> Result<Self> {
        let endpoint = env::var("CALLBACK_ENDPOINT").context("callback endpoint")?;
        ensure!(endpoint.len() <= 128, "callback endpoint bound");
        let parsed = Url::parse(&endpoint)?;
        ensure!(
            parsed.scheme() == "http"
                && parsed.host_str() == Some("127.0.0.1")
                && parsed.port().is_some()
                && parsed.path() == "/"
                && parsed.query().is_none()
                && parsed.fragment().is_none()
                && parsed.username().is_empty()
                && parsed.password().is_none(),
            "invalid callback endpoint"
        );
        let client = Client::from_conf(
            aws_sdk_sfn::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .region(Region::new("us-east-1"))
                .endpoint_url(endpoint)
                .credentials_provider(Credentials::new(
                    env::var("CALLBACK_ACCESS_KEY")?,
                    env::var("CALLBACK_SECRET_KEY")?,
                    None,
                    None,
                    "callback-fixture",
                ))
                .retry_config(RetryConfig::disabled())
                .timeout_config(
                    TimeoutConfig::builder()
                        .connect_timeout(Duration::from_secs(2))
                        .read_timeout(Duration::from_secs(4))
                        .operation_timeout(Duration::from_secs(4))
                        .build(),
                )
                .build(),
        );
        Ok(Self { client })
    }
    async fn heartbeat(&self, input: &Input) -> Result<bool> {
        let result = self
            .client
            .send_task_heartbeat()
            .task_token(input.token.expose_secret())
            .send()
            .await;
        match result {
            Ok(_) => Ok(true),
            Err(error)
                if input.mode == Mode::AbsoluteTimeout
                    && error
                        .as_service_error()
                        .and_then(ProvideErrorMetadata::code)
                        == Some("TaskTimedOut") =>
            {
                Ok(false)
            }
            Err(error) => Err(error).context("handler heartbeat"),
        }
    }
    async fn closed(&self, token: &SecretString) -> Result<()> {
        let error = self
            .client
            .send_task_heartbeat()
            .task_token(token.expose_secret())
            .send()
            .await
            .err()
            .context("closed token unexpectedly accepted")?;
        ensure!(
            error
                .as_service_error()
                .and_then(ProvideErrorMetadata::code)
                == Some("TaskTimedOut"),
            "wrong closed token classification"
        );
        Ok(())
    }
    async fn complete(&self, input: &Input, request: &str, closed: bool) -> Result<()> {
        let output = json!({"value":input.n.checked_mul(2).context("compute overflow")?,"sdk":"rust","requestId":request,"retry":input.retry});
        let result = self
            .client
            .send_task_success()
            .task_token(input.token.expose_secret())
            .output(output.to_string())
            .send()
            .await;
        if closed {
            let error = result
                .err()
                .context("expired token unexpectedly accepted")?;
            ensure!(
                error
                    .as_service_error()
                    .and_then(ProvideErrorMetadata::code)
                    == Some("TaskTimedOut"),
                "wrong expired token classification"
            );
        } else {
            result.context("handler success")?;
        }
        self.closed(&input.token).await
    }
    async fn failure(&self, input: &Input) -> Result<()> {
        self.client
            .send_task_failure()
            .task_token(input.token.expose_secret())
            .error(if input.mode == Mode::Retry {
                "CallbackRetry"
            } else {
                "CallbackExternalFailure"
            })
            .cause("actual handler SDK failure")
            .send()
            .await
            .context("handler external failure")?;
        self.closed(&input.token).await
    }
    async fn history_safe(&self, input: &Input) -> Result<bool> {
        let history = self
            .client
            .get_execution_history()
            .execution_arn(&input.execution_arn)
            .include_execution_data(true)
            .max_results(1000)
            .send()
            .await
            .context("handler history")?;
        let mut scheduled = false;
        for event in history.events() {
            if let Some(details) = event.task_scheduled_event_details() {
                scheduled = true;
                if details.parameters().contains(input.token.expose_secret()) {
                    return Ok(false);
                }
            }
            for value in [
                event
                    .execution_started_event_details()
                    .and_then(|v| v.input()),
                event.state_entered_event_details().and_then(|v| v.input()),
                event.state_exited_event_details().and_then(|v| v.output()),
                event
                    .task_submitted_event_details()
                    .and_then(|v| v.output()),
                event
                    .task_succeeded_event_details()
                    .and_then(|v| v.output()),
                event
                    .execution_succeeded_event_details()
                    .and_then(|v| v.output()),
                event.task_failed_event_details().and_then(|v| v.error()),
                event.task_failed_event_details().and_then(|v| v.cause()),
                event.task_timed_out_event_details().and_then(|v| v.error()),
                event.task_timed_out_event_details().and_then(|v| v.cause()),
                event
                    .execution_failed_event_details()
                    .and_then(|v| v.error()),
                event
                    .execution_failed_event_details()
                    .and_then(|v| v.cause()),
            ]
            .into_iter()
            .flatten()
            {
                if value.contains(input.token.expose_secret()) {
                    return Ok(false);
                }
            }
        }
        Ok(scheduled)
    }
}
async fn execute(wire: &Wire, sdk: &Sdk, request: &str, input: &Input) -> Result<()> {
    wire.record(request, "consumed", input.retry, 0, false, false)
        .await?;
    let late = matches!(
        input.mode,
        Mode::Late | Mode::Heartbeat | Mode::AbsoluteTimeout | Mode::HeartbeatTimeout
    );
    if late {
        wire.post(request, false).await?;
        wire.record(request, "returned", input.retry, 0, false, false)
            .await?;
    }
    if late || input.mode == Mode::Stop {
        wait(wire, sdk, request, input).await?;
    }
    if input.mode == Mode::ExternalFailure || input.mode == Mode::Retry && input.retry == 0 {
        sdk.failure(input).await?;
    } else {
        sdk.complete(
            input,
            request,
            matches!(input.mode, Mode::AbsoluteTimeout | Mode::HeartbeatTimeout),
        )
        .await?;
    }
    let safe = sdk.history_safe(input).await?;
    ensure!(safe, "credential exposed in history");
    wire.record(request, "callback", input.retry, 0, safe, true)
        .await?;
    if !late {
        wait(wire, sdk, request, input).await?;
        wire.post(request, input.mode == Mode::InvocationFailure)
            .await?;
    }
    Ok(())
}
async fn wait(wire: &Wire, sdk: &Sdk, request: &str, input: &Input) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(45);
    let mut heartbeats = 0_u32;
    while !wire.released(request).await? {
        ensure!(Instant::now() < until, "fixture gate expired");
        if matches!(input.mode, Mode::Heartbeat | Mode::AbsoluteTimeout) {
            if sdk.heartbeat(input).await? {
                heartbeats = heartbeats.checked_add(1).context("heartbeat overflow")?;
                wire.record(request, "heartbeat", input.retry, heartbeats, false, false)
                    .await?;
            }
        }
        sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
