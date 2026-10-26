//! Explicit, local-only qualification of the Linux relay and an official RIC.
//!
//! The CLI is a test transport. Production attach, deployment and pool behavior
//! are deliberately tested by the Docker executor's separate acceptance suite.

#![cfg(unix)]
#![forbid(unsafe_code)]

use std::{env, os::unix::fs::FileTypeExt, process::Stdio, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use rustack_lambda_runtime::{
    metadata::{deadline_unix_ms, local_trace_id},
    protocol::{
        Acknowledgement, Architecture, AttemptId, EnvironmentId, Frame, Message, Outcome, Payload,
        read_frame, write_frame,
    },
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::{Instant, sleep_until, timeout, timeout_at},
};

const OPERATION: Duration = Duration::from_secs(45);
const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Debug)]
struct Docker {
    host: String,
    architecture: Architecture,
    relay_image: String,
    function_image: String,
}

impl Docker {
    async fn configured() -> Result<Self> {
        let socket = env::var("RUSTACK_RUNTIME_DOCKER_SOCKET")
            .context("explicit local Docker socket is required")?;
        ensure!(
            socket.starts_with('/') && socket.len() <= 4096,
            "Docker socket must be a bounded absolute Unix path"
        );
        ensure!(
            tokio::fs::metadata(&socket).await?.file_type().is_socket(),
            "configured Docker socket is unavailable"
        );
        let architecture = env::var("RUSTACK_RUNTIME_ARCH").context("architecture is required")?;
        let (expected, architecture) = match architecture.as_str() {
            "arm64" => ("aarch64", Architecture::Arm64),
            "amd64" => ("x86_64", Architecture::X86_64),
            _ => bail!("architecture must be arm64 or amd64"),
        };
        let relay_image = env::var("RUSTACK_RUNTIME_IMAGE").context("relay image is required")?;
        ensure!(
            !relay_image.is_empty()
                && relay_image.len() <= 1024
                && relay_image.bytes().all(|byte| byte.is_ascii_graphic()),
            "relay image reference must be bounded printable ASCII"
        );
        let function_image = format!(
            "rustack-p0-handler:{}",
            if architecture == Architecture::Arm64 {
                "arm64"
            } else {
                "amd64"
            }
        );
        let docker = Self {
            host: format!("unix://{socket}"),
            architecture,
            relay_image,
            function_image,
        };
        let info = docker
            .checked(&["info", "--format", "{{.OSType}} {{.Architecture}}"])
            .await?;
        ensure!(
            info.trim() == format!("linux {expected}"),
            "Docker daemon platform does not match this qualification lane"
        );
        for image in [&docker.relay_image, &docker.function_image] {
            docker.checked(&["image", "inspect", image]).await?;
        }
        Ok(docker)
    }

    fn command(&self) -> Command {
        let mut command = Command::new("docker");
        command
            .arg("--host")
            .arg(&self.host)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }

    async fn output(&self, arguments: &[&str]) -> Result<(bool, String)> {
        self.output_until(arguments, Instant::now() + OPERATION)
            .await
    }

    async fn output_until(&self, arguments: &[&str], deadline: Instant) -> Result<(bool, String)> {
        ensure!(Instant::now() < deadline, "Docker CLI deadline expired");
        let mut child = self.command().args(arguments).spawn()?;
        let stdout = child.stdout.take().context("CLI stdout pipe")?;
        let stderr = child.stderr.take().context("CLI stderr pipe")?;
        let outcome = timeout_at(deadline, async {
            let (status, stdout, stderr) =
                tokio::join!(child.wait(), bounded(stdout), bounded(stderr));
            Ok::<_, anyhow::Error>((status?.success(), stdout?, stderr?))
        })
        .await;
        if let Ok(result) = outcome {
            let (success, stdout, _) = result?;
            Ok((success, String::from_utf8(stdout)?))
        } else {
            child.kill().await.context("kill expired Docker CLI")?;
            child.wait().await.context("reap expired Docker CLI")?;
            bail!("Docker CLI operation deadline exceeded")
        }
    }

    async fn checked(&self, arguments: &[&str]) -> Result<String> {
        self.checked_until(arguments, Instant::now() + OPERATION)
            .await
    }

    async fn checked_until(&self, arguments: &[&str], deadline: Instant) -> Result<String> {
        let (success, output) = self.output_until(arguments, deadline).await?;
        ensure!(success, "Docker CLI operation failed");
        Ok(output)
    }
}

