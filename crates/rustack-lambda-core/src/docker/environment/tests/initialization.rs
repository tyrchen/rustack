//! Post-bootstrap initialization failures through the actual runtime stack.

use std::{
    future::Future,
    net::SocketAddr,
    task::{Context, Wake, Waker},
};

use rustack_lambda_runtime::{
    client::{ClientConfig, ClientEvent, ControlClient},
    http,
    metadata::local_trace_id,
    protocol::{Architecture, AttemptId},
    relay::{self, RelayConfig},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, duplex, split},
    net::TcpStream,
    sync::{Notify, Semaphore},
};

use super::*;
use crate::docker::{DockerOutputReader, FunctionLogs, plan::ContainerRole};

struct Offered(Notify);
impl Wake for Offered {
    fn wake(self: Arc<Self>) {
        self.0.notify_one();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.notify_one();
    }
}

#[rstest::rstest]
#[case::idle_bounded(false, 32768)]
#[case::idle_oversized(false, 32769)]
#[case::busy_bounded(true, 32768)]
#[case::busy_oversized(true, 32769)]
#[tokio::test]
async fn test_should_preserve_init_error_after_abandoned_bootstrap_poll_and_join_retirement(
    #[case] busy: bool,
    #[case] bytes: usize,
) {
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let planned = Arc::new(plan(&directory()));
    let RuntimeFixture {
        address,
        session,
        mut relay,
        client,
    } = runtime(planned.environment_id()).await;

    let offered = Arc::new(Offered(Notify::new()));
    let waker = Waker::from(offered.clone());
    let mut observer = Box::pin(session.next(deadline()));
    if busy {
        assert!(
            observer
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
    }
    let signals = Signals::new();
    signals.phase(EnvironmentPhase::Idle);
    let (mut task, io_task, calls, mut quiet) =
        start_worker(wire.port(), planned.clone(), client, signals.clone());
    let inspected = function_inspection(&planned);
    let daemon_signals = signals.clone();
    let (retiring, observed) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let container_cleanup = tokio::spawn(async move {
        loop {
            let request = daemon.next().await;
            assert!(request.head.starts_with("GET "));
            assert!(request.head.contains(&"a".repeat(64)));
            if daemon_signals.status.borrow().phase == EnvironmentPhase::Retiring {
                retiring.send(()).unwrap();
                timeout_at(deadline(), released).await.unwrap().unwrap();
                request.reply(404, b"{}").await;
                break;
            }
            request.reply(200, &inspected).await;
        }
        daemon.close().await;
    });
    let (stop, cancellation) = watch::channel(false);
    let (reply, mut response) = oneshot::channel();
    if busy {
        calls
            .send(Call {
                invocation: Invocation::new(
                    AttemptId::default(),
                    deadline(),
                    "arn:aws:lambda:us-east-1:000000000000:function:policy-fixture"
                        .parse()
                        .unwrap(),
                    local_trace_id().unwrap(),
                    Payload::try_from(Bytes::from_static(b"{}")).unwrap(),
                )
                .unwrap(),
                capture_logs: false,
                cancellation,
                reply,
            })
            .await
            .unwrap();
        timeout_at(deadline(), offered.0.notified()).await.unwrap();
        assert_eq!(signals.status.borrow().phase, EnvironmentPhase::Busy);
    } else {
        drop(reply);
    }
    // A wake proves an offer exists. Never repoll: that would acknowledge delivery.
    drop(observer);
    let marker = b"private initialization cause";
    let mut payload = vec![b'x'; bytes];
    payload
        .get_mut(..marker.len())
        .unwrap()
        .copy_from_slice(marker);
    post_init_error(address, &payload).await;
    timeout_at(deadline(), observed).await.unwrap().unwrap();
    assert!(!task.is_finished());
    if busy {
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
    }
    release.send(()).unwrap();
    timeout_at(deadline(), &mut task).await.unwrap().unwrap();
    io_task.wait().await.unwrap();
    timeout_at(deadline(), relay.wait()).await.unwrap().unwrap();
    container_cleanup.await.unwrap();
    let error = signals.status.borrow().error.clone().unwrap();
    assert_class(&error, &payload);
    if busy {
        assert_class(&response.await.unwrap().unwrap_err(), &payload);
    }
    assert!(!format!("{error:?} {error}").contains(std::str::from_utf8(marker).unwrap()));
    assert_eq!(quiet.read(&mut [0; 1]).await.unwrap(), 0);
    assert!(TcpStream::connect(address).await.is_err());
    drop(stop);
    drop(calls);
    wire.shutdown().await.unwrap();
}

fn start_worker(
    docker: DockerPort,
    planned: Arc<EnvironmentPlan>,
    client: ControlClient,
    signals: Signals,
) -> (
    tokio::task::JoinHandle<()>,
    Arc<CleanupTask>,
    mpsc::Sender<Call>,
    tokio::io::DuplexStream,
) {
    let (quiet, reader) = duplex(8);
    let logs = FunctionLogs::start_reader(
        DockerOutputReader::new(reader, Duration::from_secs(1)).unwrap(),
        4096,
    )
    .unwrap();
    let config = EnvironmentConfig::default();
    let io_task = Arc::new(CleanupTask::default());
    let progress = Arc::new(cleanup::Progress::new());
    progress
        .phase(ContainerRole::Function)
        .send_replace(cleanup::Phase::Known("a".repeat(64).parse().unwrap()));
    let worker = worker::Worker::builder()
        .docker(docker)
        .config(config)
        .plan(planned)
        .progress(progress)
        .cleanup(Arc::new(Semaphore::new(2)))
        .health(Arc::new(Health::default()))
        .signals(signals.clone())
        .global(watch::channel(false).0)
        .io(io_task.clone())
        .runtime(Handle::current())
        .build();
    let mut io = io::IoGuard::new(Handle::current(), io_task.clone(), config, signals);
    io.test_streams(client, logs);
    let (calls, mut mailbox) = mpsc::channel(4);
    let task = tokio::spawn(async move {
        worker.serve_and_retire_test(&mut io, &mut mailbox).await;
    });
    (task, io_task, calls, quiet)
}

fn function_inspection(plan: &EnvironmentPlan) -> Vec<u8> {
    let mut owned = inspected(plan);
    owned["Name"] = json!(format!("/{}", plan.function_key().as_str()));
    owned["Config"]["Labels"]["rustack.lambda.role"] = json!("function");
    owned["State"]["Running"] = json!(true);
    serde_json::to_vec(&owned).unwrap()
}

async fn post_init_error(address: SocketAddr, payload: &[u8]) {
    timeout_at(deadline(), async {
        let mut stream = TcpStream::connect(address).await.unwrap();
        let head = format!(
            "POST /2018-06-01/runtime/init/error HTTP/1.1\r\nHost: localhost\r\nContent-Length: \
             {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(payload).await.unwrap();
        let mut response = Vec::new();
        stream.take(16384).read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 202 "), "{response:?}");
    })
    .await
    .unwrap();
}

fn assert_class(error: &EnvironmentError, payload: &[u8]) {
    if payload.len() <= 32768 {
        let EnvironmentError::Initialization(received) = error else {
            panic!("{error:?}");
        };
        assert_eq!(received.as_ref(), payload);
    } else {
        assert!(
            matches!(error, EnvironmentError::Docker(DockerError::Protocol)),
            "{error:?}"
        );
    }
}

struct RuntimeFixture {
    address: SocketAddr,
    session: rustack_lambda_runtime::session::SessionHandle,
    relay: relay::RelayHandle,
    client: ControlClient,
}
async fn runtime(environment: EnvironmentId) -> RuntimeFixture {
    let api = http::start().await.unwrap();
    let address = api.addr();
    let session = api.session().clone();
    let (host, peer) = duplex(65536);
    let (reader, writer) = split(peer);
    let relay = relay::start(
        api,
        RelayConfig::new(
            environment,
            Architecture::Arm64,
            Duration::from_secs(600),
            Duration::from_secs(1),
            Duration::from_secs(10),
        )
        .unwrap(),
        reader,
        writer,
    )
    .unwrap();
    let (reader, writer) = split(host);
    let mut client = ControlClient::start(
        ClientConfig::new(environment, Architecture::Arm64, Duration::from_secs(1)).unwrap(),
        reader,
        writer,
    )
    .unwrap();
    assert_eq!(
        client.next(deadline()).await.unwrap(),
        ClientEvent::RelayReady
    );
    let mut first_poll = Box::pin(session.next(deadline()));
    assert!(
        first_poll
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(
        client.next(deadline()).await.unwrap(),
        ClientEvent::BootstrapReady
    );
    drop(first_poll);

    RuntimeFixture {
        address,
        session,
        relay,
        client,
    }
}
