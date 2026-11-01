//! Verify the production filter against actual Bollard log-facade call sites.

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use bollard::{
    API_DEFAULT_VERSION, Docker, container::LogOutput, models::ContainerCreateBody,
    query_parameters::LogsOptionsBuilder,
};
use futures::StreamExt;
use rustack_lambda_runtime::diagnostics::runtime_dispatch;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    time::timeout,
};
use tracing::instrument::WithSubscriber;
use tracing_log::{LogTracer, log::LevelFilter};

#[derive(Clone, Debug, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut output = self
            .0
            .lock()
            .map_err(|_| io::Error::other("capture lock failed"))?;
        if output.len().saturating_add(bytes.len()) > 1024 * 1024 {
            return Err(io::Error::other("capture exceeded cap"));
        }
        output.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

async fn request(listener: &UnixListener) -> Result<UnixStream> {
    let (mut stream, _) = listener.accept().await?;
    let mut bytes = Vec::new();
    let end = loop {
        if let Some(position) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break position + 4;
        }
        ensure!(bytes.len() < 16 * 1024, "fixture header exceeded cap");
        let mut buffer = [0; 512];
        let read = stream.read(&mut buffer).await?;
        ensure!(read > 0, "fixture header EOF");
        bytes.extend_from_slice(buffer.get(..read).context("fixture slice")?);
    };
    let header = std::str::from_utf8(bytes.get(..end).context("fixture header")?)?;
    let length = header
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>())
        })
        .transpose()?
        .unwrap_or(0);
    ensure!(length <= 4096, "fixture body exceeded cap");
    let received = bytes
        .len()
        .checked_sub(end)
        .context("fixture body offset")?;
    let remaining = length
        .checked_sub(received)
        .context("fixture body length")?;
    stream.read_exact(&mut vec![0; remaining]).await?;
    Ok(stream)
}

async fn respond(stream: &mut UnixStream, body: &[u8]) -> Result<()> {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_filter_actual_bollard_request_response_and_truncated_decoder_logs()
-> Result<()> {
    LogTracer::builder()
        .with_max_level(LevelFilter::Trace)
        .init()?;
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("docker.sock");
    let listener = UnixListener::bind(&socket)?;
    let mut server = tokio::spawn(async move {
        let mut create = request(&listener).await?;
        respond(
            &mut create,
            br#"{"Id":"probe","Warnings":["secret_bollard_response"]}"#,
        )
        .await?;
        drop(create);
        let mut logs = request(&listener).await?;
        let mut truncated = vec![1, 0, 0, 0, 0, 0, 0, 100];
        truncated.extend_from_slice(b"secret_bollard_trailing");
        respond(&mut logs, &truncated).await?;
        Ok::<_, anyhow::Error>(())
    });
    let capture = Capture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let outcome = async {
        tracing::debug!("compatibility_application_marker");
        let dispatch = runtime_dispatch();
        let docker = Docker::connect_with_socket(socket.to_str().context("fixture socket UTF-8")?, 3, API_DEFAULT_VERSION)?;
        let created = docker.create_container(None, ContainerCreateBody { env: Some(vec!["SECRET=secret_bollard_request".into()]), ..Default::default() }).with_subscriber(dispatch.clone()).await?;
        ensure!(created.id == "probe", "Bollard create response mismatch");
        // logs() constructs its HTTP future synchronously. Cover construction
        // as well as every stream poll with the production dispatch policy.
        let mut logs = tracing::dispatcher::with_default(&dispatch, || docker.logs("secret_bollard_target", Some(LogsOptionsBuilder::default().stdout(true).build())));
        async {
            let output = logs.next().await.context("expected EOF fallback record")??;
            ensure!(matches!(output, LogOutput::Console { message } if message.as_ref() == b"secret_bollard_trailing"), "upstream EOF behavior changed");
            ensure!(logs.next().await.is_none(), "unexpected extra fallback record");
            Ok::<_, anyhow::Error>(())
        }.with_subscriber(dispatch).await
    }.with_subscriber(subscriber).await;
    let joined = match timeout(Duration::from_secs(3), &mut server).await {
        Ok(result) => result.context("join fixture server")?,
        Err(_) => {
            server.abort();
            let _ = server.await;
            Err(anyhow::anyhow!("fixture server deadline"))
        }
    };
    outcome.and(joined)?;
    let shown = String::from_utf8(
        capture
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("capture lock failed"))?
            .clone(),
    )?;
    ensure!(
        shown.contains("compatibility_application_marker"),
        "application logs were lost"
    );
    for secret in [
        "secret_bollard_request",
        "secret_bollard_response",
        "secret_bollard_target",
        "secret_bollard_trailing",
    ] {
        ensure!(
            !shown.contains(secret),
            "Bollard payload reached infrastructure logs"
        );
    }
    Ok(())
}
