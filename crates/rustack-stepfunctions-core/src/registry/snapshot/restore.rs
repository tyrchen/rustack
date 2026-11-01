//! Exclusive startup preparation and fully charged actor replacement graphs.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use tokio::sync::Semaphore;

use super::{
    super::{
        Registry, RegistryError, RegistryLimits, RegistryReceipt, Shared, domain::redacted,
        types::MachineSnapshot,
    },
    RegistrySnapshot, SnapshotLimits,
};
use crate::engine::HistoryRestorer;

mod history;
mod validate;

use super::{
    super::{Operation, actor::Entry, retention::Retention},
    ProfileStates,
};

const OPEN: u8 = 0;
const PREPARING: u8 = 1;
const SERVING: u8 = 2;

#[derive(Debug)]
pub(in crate::registry) struct Pools {
    pub(in crate::registry) revisions: Arc<Semaphore>,
    pub(in crate::registry) tags: Arc<Semaphore>,
    pub(in crate::registry) records: Arc<Semaphore>,
}
pub(in crate::registry) struct Admission {
    pub(in crate::registry) pools: Pools,
    pub(in crate::registry) history: HistoryRestorer,
    pub(in crate::registry) limits: RegistryLimits,
    phase: AtomicU8,
}
redacted!(Admission);
impl Admission {
    pub(in crate::registry) fn new(limits: RegistryLimits, history: HistoryRestorer) -> Self {
        Self {
            pools: Pools {
                revisions: Arc::new(Semaphore::new(limits.capacities.revisions)),
                tags: Arc::new(Semaphore::new(limits.capacities.revisions)),
                records: Arc::new(Semaphore::new(limits.capacities.records)),
            },
            history,
            limits,
            phase: AtomicU8::new(OPEN),
        }
    }
    pub(in crate::registry) fn business(&self) -> Result<(), RegistryError> {
        match self
            .phase
            .compare_exchange(OPEN, SERVING, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(SERVING) => Ok(()),
            _ => Err(RegistryError::Unavailable),
        }
    }
    pub(in crate::registry) fn preparing(&self) -> bool {
        self.phase.load(Ordering::Acquire) == PREPARING
    }
}
struct Lease {
    registry: Arc<Shared>,
}
redacted!(Lease);
impl Lease {
    fn claim(registry: Arc<Shared>) -> Result<Self, RegistryError> {
        if *registry.closing.borrow()
            || !registry.restore.history.fresh
            || registry.restore.history.used() != 0
        {
            return Err(RegistryError::Unavailable);
        }
        registry
            .restore
            .phase
            .compare_exchange(OPEN, PREPARING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| RegistryError::Unavailable)?;
        Ok(Self { registry })
    }
    fn committed(&self) {
        self.registry
            .restore
            .phase
            .store(SERVING, Ordering::Release);
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _released = self.registry.restore.phase.compare_exchange(
            PREPARING,
            OPEN,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

pub(in crate::registry) struct Staged {
    pub(in crate::registry) machines: BTreeMap<String, MachineSnapshot>,
    pub(in crate::registry) records: BTreeMap<String, Entry>,
    pub(in crate::registry) names: BTreeMap<String, Retention>,
}
redacted!(Staged);

/// Fully validated, compiled and quota-charged startup replacement for one Registry.
///
/// The graph is declared before its lease so dropping it releases charges before
/// reopening startup admission. This value cannot be cloned or serialized.
///
/// ```no_run
/// # fn restore(registry:&rustack_stepfunctions_core::Registry,document:rustack_stepfunctions_core::RegistrySnapshot)->Result<(),rustack_stepfunctions_core::RegistryError>{
/// let prepared=registry.prepare_snapshot(document,Default::default())?;
/// let receipt=registry.restore_snapshot(prepared)?;
/// # Ok(()) }
/// ```
#[non_exhaustive]
pub struct PreparedSnapshot {
    graph: Staged,
    lease: Lease,
}
redacted!(PreparedSnapshot);
impl PreparedSnapshot {
    pub(in crate::registry) fn install(
        self,
        registry: &Arc<Shared>,
    ) -> Result<Staged, RegistryError> {
        if *registry.closing.borrow()
            || !Arc::ptr_eq(registry, &self.lease.registry)
            || !registry.restore.preparing()
        {
            return Err(RegistryError::Unavailable);
        }
        self.lease.committed();
        Ok(self.graph)
    }
}
impl Registry {
    /// Validate/compile a bounded archive and reserve an exclusive startup graph.
    ///
    /// Call under the embedding caller's retained bounded CPU owner. Decode the
    /// document with a bounded redacting adapter first; serde is not domain validity.
    ///
    /// # Errors
    /// Rejects non-fresh/closed ownership, malformed or incompatible data, invalid
    /// source/hash/history/references/expiry, and any copy/live quota exhaustion.
    /// Failure leaves the namespace unchanged and releases the preparation lease.
    ///
    /// ```no_run
    /// # fn prepare(registry:&rustack_stepfunctions_core::Registry,document:rustack_stepfunctions_core::RegistrySnapshot){let prepared=registry.prepare_snapshot(document,Default::default());}
    /// ```
    pub fn prepare_snapshot(
        &self,
        document: RegistrySnapshot,
        limits: SnapshotLimits,
    ) -> Result<PreparedSnapshot, RegistryError> {
        let lease = Lease::claim(self.shared.clone())?;
        let graph = validate::prepare(document, &self.shared, limits)?;
        Ok(PreparedSnapshot { graph, lease })
    }
    /// Transfer a prepared exact-instance startup replacement to the actor once.
    ///
    /// The returned receipt observes the same accepted commit after caller loss.
    ///
    /// # Errors
    /// Rejects foreign, closed or saturated admission without submitting a graph.
    /// No prepared graph can be replayed or installed after business admission.
    ///
    /// ```no_run
    /// # fn submit(registry:&rustack_stepfunctions_core::Registry,prepared:rustack_stepfunctions_core::PreparedSnapshot){let receipt=registry.restore_snapshot(prepared);}
    /// ```
    pub fn restore_snapshot(
        &self,
        prepared: PreparedSnapshot,
    ) -> Result<RegistryReceipt, RegistryError> {
        if !Arc::ptr_eq(&self.shared, &prepared.lease.registry) {
            return Err(RegistryError::Validation);
        }
        self.submit_operation(Operation::Restore(prepared))
    }
}

#[cfg(test)]
mod tests;
