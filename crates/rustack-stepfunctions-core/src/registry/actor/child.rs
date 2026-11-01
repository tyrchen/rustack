//! Atomic retained child start, watch capture and owned relation publication.

use std::{collections::VecDeque, sync::Arc};

use tokio::time::Instant;

use super::{State, uuid};
use crate::{
    ExecutionArn, ExecutionSnapshot, RegistryResult,
    registry::child::{ChildError, ChildExecution, Lineage, Request, owner::Entry as ChildEntry},
};

impl State {
    pub(super) async fn dispatch_child(&mut self, request: Request) {
        self.children.pending = Some(request);
        self.io.ready.clear();
        let needs_entropy = self
            .children
            .pending
            .as_ref()
            .is_some_and(|request| request.context.parameters.name.is_none());
        let prepared = if needs_entropy {
            self.entropy
                .spawn_blocking(|| Ok(VecDeque::from([uuid()?, uuid()?])));
            self.await_entropy().await.map_err(ChildError::from)
        } else {
            Ok(())
        };
        self.maintain(Instant::now());
        let result = prepared.and_then(|()| self.start_child());
        if let Some(request) = self.children.pending.take() {
            let result = result.map(|snapshot| {
                if request.context.pattern.owned() {
                    self.children.entries.insert(
                        self.children.next,
                        ChildEntry {
                            parent: request.context.parent.record.clone(),
                            child: snapshot.clone(),
                            control: request.control.clone(),
                            deadline: request.context.deadline,
                            _permit: request.permit,
                        },
                    );
                }
                ChildExecution {
                    snapshot,
                    control: request.control,
                    pattern: request.context.pattern,
                }
            });
            let _sent = request.reply.send(result);
        }
        self.maintain(Instant::now());
    }
    fn pending_child(&self) -> Result<&Request, ChildError> {
        if *self.io.closing.borrow() {
            return Err(ChildError::Unavailable);
        }
        let pending = self
            .children
            .pending
            .as_ref()
            .ok_or(ChildError::Unavailable)?;
        if pending.control.cancelled() {
            return Err(ChildError::Cancelled);
        }
        if Instant::now() >= pending.context.deadline {
            return Err(ChildError::Deadline);
        }
        let parent = &pending.context.parent.record;
        let registered = self
            .executions
            .records
            .get(parent.identity.arn.as_str())
            .ok_or(ChildError::Parent)?;
        if !Arc::ptr_eq(parent, &registered.record) || parent.handle.snapshot().outcome().is_some()
        {
            return Err(ChildError::Parent);
        }
        Ok(pending)
    }
    fn start_child(&mut self) -> Result<ExecutionSnapshot, ChildError> {
        let pending = self.pending_child()?;
        let parent = &pending.context.parent.record;
        let lineage = Lineage::descendant(parent)?;
        let machine = pending.context.parameters.machine.clone();
        let input = pending.context.parameters.input.clone();
        let pattern = pending.context.pattern;
        let name = pending.context.parameters.name.clone();
        let name = match name {
            Some(name) => name,
            None => self.next_uuid()?.parse()?,
        };
        let arn = ExecutionArn::new(machine.clone(), name.clone())?;
        let pending = self
            .children
            .pending
            .as_ref()
            .ok_or(ChildError::Unavailable)?;
        self.children.check_cycle(
            pending.context.parent.record.as_ref(),
            arn.as_str(),
            pattern,
            lineage.0.len(),
        )?;
        self.children.next = self
            .children
            .next
            .checked_add(1)
            .ok_or(ChildError::Capacity)?;
        // Graph validation is bounded CPU work; refresh independently changing
        // parent/control/deadline observations at the registration boundary.
        self.pending_child()?;
        let result = self.start(&machine, Some(&name), &input, None, lineage)?;
        match result {
            RegistryResult::Execution(snapshot) => Ok(snapshot),
            _ => Err(ChildError::Unavailable),
        }
    }
}
