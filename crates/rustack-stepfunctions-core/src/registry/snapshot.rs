//! Data-only export of one immutable post-cleanup registry cut.

use std::{collections::BTreeMap, fmt, str, sync::Arc};

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use super::{
    DefinitionRevision, MachineSnapshot, MachineStatus, RegistryError, StateSupport,
    domain::redacted,
    retention::Retention,
    types::{ExecutionRecord, MachineIdentity},
};
use crate::{
    ClockSample, ExecutionFailure, ExecutionHistory, ExecutionOutcome, ExecutionView, TenantScope,
};

mod decode;
mod restore;

pub(super) use restore::Admission as RestoreAdmission;
pub use restore::PreparedSnapshot;

const METADATA_BYTES: usize = 4096;
const NAME_BYTES: usize = 512;

/// Validated conservative source-copy budget for one registry snapshot export.
///
/// The embedding encoder separately caps actual serialized bytes. The allowance
/// is1byte..=1GiB; the default is512MiB. Export fails before copying payloads when
/// the conservative estimate exceeds it.
///
/// ```
/// let limits=rustack_stepfunctions_core::SnapshotLimits::try_from(8*1024*1024)?;
/// # Ok::<(),rustack_stepfunctions_core::RegistryError>(())
/// ```
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct SnapshotLimits {
    bytes: usize,
}
impl TryFrom<usize> for SnapshotLimits {
    type Error = RegistryError;
    fn try_from(bytes: usize) -> Result<Self, Self::Error> {
        if !(1..=1024 * 1024 * 1024).contains(&bytes) {
            return Err(RegistryError::Validation);
        }
        Ok(Self { bytes })
    }
}
impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            bytes: 512 * 1024 * 1024,
        }
    }
}

