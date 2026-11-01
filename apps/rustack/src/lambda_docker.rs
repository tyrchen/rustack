//! Retained diagnostic-only listener while the exact Docker owner qualifies.

use std::{future::Future, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use rustack_lambda_core::docker::{DockerRuntime, GatewayInstanceId};
use tokio::{
    net::TcpListener,
    runtime::Handle,
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout_at},
};

pub(crate) async fn qualify(
    listener: Arc<TcpListener>,
    owner: &mut DockerRuntime,
    instance: GatewayInstanceId,
    deadline: Instant,
) -> Result<()> {
    observe(
        listener,
        instance,
        async {
            owner
                .qualify(deadline)
                .await
                .context("qualify production Docker runtime")
        },
        deadline,
    )
    .await
}

pub(crate) async fn observe<T>(
    listener: Arc<TcpListener>,
    instance: GatewayInstanceId,
    work: impl Future<Output = Result<T>>,
    deadline: Instant,
) -> Result<T> {
    let mut owner = DiagnosticOwner::start(listener, instance, deadline);
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("register startup SIGTERM")?;
    tokio::pin!(work);
    let result = tokio::select! {biased;
        result=tokio::signal::ctrl_c()=>result.context("observe startup SIGINT").and_then(|()|Err(anyhow::anyhow!("runtime startup interrupted"))),
        _=terminate.recv()=>Err(anyhow::anyhow!("runtime startup interrupted")),
        ()=tokio::time::sleep_until(deadline)=>Err(anyhow::anyhow!("runtime startup observation expired")),
        result=&mut work=>result,
        result=owner.join()=>result.and_then(|()|Err(anyhow::anyhow!("startup diagnostic listener stopped before activation"))),
    };
    // The deadline selects a result; it never cancels this retained join.
    let cleanup = owner.shutdown().await;
    let output = result?;
    cleanup?;
    ensure!(
        Instant::now() < deadline,
        "runtime startup observation expired"
    );
    Ok(output)
}

#[derive(Debug)]
struct DiagnosticOwner {
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<()>>>,
    settled: Option<Result<(), Arc<anyhow::Error>>>,
    runtime: Handle,
}
impl DiagnosticOwner {
    fn start(listener: Arc<TcpListener>, instance: GatewayInstanceId, deadline: Instant) -> Self {
        let (stop, stopped) = watch::channel(false);
        let runtime = Handle::current();
        Self {
            stop,
            task: Some(runtime.spawn(serve(listener, instance, stopped, deadline))),
            settled: None,
            runtime,
        }
    }
    async fn shutdown(&mut self) -> Result<()> {
        self.stop.send_replace(true);
        self.join().await
    }
    async fn join(&mut self) -> Result<()> {
        if self.settled.is_none() {
            let result = match self.task.as_mut() {
                Some(task) => task
                    .await
                    .context("startup diagnostic supervisor failed")
                    .and_then(|result| result),
                None => Ok(()),
            };
            self.task.take();
            self.settled = Some(result.map_err(Arc::new));
        }
        match &self.settled {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(anyhow::anyhow!(
                "startup diagnostic cleanup failed: {error:#}"
            )),
            None => bail!("startup diagnostic unsettled"),
        }
    }
}
impl Drop for DiagnosticOwner {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        if let Some(task) = self.task.take() {
            // An abandoned observer cannot abort the actor's child joins. This
            // retained reaper observes the exact supervisor on the captured runtime.
            self.runtime.spawn(async move {
                if !matches!(task.await, Ok(Ok(()))) {
                    tracing::error!("abandoned startup diagnostic cleanup failed");
                }
            });
        }
    }
}
async fn serve(
    listener: Arc<TcpListener>,
    instance: GatewayInstanceId,
    mut stopped: watch::Receiver<bool>,
    deadline: Instant,
) -> Result<()> {
    let service = crate::gateway::GatewayService::new(Vec::new())
        .with_runtime_identity(instance)
        .health_only();
    service.state().drain();
    let mut tasks = JoinSet::new();
    let result=async {loop {
        tokio::select! {biased;
            _=stopped.wait_for(|stop|*stop)=>break Ok(()),
            ()=tokio::time::sleep_until(deadline)=>break Ok(()),
            result=tasks.join_next(),if !tasks.is_empty()=>{if let Some(Err(error))=result{bail!("startup diagnostic worker failed: {error}");}},
            connection=listener.accept()=>{
                let (stream,_)=connection.context("accept startup diagnostic connection")?;
                if tasks.len()>=8{drop(stream);continue;}
                let service=service.clone();
                tasks.spawn(async move {
                    let mut http=http1::Builder::new();
                    http.timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(rustack_core::settings::budgets().header_seconds)).max_headers(32).max_buf_size(16*1024);
                    let _=timeout_at(deadline,http.serve_connection(TokioIo::new(stream),service)).await;
                });
            }
        }
    }}.await;
    tasks.abort_all();
    let mut joined = true;
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            joined &= error.is_cancelled();
        }
    }
    result?;
    ensure!(joined, "startup diagnostic workers failed during join");
    Ok(())
}
pub(crate) async fn shutdown(owner: &mut Option<DockerRuntime>, deadline: Instant) -> Result<()> {
    if let Some(owner) = owner {
        owner
            .shutdown(deadline)
            .await
            .context("Docker runtime cleanup incomplete")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::future::pending;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        time::{sleep, timeout},
    };

    use super::*;
    #[tokio::test]
    async fn test_should_join_held_diagnostic_connections_before_deadline_result() {
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
        let address = listener.local_addr().unwrap();
        let observed = listener.clone();
        let job = tokio::spawn(observe(
            observed,
            GatewayInstanceId::generate().unwrap(),
            pending::<Result<()>>(),
            Instant::now() + Duration::from_millis(50),
        ));
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(b"GET /_rustack/runtime-probe HTTP/1.1\r\n")
            .await
            .unwrap();
        assert!(job.await.unwrap().is_err());
        assert_eq!(Arc::strong_count(&listener), 1);
        assert_eq!(
            timeout(Duration::from_secs(1), client.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(TcpListener::bind(address).await.is_err());
    }
    #[tokio::test]
    async fn test_should_retain_diagnostic_join_after_startup_observer_abandonment() {
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
        let address = listener.local_addr().unwrap();
        let job = tokio::spawn(observe(
            listener.clone(),
            GatewayInstanceId::generate().unwrap(),
            pending::<Result<()>>(),
            Instant::now() + Duration::from_secs(5),
        ));
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(b"GET /_rustack/runtime-probe HTTP/1.1\r\n")
            .await
            .unwrap();
        sleep(Duration::from_millis(10)).await;
        job.abort();
        assert!(job.await.unwrap_err().is_cancelled());
        timeout(Duration::from_secs(1), async {
            while Arc::strong_count(&listener) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(1), client.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}
