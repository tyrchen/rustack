//! Controlled gateway protocol peer; actual binary route has separate socket tests.

use std::{
    io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::docker::{GatewayInstanceId, GatewayReply};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
    time::{Instant, timeout_at},
};
use typed_builder::TypedBuilder;

#[derive(Debug, TypedBuilder)]
pub(super) struct Gateway {
    pub address: SocketAddr,
    pub instance: GatewayInstanceId,
    seen: watch::Receiver<usize>,
    pub requests: Arc<AtomicUsize>,
    pub closed: Arc<AtomicBool>,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<()>>>,
    settled: Option<Result<(), Arc<anyhow::Error>>>,
}
impl Gateway {
    pub async fn start(hold: bool) -> Result<Self> {
        let listener = TcpListener::bind("0.0.0.0:0").await?;
        let address = listener.local_addr()?;
        let instance =
            GatewayInstanceId::generate_owned(Instant::now() + Duration::from_secs(5)).await?;
        let (stop, mut stopped) = watch::channel(false);
        let (observed, seen) = watch::channel(0);
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let eof = closed.clone();
        let bytes = serde_json::to_vec(&GatewayReply::new(instance))?;
        let task = tokio::spawn(async move {
            let work = async {
                loop {
                    let (mut stream, _) = listener.accept().await?;
                    let head =
                        read_head(&mut stream, Instant::now() + Duration::from_secs(10)).await?;
                    ensure!(
                        head.starts_with(b"GET /_rustack/runtime-probe HTTP/1.1\r\n"),
                        "wrong endpoint request"
                    );
                    let raw: String = instance.into();
                    ensure!(
                        !String::from_utf8_lossy(&head).contains(&raw),
                        "expected identity leaked into GET"
                    );
                    let count = counter.fetch_add(1, Ordering::AcqRel).saturating_add(1);
                    ensure!(count <= 2, "probe retried excessively");
                    observed.send_replace(count);
                    if hold {
                        let mut byte = [0];
                        let result = timeout_at(
                            Instant::now() + Duration::from_secs(60),
                            stream.read(&mut byte),
                        )
                        .await?;
                        ensure!(
                            matches!(result, Ok(0))
                                || result.is_err_and(|error| matches!(
                                    error.kind(),
                                    io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
                                )),
                            "abandoned connection still transmitted data"
                        );
                        eof.store(true, Ordering::Release);
                    } else {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: \
                             no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        );
                        stream.write_all(head.as_bytes()).await?;
                        stream.write_all(&bytes).await?;
                        stream.shutdown().await?;
                    }
                }
            };
            tokio::select! {biased;_ = stopped.changed()=>Ok(()),result=work=>result,}
        });
        Ok(Self::builder()
            .address(address)
            .instance(instance)
            .seen(seen)
            .requests(requests)
            .closed(closed)
            .stop(stop)
            .task(Some(task))
            .settled(None)
            .build())
    }
    pub async fn observed(&mut self, deadline: Instant) -> Result<()> {
        while *self.seen.borrow() == 0 {
            timeout_at(deadline, self.seen.changed())
                .await
                .context("gateway request observer expired")??;
        }
        Ok(())
    }
    pub async fn shutdown(&mut self) -> Result<()> {
        self.stop.send_replace(true);
        if self.settled.is_none() {
            let result = match self.task.as_mut() {
                Some(task) => timeout_at(Instant::now() + Duration::from_secs(10), task)
                    .await
                    .context("gateway join deadline")?
                    .context("gateway worker failed")
                    .and_then(|result| result),
                None => Ok(()),
            };
            self.task.take();
            self.settled = Some(result.map_err(Arc::new));
        }
        match &self.settled {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(anyhow::anyhow!("gateway cleanup failed: {error:#}")),
            None => anyhow::bail!("gateway cleanup unsettled"),
        }
    }
}
impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}
async fn read_head(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>> {
    timeout_at(deadline, async {
        let mut bytes = Vec::new();
        loop {
            ensure!(bytes.len() < 4096, "oversized fixture request");
            bytes.push(stream.read_u8().await?);
            if bytes.ends_with(b"\r\n\r\n") {
                return Ok(bytes);
            }
        }
    })
    .await
    .context("fixture request deadline")?
}
