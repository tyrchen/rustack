//! Test-only peer for exercising public handle ownership without external processes.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rustack_lambda_runtime::session::RuntimeResult;

use super::*;

pub(in crate::docker) struct Peer {
    calls: mpsc::Receiver<Call>,
    signals: Signals,
    done: watch::Sender<Option<Result<(), EnvironmentError>>>,
}
pub(in crate::docker) fn pair() -> (EnvironmentHandle, Peer) {
    let (calls, mailbox) = mpsc::channel(4);
    let signals = Signals::new();
    signals.phase(EnvironmentPhase::Idle);
    let (done, settlement) = watch::channel(None);
    (
        EnvironmentHandle {
            id: EnvironmentId::default(),
            calls,
            signals: signals.clone(),
            settlement,
        },
        Peer {
            calls: mailbox,
            signals,
            done,
        },
    )
}
impl Peer {
    pub(in crate::docker) async fn serve(self, count: Arc<AtomicUsize>) {
        self.serve_inner(count, None, None).await;
    }
    pub(in crate::docker) async fn serve_gated(
        self,
        count: Arc<AtomicUsize>,
        release: oneshot::Receiver<()>,
        stopping: oneshot::Sender<()>,
    ) {
        self.serve_inner(count, Some(release), Some(stopping)).await;
    }
    async fn serve_inner(
        mut self,
        count: Arc<AtomicUsize>,
        release: Option<oneshot::Receiver<()>>,
        stopping: Option<oneshot::Sender<()>>,
    ) {
        let mut stop = self.signals.stop.subscribe();
        loop {
            tokio::select! { biased;
                _ = stop.wait_for(|stop| *stop) => break,
                call = self.calls.recv() => {
                    let Some(call) = call else { break; };
                    count.fetch_add(1, Ordering::AcqRel);
                    let _ = call.reply.send(Ok(EnvironmentResponse { result: RuntimeResult::Success(Payload::try_from(Bytes::from_static(b"{}" )).unwrap()), logs: Bytes::new() }));
                }
            }
        }
        if let Some(stopping) = stopping {
            let _ = stopping.send(());
        }
        if let Some(release) = release {
            let _ = release.await;
        }
        self.signals.phase(EnvironmentPhase::Removed);
        self.done.send_replace(Some(Ok(())));
    }
}
