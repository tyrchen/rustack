//! Retained socket proxy with a causal startup barrier for one crashed owner.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use tempfile::TempDir;
use tokio::{
    io::copy_bidirectional,
    net::{UnixListener, UnixStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::timeout,
};

#[derive(Debug)]
pub(super) struct Gate {
    directory: Option<TempDir>,
    pub socket: PathBuf,
    target: PathBuf,
    held: watch::Sender<bool>,
    entered: watch::Receiver<bool>,
    stop: watch::Sender<bool>,
    root: Option<JoinHandle<Result<()>>>,
}
impl Gate {
    pub fn start(target: &Path) -> Result<Self> {
        let mut directory = tempfile::tempdir()?;
        directory.disable_cleanup(true);
        let socket = directory.path().join("recovery.sock");
        let listener = UnixListener::bind(&socket)?;
        let (held, barrier) = watch::channel(false);
        let (observed, entered) = watch::channel(false);
        let (stop, shutdown) = watch::channel(false);
        let target = target.to_owned();
        let root = tokio::spawn(serve(listener, target.clone(), barrier, observed, shutdown));
        Ok(Self {
            directory: Some(directory),
            socket,
            target,
            held,
            entered,
            stop,
            root: Some(root),
        })
    }
    pub fn arm(&mut self) {
        self.held.send_replace(true);
    }
    pub async fn blocked(&mut self) -> Result<()> {
        timeout(Duration::from_secs(20), async {
            while !*self.entered.borrow() {
                self.entered.changed().await?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("recovery did not reach held socket")??;
        ensure!(*self.held.borrow(), "recovery barrier released early");
        Ok(())
    }
    pub fn release(&self) {
        self.held.send_replace(false);
    }
    pub async fn disconnect(&mut self) -> Result<()> {
        self.release();
        self.stop.send_replace(true);
        if let Some(root) = &mut self.root {
            timeout(Duration::from_secs(10), root)
                .await
                .context("proxy root retained")???;
            self.root = None;
        }
        Ok(())
    }
    pub async fn resume(&mut self) -> Result<()> {
        ensure!(self.root.is_none(), "proxy resumed before join");
        tokio::fs::remove_file(&self.socket).await?;
        let listener = UnixListener::bind(&self.socket)?;
        let (held, barrier) = watch::channel(false);
        let (observed, entered) = watch::channel(false);
        let (stop, shutdown) = watch::channel(false);
        self.held = held;
        self.entered = entered;
        self.stop = stop;
        self.root = Some(tokio::spawn(serve(
            listener,
            self.target.clone(),
            barrier,
            observed,
            shutdown,
        )));
        Ok(())
    }
    pub async fn close(&mut self) -> Result<()> {
        self.disconnect().await?;
        if let Some(mut directory) = self.directory.take() {
            directory.disable_cleanup(false);
        }
        Ok(())
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.held.send_replace(false);
        self.stop.send_replace(true);
        // The retained actor is intentionally detached on observer loss; its
        // shutdown channel joins every connection. Evidence stays retained.
        if let Some(root) = self.root.take() {
            drop(root);
        }
    }
}
async fn serve(
    listener: UnixListener,
    target: PathBuf,
    mut held: watch::Receiver<bool>,
    entered: watch::Sender<bool>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let mut children = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            Some(result) = children.join_next() => { result.context("proxy connection panicked")?; },
            accepted = listener.accept(), if children.len() < 16 => {
                let (mut inbound, _) = accepted?;
                if *held.borrow() {
                    entered.send_replace(true);
                    loop {
                        if !*held.borrow() {break;}
                        tokio::select! { _ = stop.changed() => break, _ = held.changed() => {} }
                    }
                    if *stop.borrow() {break;}
                }
                let target = target.clone();
                let mut connection_stop = stop.clone();
                children.spawn(async move {
                    tokio::select! {
                        _ = connection_stop.changed() => {},
                        _ = timeout(Duration::from_secs(120), async {
                            let mut outbound = UnixStream::connect(target).await?;
                            copy_bidirectional(&mut inbound, &mut outbound).await
                        }) => {},
                    }
                });
            }
        }
    }
    while let Some(result) = children.join_next().await {
        result.context("proxy connection panicked")?;
    }
    Ok(())
}