/// Versioned durable document for sealed export and bounded startup decoding.
///
/// Deserialization does not grant domain validity: use `Registry::prepare_snapshot`
/// before committing it. Decode with a bounded redacting adapter. Intentional
/// definition/input/output/history payloads are available only through serialization;
/// Debug is redacted and no live handles or credentials are stored.
///
/// ```no_run
/// # fn export(registry:&rustack_stepfunctions_core::Registry)->Result<(),Box<dyn std::error::Error>> {
/// let snapshot=registry.export_snapshot(Default::default())?;
/// let bytes=serde_json::to_vec(&snapshot)?;
/// # Ok(()) }
/// ```
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct RegistrySnapshot {
    schema_version: u32,
    profile_version: u32,
    scope: ScopeRow,
    state_support: ProfileStates,
    lambda_support: bool,
    callback_support: bool,
    #[serde(deserialize_with = "decode::time")]
    captured_at: DateTime<Utc>,
    #[serde(deserialize_with = "decode::rows::<_, _, 4096>")]
    resources: Vec<ResourceRow>,
    #[serde(deserialize_with = "decode::rows::<_, _, 4096>")]
    revisions: Vec<RevisionRow>,
    #[serde(deserialize_with = "decode::rows::<_, _, 4096>")]
    machines: Vec<MachineRow>,
    #[serde(deserialize_with = "decode::rows::<_, _, 100000>")]
    executions: Vec<ExecutionRow>,
    #[serde(deserialize_with = "decode::rows::<_, _, 1000000>")]
    names: Vec<NameRow>,
}
redacted!(RegistrySnapshot);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum ProfileStates {
    Sequential,
    Structured,
}
impl From<StateSupport> for ProfileStates {
    fn from(value: StateSupport) -> Self {
        match value {
            StateSupport::Sequential => Self::Sequential,
            StateSupport::Structured => Self::Structured,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Availability {
    Active,
    Deleting,
}
impl From<MachineStatus> for Availability {
    fn from(value: MachineStatus) -> Self {
        match value {
            MachineStatus::Active => Self::Active,
            MachineStatus::Deleting => Self::Deleting,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryRow {
    #[serde(deserialize_with = "decode::time")]
    timestamp: DateTime<Utc>,
    #[serde(deserialize_with = "decode::text::<_, 2097152>")]
    event_json: String,
}
redacted!(HistoryRow);

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScopeRow {
    #[serde(deserialize_with = "decode::text::<_, 16>")]
    partition: String,
    #[serde(deserialize_with = "decode::text::<_, 64>")]
    region: String,
    #[serde(deserialize_with = "decode::text::<_, 12>")]
    account: String,
}
redacted!(ScopeRow);
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceRow {
    #[serde(deserialize_with = "decode::text::<_, 36>")]
    resource_id: String,
    #[serde(deserialize_with = "decode::text::<_, 256>")]
    arn: String,
    #[serde(deserialize_with = "decode::time")]
    created_at: DateTime<Utc>,
}
redacted!(ResourceRow);
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RevisionRow {
    #[serde(deserialize_with = "decode::text::<_, 36>")]
    revision_id: String,
    #[serde(deserialize_with = "decode::text::<_, 36>")]
    resource_id: String,
    #[serde(deserialize_with = "decode::text::<_, 1048576>")]
    definition: String,
    source_hash: [u8; 32],
    #[serde(deserialize_with = "decode::text::<_, 2048>")]
    role_arn: String,
    #[serde(deserialize_with = "decode::time")]
    published_at: DateTime<Utc>,
}
redacted!(RevisionRow);
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MachineRow {
    #[serde(deserialize_with = "decode::text::<_, 36>")]
    resource_id: String,
    #[serde(deserialize_with = "decode::text::<_, 36>")]
    revision_id: String,
    status: Availability,
    #[serde(deserialize_with = "decode::time")]
    updated_at: DateTime<Utc>,
    #[serde(deserialize_with = "decode::tags")]
    tags: BTreeMap<String, String>,
}
redacted!(MachineRow);
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExecutionRow {
    #[serde(deserialize_with = "decode::text::<_, 256>")]
    arn: String,
    #[serde(deserialize_with = "decode::text::<_, 36>")]
    resource_id: String,
    #[serde(deserialize_with = "decode::text::<_, 36>")]
    revision_id: String,
    #[serde(deserialize_with = "decode::text::<_, 262144>")]
    input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "decode::optional_text::<_, 256>")]
    trace_header: Option<String>,
    #[serde(deserialize_with = "decode::time")]
    started_at: DateTime<Utc>,
    #[serde(
        default,
        deserialize_with = "decode::optional_time",
        skip_serializing_if = "Option::is_none"
    )]
    stopped_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        deserialize_with = "decode::optional_time",
        skip_serializing_if = "Option::is_none"
    )]
    expires_at: Option<DateTime<Utc>>,
    outcome: Outcome,
    #[serde(deserialize_with = "decode::rows::<_, _, 25000>")]
    history: Vec<HistoryRow>,
}
redacted!(ExecutionRow);
#[derive(Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    tag = "status",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
enum Outcome {
    Running {},
    Succeeded {
        #[serde(deserialize_with = "decode::text::<_, 262144>")]
        output: String,
    },
    Failed {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default, deserialize_with = "decode::optional_text::<_, 256>")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default, deserialize_with = "decode::optional_text::<_, 32768>")]
        cause: Option<String>,
    },
    TimedOut {},
    Aborted {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default, deserialize_with = "decode::optional_text::<_, 256>")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default, deserialize_with = "decode::optional_text::<_, 32768>")]
        cause: Option<String>,
    },
}
redacted!(Outcome);
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NameRow {
    #[serde(deserialize_with = "decode::text::<_, 256>")]
    arn: String,
    #[serde(
        default,
        deserialize_with = "decode::optional_time",
        skip_serializing_if = "Option::is_none"
    )]
    expires_at: Option<DateTime<Utc>>,
}
redacted!(NameRow);

pub(super) struct CapturedExecution {
    pub(super) record: Arc<ExecutionRecord>,
    pub(super) view: Arc<ExecutionView>,
    pub(super) retention: Retention,
}
redacted!(CapturedExecution);
/// In-memory ownership only; serialized DTOs never contain these handles.
pub(super) struct SealedRegistry {
    pub(super) scope: TenantScope,
    pub(super) states: StateSupport,
    pub(super) lambda: bool,
    pub(super) callbacks: bool,
    pub(super) clock: ClockSample,
    pub(super) machines: BTreeMap<String, MachineSnapshot>,
    pub(super) executions: Vec<CapturedExecution>,
    pub(super) names: BTreeMap<String, Retention>,
}
redacted!(SealedRegistry);

