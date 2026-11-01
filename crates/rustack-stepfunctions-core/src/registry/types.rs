//! Immutable shared revisions and captured high-water read views.

use std::{
    sync::Arc,
    time::{Duration, Instant as StdInstant},
};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use tokio::{sync::OwnedSemaphorePermit, time::Instant};

use super::{
    ExecutionArn, RegistryError, RoleArn, StateMachineArn, Tags, child::Lineage, domain::redacted,
};
use crate::{
    CompiledMachine, ExecutionError, ExecutionFailure, ExecutionHandle, ExecutionView, Payload,
    TaskFailure,
};

macro_rules! get {
    ($owner:ident,$name:ident,$out:ty,$value:ident=>$body:expr,$doc:literal)=>{
        #[doc=$doc]
        #[doc=concat!("\n```\n# fn inspect(value:&rustack_stepfunctions_core::",stringify!($owner),"){ let _=value.",stringify!($name),"(); }\n```")]
        #[must_use]pub fn $name(&self)->$out {let $value=self;$body}
    };
}
/// Global live/retained allocation counts, including caller-owned read snapshots.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RegistryCapacities {
    pub(super) machines: usize,
    pub(super) revisions: usize,
    pub(super) records: usize,
    pub(super) names: usize,
}
impl RegistryCapacities {
    /// Validate machine/revision1..4,096, records1..100,000 and names1..1,000,000.
    ///
    /// # Errors
    /// Rejects zero or excessive caps.
    ///
    /// ```
    /// let caps=rustack_stepfunctions_core::RegistryCapacities::new(256,1024,10000,100000)?;
    /// # Ok::<(),rustack_stepfunctions_core::RegistryError>(())
    /// ```
    pub fn new(
        machines: usize,
        revisions: usize,
        records: usize,
        names: usize,
    ) -> Result<Self, RegistryError> {
        if !(1..=4096).contains(&machines)
            || !(1..=4096).contains(&revisions)
            || !(1..=100_000).contains(&records)
            || !(1..=1_000_000).contains(&names)
        {
            return Err(RegistryError::Validation);
        }
        Ok(Self {
            machines,
            revisions,
            records,
            names,
        })
    }
}
impl Default for RegistryCapacities {
    fn default() -> Self {
        Self {
            machines: 256,
            revisions: 1024,
            records: 10_000,
            names: 100_000,
        }
    }
}
/// Validated registry allocation, retention and accepted-command ceilings.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RegistryLimits {
    pub(super) capacities: RegistryCapacities,
    pub(super) retention: Duration,
    pub(super) commands: usize,
}
impl RegistryLimits {
    /// Configure record retention60s..90days and accepted commands1..1,024.
    ///
    /// # Errors
    /// Rejects out-of-range retention or command capacity.
    ///
    /// ```
    /// let limits=rustack_stepfunctions_core::RegistryLimits::new(Default::default(),std::time::Duration::from_hours(24),128)?;
    /// # Ok::<(),rustack_stepfunctions_core::RegistryError>(())
    /// ```
    pub fn new(
        capacities: RegistryCapacities,
        retention: Duration,
        commands: usize,
    ) -> Result<Self, RegistryError> {
        if !(Duration::from_secs(60)..=Duration::from_hours(2160)).contains(&retention)
            || !(1..=1024).contains(&commands)
        {
            return Err(RegistryError::Validation);
        }
        Ok(Self {
            capacities,
            retention,
            commands,
        })
    }
}
impl Default for RegistryLimits {
    fn default() -> Self {
        Self {
            capacities: RegistryCapacities::default(),
            retention: Duration::from_hours(24),
            commands: 128,
        }
    }
}
#[derive(Clone)]
pub(super) struct RevisionData {
    pub(super) id: String,
    pub(super) machine: Arc<CompiledMachine>,
    pub(super) role: RoleArn,
    pub(super) hash: [u8; 32],
    pub(super) published: DateTime<Utc>,
}
redacted!(RevisionData);
/// Immutable revision; its admission slot stays charged through the last Arc.
#[non_exhaustive]
pub struct DefinitionRevision {
    pub(super) data: RevisionData,
    pub(super) _permit: OwnedSemaphorePermit,
}
redacted!(DefinitionRevision);
impl DefinitionRevision {
    get!(DefinitionRevision,publication_date,DateTime<Utc>,s=>s.data.published,"Read the publication date captured by this revision.");
    get!(DefinitionRevision,id,&str,s=>&s.data.id,"Read the immutable secure revision UUID.");
    get!(DefinitionRevision,machine,&Arc<CompiledMachine>,s=>&s.data.machine,"Read compiled IR and exact source bytes.");
    get!(DefinitionRevision,role,&RoleArn,s=>&s.data.role,"Read captured role metadata.");
    get!(DefinitionRevision,source_hash,&[u8;32],s=>&s.data.hash,"Read SHA-256 of the original source bytes.");
}
#[derive(Clone)]
pub(super) struct MachineIdentity {
    pub(super) arn: StateMachineArn,
    pub(super) resource: String,
    pub(super) created: DateTime<Utc>,
}
redacted!(MachineIdentity);
/// Mutable resource availability; existing execution revisions are independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MachineStatus {
    /// Starts and updates are accepted.
    Active,
    /// Accepted executions finish before mutable metadata is removed.
    Deleting,
}
pub(super) struct TagAllocation {
    pub(super) value: Tags,
    pub(super) _permit: OwnedSemaphorePermit,
}
redacted!(TagAllocation);
/// Stable resource read capture; later updates cannot change this value.
#[derive(Clone)]
#[non_exhaustive]
pub struct MachineSnapshot {
    pub(super) identity: Arc<MachineIdentity>,
    pub(super) updated: DateTime<Utc>,
    pub(super) status: MachineStatus,
    pub(super) revision: Arc<DefinitionRevision>,
    pub(super) tags: Arc<TagAllocation>,
}
redacted!(MachineSnapshot);
impl MachineSnapshot {
    get!(MachineSnapshot,arn,&StateMachineArn,s=>&s.identity.arn,"Read the scoped resource ARN.");
    get!(MachineSnapshot,resource_id,&str,s=>&s.identity.resource,"Read the unique incarnation UUID, distinct across recreation.");
    get!(MachineSnapshot,creation_date,DateTime<Utc>,s=>s.identity.created,"Read original resource creation time.");
    get!(MachineSnapshot,update_date,DateTime<Utc>,s=>s.updated,"Read this revision's publication time.");
    get!(MachineSnapshot,status,MachineStatus,s=>s.status,"Read this snapshot's lifecycle phase.");
    get!(MachineSnapshot,revision,&Arc<DefinitionRevision>,s=>&s.revision,"Read the captured immutable revision.");
    get!(MachineSnapshot,tags,&Tags,s=>&s.tags.value,"Read this snapshot's atomic tags.");
}
#[derive(Clone)]
pub(super) struct ExecutionIdentity {
    pub(super) arn: ExecutionArn,
    pub(super) machine: Arc<MachineIdentity>,
    pub(super) started: DateTime<Utc>,
    pub(super) trace: Option<String>,
}
redacted!(ExecutionIdentity);
pub(super) struct ExecutionRecord {
    pub(super) lineage: Lineage,
    pub(super) deadline: Option<StdInstant>,
    pub(super) identity: ExecutionIdentity,
    pub(super) input: Payload,
    pub(super) revision: Arc<DefinitionRevision>,
    pub(super) handle: ExecutionHandle,
    pub(super) _permit: OwnedSemaphorePermit,
}
redacted!(ExecutionRecord);
/// Captured execution identity/revision plus one atomic engine read boundary.
#[derive(Clone)]
#[non_exhaustive]
pub struct ExecutionSnapshot {
    pub(super) record: Arc<ExecutionRecord>,
    pub(super) view: Arc<ExecutionView>,
}
redacted!(ExecutionSnapshot);
impl ExecutionSnapshot {
    /// Observe termination while preserving captured identity and revision.
    ///
    /// # Errors
    /// Returns the execution observer deadline/owner error without cancellation.
    ///
    /// ```no_run
    /// # async fn inspect(snapshot:&rustack_stepfunctions_core::ExecutionSnapshot,deadline:tokio::time::Instant){let result=snapshot.wait(deadline).await;}
    /// ```
    pub async fn wait(&self, deadline: Instant) -> Result<Self, ExecutionError> {
        let mut handle = self.handle();
        let view = handle.wait(deadline).await?;
        Ok(Self {
            record: self.record.clone(),
            view,
        })
    }
    get!(ExecutionSnapshot,arn,&ExecutionArn,s=>&s.record.identity.arn,"Read the scoped execution ARN.");
    get!(ExecutionSnapshot,resource_id,&str,s=>&s.record.identity.machine.resource,"Read the captured machine incarnation UUID.");
    get!(ExecutionSnapshot,start_date,DateTime<Utc>,s=>s.record.identity.started,"Read original start acceptance time.");
    get!(ExecutionSnapshot,trace_header,Option<&str>,s=>s.record.identity.trace.as_deref(),"Read optional original trace metadata.");
    get!(ExecutionSnapshot,input,&Payload,s=>&s.record.input,"Read parsed input and its exact original bytes.");
    get!(ExecutionSnapshot,revision,&Arc<DefinitionRevision>,s=>&s.record.revision,"Read the revision captured at Start, even after deletion.");
    get!(ExecutionSnapshot,view,&Arc<ExecutionView>,s=>&s.view,"Read the immutable atomic history/outcome capture.");
    get!(ExecutionSnapshot,handle,ExecutionHandle,s=>s.record.handle.clone(),"Create an independent observer/control handle; dropping it does not cancel.");
}

