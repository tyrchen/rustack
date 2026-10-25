//! Temporary experiment relay: real HTTP Runtime API with framed CBOR stdio.
#![forbid(unsafe_code)]

use std::{
    convert::Infallible,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Request, Response, StatusCode, body::Incoming, server::conn::http1, service::service_fn,
};
use hyper_util::rt::TokioIo;
use rustack_workflows_spikes::{Frame, read_frame, write_frame};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
    time::timeout,
};

#[derive(Debug)]
struct ApiRequest {
    method: String,
    path: String,
    body: Bytes,
    reply: oneshot::Sender<Response<Full<Bytes>>>,
}

fn response(status: StatusCode, body: Bytes) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body));
    *response.status_mut() = status;
    response
}

async fn serve(
    request: Request<Incoming>,
    tx: mpsc::Sender<ApiRequest>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let result = timeout(
        Duration::from_secs(5),
        Limited::new(request.into_body(), 6 * 1024 * 1024).collect(),
    )
    .await;
    let body = match result {
        Ok(Ok(body)) => body.to_bytes(),
        _ => return Ok(response(StatusCode::BAD_REQUEST, Bytes::new())),
    };
    let (reply, rx) = oneshot::channel();
    if tx
        .send(ApiRequest {
            method,
            path,
            body,
            reply,
        })
        .await
        .is_err()
    {
        return Ok(response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new()));
    }
    Ok(match rx.await {
        Ok(value) => value,
        Err(_) => response(StatusCode::SERVICE_UNAVAILABLE, Bytes::new()),
    })
}

async fn http_server(
    listener: TcpListener,
    tx: mpsc::Sender<ApiRequest>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            connection = listener.accept() => {
                let (socket, _) = connection?;
                ensure!(connections.len() < 8, "connection cap");
                let tx = tx.clone();
                connections.spawn(async move {
                    http1::Builder::new().serve_connection(TokioIo::new(socket), service_fn(move |request| serve(request, tx.clone()))).await
                });
            }
            completed = connections.join_next(), if !connections.is_empty() => {
                if let Some(completed) = completed { completed??; }
            }
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}

async fn run() -> Result<()> {
    let environment = std::env::var("SPIKE_ENVIRONMENT_UUID").context("environment identity")?;
    ensure!(environment.len() == 36, "invalid environment identity");
    let listener = TcpListener::bind(("127.0.0.1", 9001)).await?;
    let (api_tx, mut api_rx) = mpsc::channel(8);
    let (control_tx, mut control_rx) = mpsc::channel(8);
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut workers = JoinSet::new();
    workers.spawn(http_server(listener, api_tx, stop_rx));
    workers.spawn(async move {
        let mut input = tokio::io::stdin();
        loop {
            let frame = read_frame(&mut input).await?;
            let close = frame.kind == "close";
            control_tx.send(frame).await.context("control receiver")?;
            if close {
                return std::future::pending::<Result<()>>().await;
            }
        }
    });
    let mut output = tokio::io::stdout();
    write_frame(
        &mut output,
        &Frame::new(&environment, "hello", "", Bytes::new()),
    )
    .await?;
    write_frame(
        &mut output,
        &Frame::new(&environment, "ready", "", Bytes::new()),
    )
    .await?;
    let mut poll: Option<oneshot::Sender<Response<Full<Bytes>>>> = None;
    let mut job: Option<Frame> = None;
    let mut delivered = false;
    let mut bootstrap_seen = false;
    let outcome: Result<()> = async {
        loop {
            tokio::select! {
                frame = control_rx.recv() => {
                    let Some(frame) = frame else { bail!("control EOF") };
                    ensure!(frame.environment == environment, "foreign environment");
                    match frame.kind.as_str() {
                        "invoke" => { ensure!(job.is_none(), "concurrent invocation"); job = Some(frame); delivered = false; }
                        "close" => { write_frame(&mut output, &Frame::new(&environment, "ack", "", Bytes::new())).await?; break; }
                        _ => bail!("unexpected control variant"),
                    }
                }
                request = api_rx.recv() => {
                    let Some(request) = request else { bail!("API channel closed") };
                    if request.method == "GET" && request.path == "/2018-06-01/runtime/invocation/next" {
                        if poll.is_some() || delivered {
                            let _ = request.reply.send(response(StatusCode::CONFLICT, Bytes::new()));
                        } else {
                            poll = Some(request.reply);
                            if !bootstrap_seen {
                                write_frame(&mut output, &Frame::new(&environment, "bootstrapReady", "", Bytes::new())).await?;
                                bootstrap_seen = true;
                            }
                        }
                    } else if request.method == "POST" && request.path == "/2018-06-01/runtime/init/error" {
                        let _ = request.reply.send(response(StatusCode::ACCEPTED, Bytes::new()));
                        write_frame(&mut output, &Frame::new(&environment, "initError", "", request.body)).await?;
                    } else if let Some(active) = &job {
                        let success = format!("/2018-06-01/runtime/invocation/{}/response", active.request_id);
                        let failure = format!("/2018-06-01/runtime/invocation/{}/error", active.request_id);
                        if request.method == "POST" && delivered && (request.path == success || request.path == failure) {
                            let kind = if request.path == success { "result" } else { "error" };
                            let _ = request.reply.send(response(StatusCode::ACCEPTED, Bytes::new()));
                            write_frame(&mut output, &Frame::new(&environment, kind, &active.request_id, request.body)).await?;
                            job = None; delivered = false;
                        } else { let _ = request.reply.send(response(StatusCode::BAD_REQUEST, Bytes::new())); }
                    } else { let _ = request.reply.send(response(StatusCode::BAD_REQUEST, Bytes::new())); }
                }
                worker = workers.join_next(), if !workers.is_empty() => {
                    if let Some(worker) = worker { worker??; }
                    bail!("relay worker terminated");
                }
                () = tokio::time::sleep(Duration::from_secs(120)) => bail!("idle lease expired"),
            }
            if !delivered && let Some(active) = &job && let Some(reply) = poll.take() {
                        let deadline = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().checked_add(10_000).context("deadline overflow")?;
                        let mut event = response(StatusCode::OK, active.payload.clone());
                        for (key, value) in [
                            ("lambda-runtime-aws-request-id", active.request_id.clone()),
                            ("lambda-runtime-deadline-ms", deadline.to_string()),
                            ("lambda-runtime-invoked-function-arn", "arn:aws:lambda:us-east-1:000000000000:function:spike".into()),
                            ("lambda-runtime-trace-id", "Root=1-00000000-000000000000000000000000;Sampled=0".into()),
                            ("content-type", "application/json".into()),
                        ] { event.headers_mut().insert(key, value.parse()?); }
                        reply.send(event).map_err(|_| anyhow::anyhow!("RIC disconnected before delivery"))?;
                        delivered = true;
            }
        }
        Ok(())
    }.await;
    let _ = stop_tx.send(true);
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    outcome
}

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}