struct Budget {
    remaining: usize,
}
redacted!(Budget);
impl Budget {
    fn charge(&mut self, bytes: usize) -> Result<(), RegistryError> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or(RegistryError::Capacity)?;
        Ok(())
    }
    fn failure(&mut self, failure: &ExecutionFailure) -> Result<(), RegistryError> {
        self.charge(failure.error().map_or(0, str::len))?;
        self.charge(failure.cause().map_or(0, str::len))
    }
}
struct Plan<'a> {
    resources: BTreeMap<&'a str, &'a Arc<MachineIdentity>>,
    revisions: BTreeMap<&'a str, (&'a str, &'a Arc<DefinitionRevision>)>,
    budget: Budget,
}
impl fmt::Debug for Plan<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SnapshotPlan(<redacted>)")
    }
}
impl<'a> Plan<'a> {
    fn identity(&mut self, identity: &'a Arc<MachineIdentity>) -> Result<(), RegistryError> {
        if let Some(existing) = self.resources.get(identity.resource.as_str()) {
            if !Arc::ptr_eq(existing, identity) {
                return Err(RegistryError::Validation);
            }
            return Ok(());
        }
        self.budget.charge(METADATA_BYTES)?;
        self.resources.insert(&identity.resource, identity);
        Ok(())
    }
    fn revision(
        &mut self,
        resource: &'a str,
        revision: &'a Arc<DefinitionRevision>,
    ) -> Result<(), RegistryError> {
        if let Some((identity, existing)) = self.revisions.get(revision.id()) {
            if *identity != resource || !Arc::ptr_eq(existing, revision) {
                return Err(RegistryError::Validation);
            }
            return Ok(());
        }
        self.budget.charge(METADATA_BYTES)?;
        self.budget
            .charge(revision.machine().original_bytes().len())?;
        self.budget.charge(revision.role().as_str().len())?;
        self.revisions.insert(revision.id(), (resource, revision));
        Ok(())
    }
    fn new(source: &'a SealedRegistry, limits: SnapshotLimits) -> Result<Self, RegistryError> {
        let mut plan = Self {
            resources: BTreeMap::new(),
            revisions: BTreeMap::new(),
            budget: Budget {
                remaining: limits.bytes,
            },
        };
        plan.budget.charge(METADATA_BYTES)?;
        plan.budget.charge(source.scope.partition().len())?;
        plan.budget.charge(source.scope.region().len())?;
        plan.budget.charge(source.scope.account().len())?;
        for machine in source.machines.values() {
            plan.identity(&machine.identity)?;
            plan.revision(machine.resource_id(), &machine.revision)?;
            plan.budget.charge(METADATA_BYTES)?;
            for (key, value) in machine.tags().values() {
                plan.budget.charge(key.len())?;
                plan.budget.charge(value.len())?;
            }
        }
        for execution in &source.executions {
            let record = &execution.record;
            plan.identity(&record.identity.machine)?;
            plan.revision(&record.identity.machine.resource, &record.revision)?;
            plan.budget.charge(METADATA_BYTES)?;
            plan.budget.charge(record.input.original_bytes().len())?;
            plan.budget
                .charge(record.identity.trace.as_ref().map_or(0, String::len))?;
            plan.budget
                .charge(execution.view.history().charged_bytes())?;
            match execution.view.outcome().map(Arc::as_ref) {
                Some(ExecutionOutcome::Succeeded(output)) => {
                    plan.budget.charge(output.original_bytes().len())?;
                }
                Some(ExecutionOutcome::Failed(failure)) => plan.budget.failure(failure)?,
                Some(ExecutionOutcome::Aborted(failure)) => {
                    plan.budget.failure(failure.details())?;
                }
                Some(ExecutionOutcome::TimedOut) => {}
                None => return Err(RegistryError::Unavailable),
            }
        }
        plan.budget.charge(
            source
                .names
                .len()
                .checked_mul(NAME_BYTES)
                .ok_or(RegistryError::Capacity)?,
        )?;
        Ok(plan)
    }
}
fn text(bytes: &[u8]) -> Result<String, RegistryError> {
    str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| RegistryError::Validation)
}
fn outcome(view: &ExecutionView) -> Result<Outcome, RegistryError> {
    Ok(
        match view
            .outcome()
            .map(Arc::as_ref)
            .ok_or(RegistryError::Unavailable)?
        {
            ExecutionOutcome::Succeeded(output) => Outcome::Succeeded {
                output: text(output.original_bytes())?,
            },
            ExecutionOutcome::Failed(failure) => Outcome::Failed {
                error: failure.error().map(str::to_owned),
                cause: failure.cause().map(str::to_owned),
            },
            ExecutionOutcome::TimedOut => Outcome::TimedOut {},
            ExecutionOutcome::Aborted(failure) => Outcome::Aborted {
                error: failure.details().error().map(str::to_owned),
                cause: failure.details().cause().map(str::to_owned),
            },
        },
    )
}
fn history(source: &ExecutionHistory) -> Result<Vec<HistoryRow>, RegistryError> {
    let count = usize::try_from(source.len()).map_err(|_| RegistryError::Validation)?;
    let mut history = Vec::new();
    history
        .try_reserve_exact(count)
        .map_err(|_| RegistryError::Capacity)?;
    for event in source.snapshot_events()? {
        history.push(HistoryRow {
            timestamp: event.timestamp,
            event_json: serde_json::to_string(event).map_err(|_| RegistryError::Validation)?,
        });
    }
    Ok(history)
}
impl SealedRegistry {
    fn expiry(&self, retention: Retention) -> Result<DateTime<Utc>, RegistryError> {
        let remaining =
            TimeDelta::from_std(retention.remaining(Instant::from_std(self.clock.monotonic))?)
                .map_err(|_| RegistryError::Validation)?;
        self.clock
            .wall
            .checked_add_signed(remaining)
            .ok_or(RegistryError::Validation)
    }
    pub(super) fn export(&self, limits: SnapshotLimits) -> Result<RegistrySnapshot, RegistryError> {
        let plan = Plan::new(self, limits)?;
        let resources = plan
            .resources
            .values()
            .map(|identity| ResourceRow {
                resource_id: identity.resource.clone(),
                arn: identity.arn.as_str().into(),
                created_at: identity.created,
            })
            .collect();
        let revisions = plan
            .revisions
            .values()
            .map(|(resource, revision)| {
                Ok(RevisionRow {
                    revision_id: revision.id().into(),
                    resource_id: (*resource).into(),
                    definition: text(revision.machine().original_bytes())?,
                    source_hash: *revision.source_hash(),
                    role_arn: revision.role().as_str().into(),
                    published_at: revision.publication_date(),
                })
            })
            .collect::<Result<_, RegistryError>>()?;
        let machines = self
            .machines
            .values()
            .map(|machine| MachineRow {
                resource_id: machine.resource_id().into(),
                revision_id: machine.revision().id().into(),
                status: machine.status().into(),
                updated_at: machine.update_date(),
                tags: machine.tags().values().clone(),
            })
            .collect();
        let executions = self
            .executions
            .iter()
            .map(|execution| {
                let record = &execution.record;
                Ok(ExecutionRow {
                    arn: record.identity.arn.as_str().into(),
                    resource_id: record.identity.machine.resource.clone(),
                    revision_id: record.revision.id().into(),
                    input: text(record.input.original_bytes())?,
                    trace_header: record.identity.trace.clone(),
                    started_at: record.identity.started,
                    stopped_at: Some(
                        execution
                            .view
                            .stop_date()
                            .ok_or(RegistryError::Unavailable)?,
                    ),
                    expires_at: Some(self.expiry(execution.retention)?),
                    outcome: outcome(&execution.view)?,
                    history: history(execution.view.history())?,
                })
            })
            .collect::<Result<_, RegistryError>>()?;
        let names = self
            .names
            .iter()
            .map(|(arn, closed)| {
                Ok(NameRow {
                    arn: arn.clone(),
                    expires_at: Some(self.expiry(*closed)?),
                })
            })
            .collect::<Result<_, RegistryError>>()?;
        Ok(RegistrySnapshot {
            schema_version: 1,
            profile_version: 1,
            scope: ScopeRow {
                partition: self.scope.partition().into(),
                region: self.scope.region().into(),
                account: self.scope.account().into(),
            },
            state_support: self.states.into(),
            lambda_support: self.lambda,
            callback_support: self.callbacks,
            captured_at: self.clock.wall,
            resources,
            revisions,
            machines,
            executions,
            names,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, error::Error, sync::Arc, time::Duration};

    use chrono::DateTime;
    use rstest::rstest;
    use rustack_stepfunctions_model::types::{HistoryEvent, HistoryEventType};
    use tokio::time::Instant;

    use super::{Budget, HistoryRow, Plan};
    use crate::registry::{
        RegistryError, RegistryLimits,
        tests::{machine, registry},
        types::MachineIdentity,
    };

    #[tokio::test]
    async fn test_should_reject_inconsistent_identity_and_revision_bindings_before_copying()
    -> Result<(), Box<dyn Error>> {
        let (registry, mut owner) = registry(RegistryLimits::default(), None);
        let machine = machine(&registry, 42, false).await;
        let mut plan = Plan {
            resources: BTreeMap::new(),
            revisions: BTreeMap::new(),
            budget: Budget {
                remaining: 1024 * 1024,
            },
        };
        plan.identity(&machine.identity)?;
        let remaining = plan.budget.remaining;
        plan.identity(&machine.identity)?;
        assert_eq!(remaining, plan.budget.remaining);
        let collision = Arc::new(MachineIdentity {
            arn: machine.arn().clone(),
            resource: machine.resource_id().into(),
            created: machine.creation_date(),
        });
        assert!(matches!(
            plan.identity(&collision),
            Err(RegistryError::Validation)
        ));
        plan.revision(machine.resource_id(), machine.revision())?;
        assert!(matches!(
            plan.revision("different-resource", machine.revision()),
            Err(RegistryError::Validation)
        ));
        owner
            .shutdown(Instant::now() + Duration::from_secs(5))
            .await?;
        Ok(())
    }

    #[rstest]
    #[case(123, 123_456_789)]
    #[case(-1,123_456_789)]
    fn test_should_preserve_nanosecond_history_through_real_cbor(
        #[case] seconds: i64,
        #[case] nanos: u32,
    ) -> Result<(), Box<dyn Error>> {
        let timestamp = DateTime::from_timestamp(seconds, nanos).ok_or("invalid test timestamp")?;
        let event = HistoryEvent::builder()
            .id(7)
            .previous_event_id(Some(3))
            .timestamp(timestamp)
            .r#type(HistoryEventType::TaskStateEntered)
            .build();
        let mut direct = Vec::new();
        ciborium::into_writer(&event, &mut direct)?;
        assert!(ciborium::from_reader::<HistoryEvent, _>(direct.as_slice()).is_err());
        let json = serde_json::to_string(&event)?;
        let public: HistoryEvent = serde_json::from_str(&json)?;
        assert_ne!(public.timestamp, timestamp);
        let wrapper = HistoryRow {
            timestamp,
            event_json: json,
        };
        let mut bytes = Vec::new();
        ciborium::into_writer(&wrapper, &mut bytes)?;
        let decoded: HistoryRow = ciborium::from_reader(bytes.as_slice())?;
        assert_eq!(decoded.timestamp, timestamp);
        let mut restored: HistoryEvent = serde_json::from_str(&decoded.event_json)?;
        restored.timestamp = decoded.timestamp;
        assert_eq!(restored.timestamp, timestamp);
        assert_eq!(restored.id, event.id);
        assert_eq!(restored.previous_event_id, event.previous_event_id);
        assert_eq!(
            serde_json::to_string(&restored)?,
            serde_json::to_string(&event)?
        );
        Ok(())
    }
}
