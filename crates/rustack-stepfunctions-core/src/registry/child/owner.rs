//! Registry-retained child relations and bounded cancellation graph traversal.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use tokio::{
    sync::{OwnedSemaphorePermit, mpsc},
    time::Instant,
};

use super::{
    super::{Shared, types::ExecutionRecord},
    ChildError, ChildPattern, Control, Request,
};
use crate::{CleanupState, ExecutionFailure, ExecutionSnapshot, TaskFailure};

#[derive(Debug)]
pub(in crate::registry) struct Inbox {
    pub(in crate::registry) requests: mpsc::Receiver<Request>,
    pub(in crate::registry) registry: Arc<Shared>,
}
#[derive(Debug)]
pub(in crate::registry) struct Entry {
    pub(in crate::registry) parent: Arc<ExecutionRecord>,
    pub(in crate::registry) child: ExecutionSnapshot,
    pub(in crate::registry) control: Arc<Control>,
    pub(in crate::registry) deadline: Instant,
    pub(in crate::registry) _permit: OwnedSemaphorePermit,
}
#[derive(Debug)]
pub(in crate::registry) struct Owner {
    pub(in crate::registry) pending: Option<Request>,
    pub(in crate::registry) entries: BTreeMap<u64, Entry>,
    pub(in crate::registry) next: u64,
    pub(in crate::registry) registry: Arc<Shared>,
}
impl Owner {
    pub(in crate::registry) fn new(registry: Arc<Shared>) -> Self {
        Self {
            pending: None,
            entries: BTreeMap::new(),
            next: 0,
            registry,
        }
    }
    pub(in crate::registry) fn check_cycle(
        &self,
        parent: &ExecutionRecord,
        child: &str,
        pattern: ChildPattern,
        depth: usize,
    ) -> Result<(), ChildError> {
        if !pattern.owned() {
            return Ok(());
        }
        let mut graph: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut levels = BTreeMap::new();
        for entry in self.entries.values() {
            graph
                .entry(entry.parent.identity.arn.as_str())
                .or_default()
                .push(entry.child.arn().as_str());
            levels
                .entry(entry.parent.identity.arn.as_str())
                .or_insert(entry.parent.lineage.0.len());
            levels
                .entry(entry.child.arn().as_str())
                .or_insert(entry.child.nesting_depth());
        }
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([child]);
        while let Some(current) = queue.pop_front() {
            if current == parent.identity.arn.as_str() {
                return Err(ChildError::Cycle);
            }
            if !seen.insert(current) {
                continue;
            }
            if let Some(children) = graph.get(current) {
                queue.extend(children);
            }
        }
        graph
            .entry(parent.identity.arn.as_str())
            .or_default()
            .push(child);
        levels
            .entry(parent.identity.arn.as_str())
            .or_insert(parent.lineage.0.len());
        levels.entry(child).or_insert(depth);
        let mut incoming: BTreeMap<&str, usize> = levels.keys().map(|key| (*key, 0)).collect();
        for children in graph.values() {
            for child in children {
                let count = incoming.get_mut(child).ok_or(ChildError::Invalid)?;
                *count = count.checked_add(1).ok_or(ChildError::Capacity)?;
            }
        }
        let mut queue: VecDeque<&str> = incoming
            .iter()
            .filter_map(|(key, count)| (*count == 0).then_some(*key))
            .collect();
        let mut visited = 0_usize;
        while let Some(current) = queue.pop_front() {
            visited = visited.checked_add(1).ok_or(ChildError::Capacity)?;
            let level = levels.get(current).copied().ok_or(ChildError::Invalid)?;
            if level > 16 {
                return Err(ChildError::Capacity);
            }
            if let Some(children) = graph.get(current) {
                for child in children {
                    let next = level.checked_add(1).ok_or(ChildError::Capacity)?;
                    let value = levels.get_mut(child).ok_or(ChildError::Invalid)?;
                    *value = (*value).max(next);
                    let count = incoming.get_mut(child).ok_or(ChildError::Invalid)?;
                    *count = count.checked_sub(1).ok_or(ChildError::Invalid)?;
                    if *count == 0 {
                        queue.push_back(child);
                    }
                }
            }
        }
        if visited != incoming.len() {
            return Err(ChildError::Cycle);
        }
        Ok(())
    }
    pub(in crate::registry) fn maintain(&mut self, now: Instant) {
        {
            let mut graph: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
            let mut queue = VecDeque::new();
            for (id, entry) in &self.entries {
                graph
                    .entry(entry.parent.identity.arn.as_str())
                    .or_default()
                    .push(*id);
                if entry.control.cancelled()
                    || now >= entry.deadline
                    || entry.parent.handle.snapshot().outcome().is_some()
                {
                    queue.push_back(*id);
                }
            }
            let mut seen = BTreeSet::new();
            while let Some(id) = queue.pop_front() {
                if !seen.insert(id) {
                    continue;
                }
                if let Some(entry) = self.entries.get(&id) {
                    entry
                        .child
                        .handle()
                        .request_stop(TaskFailure(ExecutionFailure::named(
                            "Rustack.ParentCancelled",
                            "Owned child execution cancelled",
                        )));
                    if let Some(children) = graph.get(entry.child.arn().as_str()) {
                        queue.extend(children);
                    }
                }
            }
        }
        self.entries
            .retain(|_, entry| entry.child.handle().snapshot().cleanup() != CleanupState::Clean);
    }
}
