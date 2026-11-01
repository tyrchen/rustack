//! Bounded log writer whose active phase spans an authenticated health barrier.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use ureq::Agent;

use super::{
    Result,
    fixture_collector::{barrier, variable},
};

const PROGRESS_WINDOW: Duration = Duration::from_secs(1);

pub(super) fn prepare(agent: &Agent) -> Result<()> {
    let name = variable("AWS_LAMBDA_FUNCTION_NAME")?;
    let response = barrier(agent, "flood-ready", &json!({"functionName": name}))?;
    if response.get("ready") != Some(&json!(true)) {
        return Err("fixture flood channel was not prepared");
    }
    Ok(())
}

pub(super) fn flood(agent: &Agent, request: &str) -> Result<()> {
    let cutoff = Instant::now() + PROGRESS_WINDOW;
    let stop = Arc::new(AtomicBool::new(false));
    let bytes = Arc::new(AtomicU64::new(0));
    let (sender, receiver) = sync_channel(1);
    let stopped = stop.clone();
    let written = bytes.clone();
    let writer = thread::Builder::new()
        .name("fixture-log-writer".to_owned())
        .spawn(move || {
            let mut output = io::stderr().lock();
            let chunk = [b'x'; 1024];
            for _ in 0..1024 {
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                output
                    .write_all(&chunk)
                    .map_err(|_| "fixture log output failed")?;
                output.flush().map_err(|_| "fixture log flush failed")?;
                written.fetch_add(1024, Ordering::Release);
                notify_progress(&sender)?;
                thread::sleep(Duration::from_millis(1));
            }
            Ok::<(), &'static str>(())
        })
        .map_err(|_| "fixture log writer could not start")?;
    let observed = (|| {
        let before = progress(&receiver, &bytes, 0, cutoff, Instant::now)?;
        if before >= 1024 * 1024 {
            return Err("fixture flood did not enter");
        }
        let name = variable("AWS_LAMBDA_FUNCTION_NAME")?;
        let release = barrier(
            agent,
            "flood-begin",
            &json!({
                "functionName":name,"requestId":request,"beforeBytes":before
            }),
        )?;
        if release.get("release") != Some(&json!(true)) {
            return Err("fixture flood was not released");
        }
        progress(&receiver, &bytes, before, cutoff, Instant::now)?;
        Ok((name, before))
    })();
    stop.store(true, Ordering::Release);
    let joined = writer.join().map_err(|_| "fixture log writer panicked")?;
    joined?;
    let (name, before) = observed?;
    let after = bytes.load(Ordering::Acquire);
    if after <= before || after >= 1024 * 1024 {
        return Err("fixture flood phase was exhausted or inactive");
    }
    let accepted = barrier(
        agent,
        "flood-end",
        &json!({
            "functionName":name,"requestId":request,"beforeBytes":before,"afterBytes":after
        }),
    )?;
    if accepted.get("recorded") != Some(&json!(true)) {
        return Err("fixture flood end was not recorded");
    }
    Ok(())
}

fn notify_progress(sender: &SyncSender<()>) -> Result<()> {
    match sender.try_send(()) {
        Ok(()) | Err(TrySendError::Full(())) => Ok(()),
        Err(TrySendError::Disconnected(())) => Err("fixture log observer disconnected"),
    }
}

fn progress(
    receiver: &Receiver<()>,
    bytes: &AtomicU64,
    previous: u64,
    cutoff: Instant,
    mut now: impl FnMut() -> Instant,
) -> Result<u64> {
    loop {
        let remaining = cutoff
            .checked_duration_since(now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or("fixture log progress expired")?;
        let current = bytes.load(Ordering::Acquire);
        if current > previous {
            return if now() < cutoff {
                Ok(current)
            } else {
                Err("fixture log progress expired")
            };
        }
        match receiver.recv_timeout(remaining) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => return Err("fixture log progress expired"),
            Err(RecvTimeoutError::Disconnected) => {
                return Err("fixture log writer disconnected");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc::sync_channel,
        },
        time::{Duration, Instant},
    };

    use super::{PROGRESS_WINDOW, notify_progress, progress};

    #[test]
    fn test_should_coalesce_notifications_without_blocking_the_writer() {
        let (sender, receiver) = sync_channel(1);
        assert!(notify_progress(&sender).is_ok());
        assert!(notify_progress(&sender).is_ok());
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err());
        drop(receiver);
        assert_eq!(
            notify_progress(&sender),
            Err("fixture log observer disconnected")
        );
    }

    #[test]
    fn test_should_fail_disconnected_progress_without_waiting() {
        let (sender, receiver) = sync_channel(1);
        drop(sender);
        assert_eq!(
            progress(
                &receiver,
                &AtomicU64::new(0),
                0,
                Instant::now() + PROGRESS_WINDOW,
                Instant::now
            ),
            Err("fixture log writer disconnected")
        );
    }

    #[test]
    fn test_should_reject_stale_notifications_at_the_original_cutoff() {
        let (sender, receiver) = sync_channel(1);
        assert!(notify_progress(&sender).is_ok());
        let began = Instant::now();
        let cutoff = began + PROGRESS_WINDOW;
        let mut times = [began, cutoff].into_iter();
        assert_eq!(
            progress(&receiver, &AtomicU64::new(1024), 1024, cutoff, || times
                .next()
                .unwrap_or(cutoff)),
            Err("fixture log progress expired")
        );
    }

    #[test]
    fn test_should_accept_delayed_first_progress_inside_the_invocation_window() {
        let (sender, receiver) = sync_channel(1);
        assert!(notify_progress(&sender).is_ok());
        let began = Instant::now();
        let delayed = began + Duration::from_millis(150);
        let bytes = AtomicU64::new(0);
        let mut first = true;
        assert_eq!(
            progress(&receiver, &bytes, 0, began + PROGRESS_WINDOW, || {
                if first {
                    first = false;
                    began
                } else {
                    bytes.store(1024, Ordering::Release);
                    delayed
                }
            }),
            Ok(1024)
        );
        assert_eq!(
            progress(&receiver, &bytes, 0, began + PROGRESS_WINDOW, || began
                + PROGRESS_WINDOW),
            Err("fixture log progress expired")
        );
    }
}
