//! Mandatory per-task dependency filter, independent of application `RUST_LOG`.
//!
//! Hyper's chunk decoder can put partial trailer bytes in TRACE fields. Keep
//! its verbose events/spans out before delegating to the application's subscriber.

use tracing_core::{
    Dispatch, Event, Interest, Level, Metadata, Subscriber, dispatcher,
    span::{Attributes, Current, Id, Record},
};

#[derive(Debug)]
struct RuntimeSubscriber {
    parent: Dispatch,
}

fn allowed(metadata: &Metadata<'_>) -> bool {
    let target = metadata.target();
    let tls = ["rustls", "tokio_rustls"].iter().any(|name| {
        target == *name
            || target
                .strip_prefix(name)
                .is_some_and(|suffix| suffix.starts_with("::"))
    });
    if tls {
        return false;
    }
    let dependency = ["hyper", "hyper_util", "bollard"].iter().any(|name| {
        target == *name
            || target
                .strip_prefix(name)
                .is_some_and(|suffix| suffix.starts_with("::"))
    });
    !dependency || *metadata.level() <= Level::INFO
}

impl Subscriber for RuntimeSubscriber {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if allowed(metadata) {
            self.parent.register_callsite(metadata)
        } else {
            Interest::never()
        }
    }
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        allowed(metadata) && self.parent.enabled(metadata)
    }
    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        self.parent.new_span(attributes)
    }
    fn record(&self, id: &Id, values: &Record<'_>) {
        self.parent.record(id, values);
    }
    fn record_follows_from(&self, id: &Id, follows: &Id) {
        self.parent.record_follows_from(id, follows);
    }
    fn event(&self, event: &Event<'_>) {
        // Recheck even when a different live dispatcher enabled this callsite.
        if allowed(event.metadata()) {
            self.parent.event(event);
        }
    }
    fn enter(&self, id: &Id) {
        self.parent.enter(id);
    }
    fn exit(&self, id: &Id) {
        self.parent.exit(id);
    }
    fn clone_span(&self, id: &Id) -> Id {
        self.parent.clone_span(id)
    }
    fn try_close(&self, id: Id) -> bool {
        self.parent.try_close(id)
    }
    fn current_span(&self) -> Current {
        self.parent.current_span()
    }
}

/// Capture application context while denying verbose transport dependency logs.
///
/// The policy covers verbose Hyper/`hyper_util`/Bollard and all rustls/
/// tokio-rustls events independently of `RUST_LOG`; TLS diagnostics may contain
/// operator server names or peer certificate data even at warning levels.
/// Apply the dispatch to construction and every poll/spawned transport worker.
///
/// ```
/// let dispatch = rustack_lambda_runtime::diagnostics::runtime_dispatch();
/// tracing::dispatcher::with_default(&dispatch, || tracing::debug!("application event"));
/// ```
#[must_use]
pub fn runtime_dispatch() -> Dispatch {
    dispatcher::get_default(|parent| {
        Dispatch::new(RuntimeSubscriber {
            parent: parent.clone(),
        })
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    use tracing::{Level, dispatcher};

    use super::runtime_dispatch;

    #[derive(Debug, Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut output = self.0.lock().unwrap();
            if output.len().saturating_add(bytes.len()) > 16_384 {
                return Err(io::Error::other("fixture capture exceeded budget"));
            }
            output.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn test_should_filter_all_tls_peer_events_while_preserving_application_diagnostics() {
        let capture = Capture(Arc::default());
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let dispatch = runtime_dispatch();
            dispatcher::with_default(&dispatch, || {
                tracing::trace!(target:"rustls","PRIVATE-TLS-TRACE");
                tracing::warn!(target:"rustls::client","PRIVATE-PEER-CERT");
                tracing::error!(target:"tokio_rustls","PRIVATE-SNI");
                tracing::info!("owned_application_event");
            });
        });
        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(output.contains("owned_application_event"));
        assert!(!output.contains("PRIVATE"));
    }
}
