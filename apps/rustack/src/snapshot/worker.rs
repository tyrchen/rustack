//! Retain typed worker evidence without displaying potentially secret panic text.

use std::{error::Error, fmt};

use tokio::task::JoinError;

pub(super) struct WorkerFailure {
    source: JoinError,
}
impl WorkerFailure {
    pub(super) fn new(source: JoinError) -> Self {
        Self { source }
    }
    /// Explicit typed inspection; raw `JoinError` Display must not reach diagnostics.
    pub(super) fn join_error(&self) -> &JoinError {
        &self.source
    }
}
impl fmt::Debug for WorkerFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Display for WorkerFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.join_error().is_panic() {
            "snapshot worker panicked"
        } else {
            "snapshot worker cancelled"
        })
    }
}
// The original is retained for explicit typed inspection. Chaining it would leak
// JoinError's panic payload through main's formatted anyhow diagnostic chain.
impl Error for WorkerFailure {}

#[cfg(test)]
mod tests {
    use std::future::pending;

    use anyhow::{Context, Result};

    use super::*;

    fn panic_source() {
        panic!("private-panic-payload");
    }
    #[tokio::test]
    async fn test_should_retain_panic_and_cancellation_without_exposing_panic_text() -> Result<()> {
        for panic in [false, true] {
            let task = tokio::spawn(async move {
                if panic {
                    panic_source();
                } else {
                    pending::<()>().await;
                }
            });
            if !panic {
                task.abort();
            }
            let original = task.await.err().context("missing controlled failure")?;
            let failure = WorkerFailure::new(original);
            assert_eq!(failure.join_error().is_panic(), panic);
            assert_eq!(failure.join_error().is_cancelled(), !panic);
            let error = anyhow::Error::new(failure).context("outer archive owner");
            for diagnostic in [
                format!("{error}"),
                format!("{error:#}"),
                format!("{error:?}"),
            ] {
                assert!(!diagnostic.contains("private-panic-payload"));
            }
            assert!(error.downcast_ref::<WorkerFailure>().is_some());
        }
        Ok(())
    }
}