/// Precomputed source fingerprint and compiled IR, prepared under caller CPU admission.
#[derive(Clone)]
#[non_exhaustive]
pub struct PreparedDefinition {
    pub(super) machine: Arc<CompiledMachine>,
    pub(super) hash: [u8; 32],
}
redacted!(PreparedDefinition);
impl PreparedDefinition {
    /// Fingerprint an already validated bounded definition.
    ///
    /// Registry admission rejects nested scope artifacts; their source bytes
    /// identify the enclosing root definition, not a standalone revision.
    ///
    /// Async callers run compilation and this synchronous preparation under their
    /// retained bounded CPU owner before submitting a registry command.
    ///
    /// ```
    /// let compiled=rustack_stepfunctions_core::AslCompiler::default().compile(br#"{"StartAt":"D","States":{"D":{"Type":"Succeed"}}}"#)?;
    /// let prepared=rustack_stepfunctions_core::PreparedDefinition::new(compiled.machine().clone());
    /// # Ok::<(),rustack_stepfunctions_core::CompileError>(())
    /// ```
    #[must_use]
    pub fn new(machine: Arc<CompiledMachine>) -> Self {
        let hash = Sha256::digest(machine.original_bytes()).into();
        Self { machine, hash }
    }
}

/// Validated external Stop fields, retaining omission separately from empty strings.
#[derive(Clone)]
#[non_exhaustive]
pub struct StopReason(pub(super) TaskFailure);
redacted!(StopReason);
impl StopReason {
    /// Validate optional Stop error/cause without interpreting them as matchers.
    ///
    /// # Errors
    /// Rejects error above256 bytes or cause above32,768 bytes.
    ///
    /// ```
    /// let reason=rustack_stepfunctions_core::StopReason::new(None,Some(String::new()))?;
    /// # Ok::<(),rustack_stepfunctions_core::RegistryError>(())
    /// ```
    pub fn new(error: Option<String>, cause: Option<String>) -> Result<Self, RegistryError> {
        if error.as_ref().is_some_and(|v| v.len() > 256)
            || cause.as_ref().is_some_and(|v| v.len() > 32768)
        {
            return Err(RegistryError::Validation);
        }
        Ok(Self(TaskFailure(ExecutionFailure::fields(error, cause))))
    }
}