async fn bounded(reader: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take(u64::try_from(MAX_OUTPUT)? + 1)
        .read_to_end(&mut output)
        .await?;
    ensure!(output.len() <= MAX_OUTPUT, "Docker CLI output exceeded cap");
    Ok(output)
}

#[derive(Debug)]
struct Names {
    relay: String,
    function: String,
    label: String,
}

#[derive(Debug)]
struct Control {
    child: Child,
    input: Option<ChildStdin>,
    output: ChildStdout,
}

#[derive(Debug)]
struct Environment {
    docker: Docker,
    identity: EnvironmentId,
    names: Names,
    control: Option<Control>,
}

impl Environment {
    async fn new() -> Result<Self> {
        let identity = EnvironmentId::default();
        Ok(Self {
            docker: Docker::configured().await?,
            identity,
            names: Names {
                relay: format!("rustack-runtime-relay-{identity}"),
                function: format!("rustack-runtime-function-{identity}"),
                label: format!("rustack.runtime.test={identity}"),
            },
            control: None,
        })
    }

    async fn start(&mut self, idle_seconds: &str) -> Result<()> {
        let mut command = self.docker.command();
        command
            .args([
                "run",
                "--pull=never",
                "--rm",
                "--interactive",
                "--name",
                &self.names.relay,
                "--label",
                &self.names.label,
                "--network",
                "none",
                "--user",
                "1000:1000",
                "--read-only",
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges=true",
                "--memory",
                "64m",
                "--cpus",
                "0.1",
                "--pids-limit",
                "64",
                "--tmpfs",
                "/tmp:rw,noexec,nosuid,size=16777216",
                "--env",
                &format!("RUSTACK_LAMBDA_ENVIRONMENT_ID={}", self.identity),
                "--env",
                &format!("RUSTACK_LAMBDA_RELAY_IDLE_SECONDS={idle_seconds}"),
                &self.docker.relay_image,
            ])
            .stdin(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;
        let input = child.stdin.take().context("relay input pipe")?;
        let output = child.stdout.take().context("relay output pipe")?;
        self.control = Some(Control {
            child,
            input: Some(input),
            output,
        });
        ensure!(
            self.next().await?
                == Message::Hello {
                    architecture: self.docker.architecture
                },
            "relay Hello/platform mismatch"
        );
        ensure!(self.next().await? == Message::Ready, "missing relay Ready");
        let config = self
            .docker
            .checked(&["inspect", "--format", "{{json .Config}}", &self.names.relay])
            .await?;
        let config: Value = serde_json::from_str(&config)?;
        ensure!(
            config["StdinOnce"] == true && config["Tty"] == false,
            "qualification requires non-TTY stdin with EOF delivery"
        );
        Ok(())
    }

    async fn next(&mut self) -> Result<Message> {
        let control = self.control.as_mut().context("relay has not started")?;
        let frame = read_frame(&mut control.output, Instant::now() + OPERATION)
            .await?
            .context("relay closed control output")?;
        ensure!(frame.environment() == self.identity, "wrong environment");
        Ok(frame.into_message())
    }

    async fn send(&mut self, message: Message) -> Result<()> {
        let input = self
            .control
            .as_mut()
            .and_then(|control| control.input.as_mut())
            .context("relay input is closed")?;
        write_frame(
            input,
            &Frame::new(self.identity, message),
            Instant::now() + OPERATION,
        )
        .await?;
        Ok(())
    }

    async fn function(&mut self) -> Result<()> {
        self.docker
            .checked(&[
                "create",
                "--pull=never",
                "--name",
                &self.names.function,
                "--label",
                &self.names.label,
                "--network",
                &format!("container:{}", self.names.relay),
                "--user",
                "1000:1000",
                "--read-only",
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges=true",
                "--memory",
                "192m",
                "--cpus",
                "1",
                "--pids-limit",
                "256",
                "--tmpfs",
                "/tmp:rw,noexec,nosuid,size=536870912",
                "--env",
                "AWS_LAMBDA_RUNTIME_API=127.0.0.1:9001",
                "--env",
                "AWS_LAMBDA_FUNCTION_NAME=spike",
                "--env",
                "AWS_LAMBDA_FUNCTION_VERSION=$LATEST",
                "--env",
                "AWS_LAMBDA_FUNCTION_MEMORY_SIZE=192",
                "--env",
                "AWS_REGION=us-east-1",
                "--env",
                "AWS_DEFAULT_REGION=us-east-1",
                &self.docker.function_image,
            ])
            .await?;
        self.docker
            .checked(&["start", &self.names.function])
            .await?;
        ensure!(
            self.next().await? == Message::BootstrapReady,
            "missing first RIC poll"
        );
        Ok(())
    }

    async fn invoke(&mut self, event: Value) -> Result<AttemptId> {
        let attempt = AttemptId::default();
        self.send(Message::Invoke {
            attempt,
            deadline_unix_ms: deadline_unix_ms(Instant::now() + Duration::from_secs(60))?,
            function_arn: "arn:aws:lambda:us-east-1:000000000000:function:spike:prod".parse()?,
            trace_id: local_trace_id()?,
            payload: Payload::try_from(Bytes::from(serde_json::to_vec(&event)?))?,
        })
        .await?;
        Ok(attempt)
    }

    async fn result(&mut self, attempt: AttemptId, expected: Outcome) -> Result<Value> {
        match self.next().await? {
            Message::InvocationResult {
                attempt: actual,
                outcome,
                payload,
            } if actual == attempt && outcome == expected => {
                serde_json::from_slice(payload.as_ref()).context("RIC JSON result")
            }
            _ => bail!("unexpected relay result identity/outcome/variant"),
        }
    }

    async fn wait(&mut self, success: bool) -> Result<()> {
        let control = self.control.as_mut().context("relay has not started")?;
        let status = timeout(OPERATION, control.child.wait())
            .await
            .context("relay failed to exit with input ownership retained")??;
        ensure!(status.success() == success, "unexpected relay exit status");
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.send(Message::Close).await?;
        ensure!(
            self.next().await? == Message::Ack(Acknowledgement::Close),
            "missing flushed Close acknowledgement"
        );
        self.wait(true).await
    }

    async fn remove_function(&self) -> Result<()> {
        self.docker
            .checked(&["rm", "--force", &self.names.function])
            .await?;
        Ok(())
    }

    async fn cleanup(&mut self) -> Result<()> {
        // Names are reserved before create/run, even if the response is lost.
        let removal = self
            .docker
            .output(&["rm", "--force", &self.names.function, &self.names.relay])
            .await;
        let reaped = if let Some(control) = &mut self.control {
            control.input.take();
            let killed = control.child.start_kill().context("kill relay CLI");
            let waited = if let Ok(result) = timeout(OPERATION, control.child.wait()).await {
                result.map(|_| ()).context("reap relay CLI")
            } else {
                // An expired waiter must not discard process ownership.
                let result = control
                    .child
                    .wait()
                    .await
                    .context("retained relay CLI reap");
                result.and_then(|_| bail!("relay CLI cleanup deadline"))
            };
            killed.and(waited)
        } else {
            Ok(())
        };
        let reconciled = async {
            let remaining = self
                .docker
                .checked(&[
                    "ps",
                    "--all",
                    "--quiet",
                    "--filter",
                    &format!("label={}", self.names.label),
                ])
                .await?;
            ensure!(remaining.trim().is_empty(), "owned containers remain");
            Ok(())
        }
        .await;
        removal.map(|_| ()).and(reaped).and(reconciled)
    }

    async fn finish(&mut self, outcome: Result<()>) -> Result<()> {
        let cleanup = self.cleanup().await;
        match (outcome, cleanup) {
            (Err(error), Err(cleanup)) => {
                Err(error.context(format!("cleanup failed: {cleanup:#}")))
            }
            (Err(error), Ok(())) => Err(error),
            (Ok(()), cleanup) => cleanup,
        }
    }
}

#[tokio::test]
#[ignore = "requires the explicit local Docker relay qualification target"]
async fn test_should_exit_on_close_eof_truncation_and_io_timeout_with_owned_pipes() -> Result<()> {
    for mode in ["close", "eof", "truncated", "partial", "idle"] {
        let mut environment = Environment::new().await?;
        let result = async {
            // Allow time for inspect during setup; idle expiry must still exit
            // while the parent retains the open, otherwise silent stdin pipe.
            environment
                .start(if mode == "idle" { "5" } else { "600" })
                .await?;
            if mode == "close" {
                environment.close().await?;
            } else {
                let control = environment.control.as_mut().context("relay control")?;
                if mode == "truncated" || mode == "partial" {
                    control
                        .input
                        .as_mut()
                        .context("input")?
                        .write_all(&[0])
                        .await?;
                }
                if mode == "eof" || mode == "truncated" {
                    control.input.take();
                }
                environment.wait(false).await?;
            }
            Ok(())
        }
        .await;
        environment
            .finish(result)
            .await
            .with_context(|| format!("pipe case {mode}"))?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires the explicit local Docker relay qualification target"]
async fn test_should_reconcile_failed_setup_and_reap_an_expired_cli_helper() -> Result<()> {
    for mode in ["setup", "scope", "cli"] {
        let mut environment = Environment::new().await?;
        let result = async {
            environment.start("600").await?;
            match mode {
                "setup" => {
                    environment.docker.function_image =
                        format!("rustack-runtime-missing:{}", environment.identity);
                    ensure!(
                        environment.function().await.is_err(),
                        "missing image setup unexpectedly succeeded"
                    );
                }
                "scope" => {
                    let input = environment
                        .control
                        .as_mut()
                        .and_then(|control| control.input.as_mut())
                        .context("relay input")?;
                    write_frame(
                        input,
                        &Frame::new(EnvironmentId::default(), Message::Close),
                        Instant::now() + OPERATION,
                    )
                    .await?;
                    environment.wait(false).await?;
                }
                _ => {
                    // docker wait owns a different CLI child and cannot finish
                    // while the relay is running. Its own deadline must reap
                    // that child without cancelling or stopping the relay.
                    let expired = environment
                        .docker
                        .checked_until(
                            &["wait", &environment.names.relay],
                            Instant::now() + Duration::from_millis(200),
                        )
                        .await;
                    ensure!(expired.is_err(), "CLI helper did not reach its deadline");
                    environment.close().await?;
                }
            }
            Ok(())
        }
        .await;
        environment
            .finish(result)
            .await
            .with_context(|| format!("failure case {mode}"))?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires the explicit local Docker relay qualification target"]
async fn test_should_compute_reuse_and_report_official_ric_handler_errors() -> Result<()> {
    let mut environment = Environment::new().await?;
    let result = async {
        environment.start("600").await?;
        environment.function().await?;
        for (value, computed, counter) in [(35, 42, 1), (0, 7, 2)] {
            let attempt = environment.invoke(json!({ "value": value })).await?;
            let result = environment.result(attempt, Outcome::Success).await?;
            ensure!(
                result["computed"] == computed && result["counter"] == counter,
                "incorrect computation or warm counter"
            );
            ensure!(
                result["requestId"] == attempt.to_string(),
                "incorrect RIC attempt metadata"
            );
            ensure!(
                result["uid"] == 1000 && result["rootReadOnly"] == true,
                "function user/rootfs limits differ"
            );
            ensure!(
                result["memoryMax"] == "201326592" && result["pidsMax"] == "256",
                "function cgroup limits differ"
            );
        }
        let attempt = environment.invoke(json!({ "fail": true })).await?;
        let error = environment.result(attempt, Outcome::FunctionError).await?;
        ensure!(
            error["errorType"] == "ValueError",
            "handler error was not preserved"
        );
        environment.remove_function().await?;
        environment.close().await
    }
    .await;
    environment.finish(result).await
}

#[tokio::test]
#[ignore = "requires the explicit local Docker relay qualification target"]
async fn test_should_cancel_a_running_official_ric_and_remove_both_containers() -> Result<()> {
    let mut environment = Environment::new().await?;
    let result = async {
        environment.start("600").await?;
        environment.function().await?;
        let attempt = environment.invoke(json!({ "sleep": true })).await?;
        let marker = format!("P0_BUSY_START={attempt}");
        let busy_deadline = Instant::now() + OPERATION;
        loop {
            // Forward the absolute deadline into the CLI owner. Cancelling an
            // outer timeout here would discard its in-flight kill/reap path.
            let logs = environment
                .docker
                .checked_until(&["logs", &environment.names.function], busy_deadline)
                .await?;
            if logs.contains(&marker) {
                break;
            }
            ensure!(
                Instant::now() < busy_deadline,
                "handler never entered its busy section"
            );
            sleep_until((Instant::now() + Duration::from_millis(50)).min(busy_deadline)).await;
        }
        for _ in 0..2 {
            environment.send(Message::Cancel { attempt }).await?;
            ensure!(
                environment.next().await? == Message::Ack(Acknowledgement::Cancel(attempt)),
                "missing matching cancel acknowledgement"
            );
        }
        let running = environment
            .docker
            .checked(&[
                "inspect",
                "--format",
                "{{.State.Running}}",
                &environment.names.function,
            ])
            .await?;
        ensure!(
            running.trim() == "true",
            "Cancel Ack must not claim external process termination"
        );
        environment.remove_function().await?;
        environment.close().await
    }
    .await;
    environment.finish(result).await
}
