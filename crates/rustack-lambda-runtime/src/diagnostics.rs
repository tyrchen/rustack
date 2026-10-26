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
    let dependency = ["hyper", "hyper_util"].iter().any(|name| {
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

pub(crate) fn runtime_dispatch() -> Dispatch {
    dispatcher::get_default(|parent| {
        Dispatch::new(RuntimeSubscriber {
            parent: parent.clone(),
        })
    })
}
