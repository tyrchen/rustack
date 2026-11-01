//! Complete archive validation and pure quota-reserved replacement construction.

use std::{
    collections::{BTreeMap, BTreeSet},
    str,
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, TimeDelta, Utc};
use rustack_stepfunctions_model::types::HistoryEvent;
use tokio::time::Instant;
use uuid::{Uuid, Variant};

use super::{
    super::{
        super::{
            DefinitionRevision, ExecutionArn, MachineSnapshot, MachineStatus, PreparedDefinition,
            RegistryError, RoleArn, Shared, StateMachineArn, Tags,
            actor::Entry,
            child::Lineage,
            domain::{redacted, same_scope},
            retention::Retention,
            types::{
                ExecutionIdentity, ExecutionRecord, MachineIdentity, RevisionData, TagAllocation,
            },
        },
        Availability, Budget, ExecutionRow, METADATA_BYTES, MachineRow, NAME_BYTES, NameRow,
        Outcome, ProfileStates, RegistrySnapshot, ResourceRow, RevisionRow, SnapshotLimits,
    },
    Staged,
    history::{Profile as HistoryProfile, validate_rows},
};
use crate::{
    AslCompiler, ClockSample, ExecutionFailure, ExecutionOutcome, Payload, StateSupport,
    TaskFailure, TaskSupport, TenantScope, engine::HistoryRestorer,
};

const NAME_RETENTION: Duration = Duration::from_hours(2160);
struct Context {
    scope: TenantScope,
    states: ProfileStates,
    lambda: bool,
    callbacks: bool,
    captured: DateTime<Utc>,
}
redacted!(Context);
struct RevisionInput {
    resource: String,
    definition: PreparedDefinition,
    role: RoleArn,
    published: DateTime<Utc>,
}
redacted!(RevisionInput);
struct MachineInput {
    resource: String,
    revision: String,
    updated: DateTime<Utc>,
    tags: Tags,
    active: bool,
}
redacted!(MachineInput);
struct ExecutionInput {
    identity: ExecutionIdentity,
    revision: String,
    input: Payload,
    stopped: Option<DateTime<Utc>>,
    expires: Option<DateTime<Utc>>,
    outcome: Option<ExecutionOutcome>,
    history: Vec<HistoryEvent>,
}
redacted!(ExecutionInput);
struct Validated {
    context: Context,
    resources: BTreeMap<String, Arc<MachineIdentity>>,
    revisions: BTreeMap<String, RevisionInput>,
    machines: Vec<MachineInput>,
    executions: Vec<ExecutionInput>,
    names: BTreeMap<String, Option<DateTime<Utc>>>,
}
redacted!(Validated);
fn time(value: DateTime<Utc>) -> Result<(), RegistryError> {
    if value.timestamp_subsec_nanos() >= 1_000_000_000 {
        return Err(RegistryError::Validation);
    }
    Ok(())
}
fn expiry(value: DateTime<Utc>, captured: DateTime<Utc>) -> Result<(), RegistryError> {
    time(value)?;
    let maximum = captured
        .checked_add_signed(TimeDelta::hours(2160))
        .ok_or(RegistryError::Validation)?;
    if value <= captured || value > maximum {
        return Err(RegistryError::Validation);
    }
    Ok(())
}
fn identifier(value: &str) -> Result<(), RegistryError> {
    let parsed = Uuid::parse_str(value).map_err(|_| RegistryError::Validation)?;
    if parsed.get_version_num() != 4
        || parsed.get_variant() != Variant::RFC4122
        || parsed.to_string() != value
    {
        return Err(RegistryError::Validation);
    }
    Ok(())
}
fn budget(document: &RegistrySnapshot, limits: SnapshotLimits) -> Result<(), RegistryError> {
    let mut budget = Budget {
        remaining: limits.bytes,
    };
    budget.charge(METADATA_BYTES)?;
    budget.charge(document.scope.partition.len())?;
    budget.charge(document.scope.region.len())?;
    budget.charge(document.scope.account.len())?;
    for count in [
        document.resources.len(),
        document.revisions.len(),
        document.machines.len(),
        document.executions.len(),
    ] {
        budget.charge(
            count
                .checked_mul(METADATA_BYTES)
                .ok_or(RegistryError::Capacity)?,
        )?;
    }
    budget.charge(
        document
            .names
            .len()
            .checked_mul(NAME_BYTES)
            .ok_or(RegistryError::Capacity)?,
    )?;
    for revision in &document.revisions {
        budget.charge(revision.definition.len())?;
        budget.charge(revision.role_arn.len())?;
    }
    for machine in &document.machines {
        for (key, value) in &machine.tags {
            budget.charge(key.len())?;
            budget.charge(value.len())?;
        }
    }
    for execution in &document.executions {
        budget.charge(execution.input.len())?;
        budget.charge(execution.trace_header.as_ref().map_or(0, String::len))?;
        match &execution.outcome {
            Outcome::Succeeded { output } => budget.charge(output.len())?,
            Outcome::Failed { error, cause } | Outcome::Aborted { error, cause } => {
                budget.charge(error.as_ref().map_or(0, String::len))?;
                budget.charge(cause.as_ref().map_or(0, String::len))?;
            }
            Outcome::Running {} | Outcome::TimedOut {} => {}
        }
        budget.charge(HistoryRestorer::estimate(
            execution.history.iter().map(|event| event.event_json.len()),
            !matches!(execution.outcome, Outcome::Running {}),
        )?)?;
    }
    Ok(())
}
impl Context {
    fn new(document: &RegistrySnapshot, shared: &Shared) -> Result<Self, RegistryError> {
        if document.schema_version != 1
            || document.profile_version != 1
            || (document.callback_support && !document.lambda_support)
        {
            return Err(RegistryError::Validation);
        }
        let scope = TenantScope::new(
            &document.scope.partition,
            &document.scope.region,
            &document.scope.account,
        )?;
        if !same_scope(&scope, &shared.scope) {
            return Err(RegistryError::Validation);
        }
        time(document.captured_at)?;
        if document.captured_at > Utc::now() {
            return Err(RegistryError::Validation);
        }
        Ok(Self {
            scope,
            states: document.state_support,
            lambda: document.lambda_support,
            callbacks: document.callback_support,
            captured: document.captured_at,
        })
    }
    fn compatible(&self, shared: &Shared) -> Result<(), RegistryError> {
        if (matches!(self.states, ProfileStates::Structured)
            && shared.states != StateSupport::Structured)
            || (self.lambda && !shared.restore.history.lambda)
            || (self.callbacks && shared.callbacks.is_none())
        {
            return Err(RegistryError::Validation);
        }
        Ok(())
    }
    fn compiler(&self) -> AslCompiler {
        AslCompiler::new(
            self.scope.clone(),
            if self.lambda {
                TaskSupport::Lambda
            } else {
                TaskSupport::Disabled
            },
        )
        .with_state_support(match self.states {
            ProfileStates::Sequential => StateSupport::Sequential,
            ProfileStates::Structured => StateSupport::Structured,
        })
        .with_child_support(true)
        .with_callback_support(self.callbacks)
    }
}
fn resources(
    rows: Vec<ResourceRow>,
    context: &Context,
) -> Result<BTreeMap<String, Arc<MachineIdentity>>, RegistryError> {
    let mut resources = BTreeMap::new();
    for row in rows {
        identifier(&row.resource_id)?;
        time(row.created_at)?;
        let arn: StateMachineArn = row.arn.parse()?;
        arn.check(&context.scope)?;
        let identity = Arc::new(MachineIdentity {
            arn,
            resource: row.resource_id.clone(),
            created: row.created_at,
        });
        if resources.insert(row.resource_id, identity).is_some() {
            return Err(RegistryError::Validation);
        }
    }
    Ok(resources)
}
fn revisions(
    rows: Vec<RevisionRow>,
    context: &Context,
    resources: &BTreeMap<String, Arc<MachineIdentity>>,
) -> Result<BTreeMap<String, RevisionInput>, RegistryError> {
    let compiler = context.compiler();
    let mut revisions = BTreeMap::new();
    for row in rows {
        identifier(&row.revision_id)?;
        time(row.published_at)?;
        if !resources.contains_key(&row.resource_id) {
            return Err(RegistryError::Validation);
        }
        let role: RoleArn = row.role_arn.parse()?;
        role.check(&context.scope)?;
        let definition = PreparedDefinition::new(
            compiler
                .compile(row.definition.as_bytes())
                .map_err(|_| RegistryError::Validation)?
                .machine()
                .clone(),
        );
        if definition.hash != row.source_hash {
            return Err(RegistryError::Validation);
        }
        if revisions
            .insert(
                row.revision_id,
                RevisionInput {
                    resource: row.resource_id,
                    definition,
                    role,
                    published: row.published_at,
                },
            )
            .is_some()
        {
            return Err(RegistryError::Validation);
        }
    }
    Ok(revisions)
}
fn names(
    rows: Vec<NameRow>,
    context: &Context,
) -> Result<BTreeMap<String, Option<DateTime<Utc>>>, RegistryError> {
    let mut names = BTreeMap::new();
    for row in rows {
        let arn: ExecutionArn = row.arn.parse()?;
        arn.machine().check(&context.scope)?;
        if let Some(value) = row.expires_at {
            expiry(value, context.captured)?;
        }
        if names.insert(arn.as_str().into(), row.expires_at).is_some() {
            return Err(RegistryError::Validation);
        }
    }
    Ok(names)
}
fn machines(
    rows: Vec<MachineRow>,
    resources: &BTreeMap<String, Arc<MachineIdentity>>,
    revisions: &BTreeMap<String, RevisionInput>,
) -> Result<Vec<MachineInput>, RegistryError> {
    let mut machines = Vec::new();
    let mut arns = BTreeSet::new();
    for row in rows {
        time(row.updated_at)?;
        let resource = resources
            .get(&row.resource_id)
            .ok_or(RegistryError::Validation)?;
        let revision = revisions
            .get(&row.revision_id)
            .ok_or(RegistryError::Validation)?;
        if revision.resource != row.resource_id
            || revision.published != row.updated_at
            || !arns.insert(resource.arn.as_str())
        {
            return Err(RegistryError::Validation);
        }
        let tags = if row.tags.is_empty() {
            Tags::default()
        } else {
            Tags::new(row.tags.into_iter().collect())?
        };
        machines.push(MachineInput {
            resource: row.resource_id,
            revision: row.revision_id,
            updated: row.updated_at,
            tags,
            active: matches!(row.status, Availability::Active),
        });
    }
    Ok(machines)
}
fn outcome(value: Outcome) -> Result<Option<ExecutionOutcome>, RegistryError> {
    Ok(match value {
        Outcome::Running {} => None,
        Outcome::Succeeded { output } => Some(ExecutionOutcome::Succeeded(Payload::parse(
            output.as_bytes(),
        )?)),
        Outcome::Failed { error, cause } => Some(ExecutionOutcome::Failed(
            ExecutionFailure::fields(error, cause),
        )),
        Outcome::Aborted { error, cause } => Some(ExecutionOutcome::Aborted(TaskFailure(
            ExecutionFailure::fields(error, cause),
        ))),
        Outcome::TimedOut {} => Some(ExecutionOutcome::TimedOut),
    })
}
fn executions(
    rows: Vec<ExecutionRow>,
    context: &Context,
    resources: &BTreeMap<String, Arc<MachineIdentity>>,
    revisions: &BTreeMap<String, RevisionInput>,
    names: &BTreeMap<String, Option<DateTime<Utc>>>,
) -> Result<Vec<ExecutionInput>, RegistryError> {
    let profile = HistoryProfile {
        states: context.states,
        lambda: context.lambda,
        callbacks: context.callbacks,
        scope: &context.scope,
    };
    let mut executions = Vec::new();
    let mut arns = BTreeSet::new();
    let mut pending_names = BTreeSet::new();
    for row in rows {
        time(row.started_at)?;
        if let Some(stopped) = row.stopped_at {
            time(stopped)?;
        }
        let resource = resources
            .get(&row.resource_id)
            .ok_or(RegistryError::Validation)?;
        let revision = revisions
            .get(&row.revision_id)
            .ok_or(RegistryError::Validation)?;
        let arn: ExecutionArn = row.arn.parse()?;
        if revision.resource != row.resource_id
            || arn.machine().as_str() != resource.arn.as_str()
            || !arns.insert(arn.as_str().to_owned())
        {
            return Err(RegistryError::Validation);
        }
        let name = names.get(arn.as_str()).ok_or(RegistryError::Validation)?;
        let result = outcome(row.outcome)?;
        match (result.is_some(), row.stopped_at, row.expires_at, *name) {
            (true, Some(_), Some(record), Some(name)) => {
                expiry(record, context.captured)?;
                if record > name {
                    return Err(RegistryError::Validation);
                }
            }
            (false, None, None, None) => {
                pending_names.insert(arn.as_str().to_owned());
            }
            _ => return Err(RegistryError::Validation),
        }
        let input = Payload::parse(row.input.as_bytes())?;
        if row
            .trace_header
            .as_ref()
            .is_some_and(|value| !value.is_ascii())
        {
            return Err(RegistryError::Validation);
        }
        let history = validate_rows(
            row.history,
            &row.input,
            result.as_ref(),
            row.stopped_at,
            &profile,
        )?;
        if history
            .first()
            .and_then(|event| event.execution_started_event_details.as_ref())
            .and_then(|details| details.role_arn.as_deref())
            .is_some_and(|role| role != revision.role.as_str())
        {
            return Err(RegistryError::Validation);
        }
        executions.push(ExecutionInput {
            identity: ExecutionIdentity {
                arn,
                machine: resource.clone(),
                started: row.started_at,
                trace: row.trace_header,
            },
            revision: row.revision_id,
            input,
            stopped: row.stopped_at,
            expires: row.expires_at,
            outcome: result,
            history,
        });
    }
    if names
        .iter()
        .any(|(arn, expiry)| expiry.is_none() && !pending_names.contains(arn))
    {
        return Err(RegistryError::Validation);
    }
    Ok(executions)
}
fn references(data: &Validated) -> Result<(), RegistryError> {
    let mut resources = BTreeSet::new();
    let mut revisions = BTreeSet::new();
    for machine in &data.machines {
        resources.insert(machine.resource.as_str());
        revisions.insert(machine.revision.as_str());
    }
    for execution in &data.executions {
        resources.insert(execution.identity.machine.resource.as_str());
        revisions.insert(execution.revision.as_str());
    }
    if resources.len() != data.resources.len() || revisions.len() != data.revisions.len() {
        return Err(RegistryError::Validation);
    }
    Ok(())
}
fn until(value: DateTime<Utc>, clock: ClockSample) -> Result<Retention, RegistryError> {
    let remaining = value
        .signed_duration_since(clock.wall)
        .to_std()
        .map_err(|_| RegistryError::Validation)?;
    Ok(Retention::Until(
        Instant::from_std(clock.monotonic)
            .checked_add(remaining)
            .ok_or(RegistryError::Validation)?,
    ))
}
fn after(value: Duration, clock: ClockSample) -> Result<Retention, RegistryError> {
    Ok(Retention::Until(
        Instant::from_std(clock.monotonic)
            .checked_add(value)
            .ok_or(RegistryError::Validation)?,
    ))
}
fn stage_revisions(
    rows: BTreeMap<String, RevisionInput>,
    needed: &BTreeSet<&str>,
    shared: &Shared,
) -> Result<BTreeMap<String, Arc<DefinitionRevision>>, RegistryError> {
    let mut revisions = BTreeMap::new();
    for (id, row) in rows {
        if needed.contains(id.as_str()) {
            let permit = shared
                .restore
                .pools
                .revisions
                .clone()
                .try_acquire_owned()
                .map_err(|_| RegistryError::Capacity)?;
            revisions.insert(
                id.clone(),
                Arc::new(DefinitionRevision {
                    data: RevisionData {
                        id,
                        machine: row.definition.machine,
                        role: row.role,
                        hash: row.definition.hash,
                        published: row.published,
                    },
                    _permit: permit,
                }),
            );
        }
    }
    Ok(revisions)
}
fn stage_names(
    rows: BTreeMap<String, Option<DateTime<Utc>>>,
    shared: &Shared,
    clock: ClockSample,
) -> Result<BTreeMap<String, Retention>, RegistryError> {
    let mut names = BTreeMap::new();
    for (arn, expiry) in rows {
        let retention = match expiry {
            Some(expiry) if expiry > clock.wall => until(expiry, clock)?,
            Some(_) => continue,
            None => after(NAME_RETENTION, clock)?,
        };
        if names.len() >= shared.restore.limits.capacities.names {
            return Err(RegistryError::Capacity);
        }
        names.insert(arn, retention);
    }
    Ok(names)
}
impl Validated {
    fn stage(self, shared: &Shared) -> Result<Staged, RegistryError> {
        let clock = ClockSample::new(Instant::now().into_std(), Utc::now());
        if self.context.captured > clock.wall {
            return Err(RegistryError::Validation);
        }
        let executions: Vec<_> = self
            .executions
            .into_iter()
            .filter(|entry| entry.expires.is_none_or(|expiry| expiry > clock.wall))
            .collect();
        let machines: Vec<_> = self
            .machines
            .into_iter()
            .filter(|entry| entry.active)
            .collect();
        let mut needed = BTreeSet::new();
        for entry in &executions {
            needed.insert(entry.revision.as_str());
        }
        for entry in &machines {
            needed.insert(entry.revision.as_str());
        }
        if machines.len() > shared.restore.limits.capacities.machines {
            return Err(RegistryError::Capacity);
        }
        let revisions = stage_revisions(self.revisions, &needed, shared)?;
        let mut staged = Staged {
            machines: BTreeMap::new(),
            records: BTreeMap::new(),
            names: BTreeMap::new(),
        };
        for row in machines {
            let identity = self
                .resources
                .get(&row.resource)
                .ok_or(RegistryError::Validation)?
                .clone();
            let permit = shared
                .restore
                .pools
                .tags
                .clone()
                .try_acquire_owned()
                .map_err(|_| RegistryError::Capacity)?;
            staged.machines.insert(
                identity.arn.as_str().into(),
                MachineSnapshot {
                    identity,
                    updated: row.updated,
                    status: MachineStatus::Active,
                    revision: revisions
                        .get(&row.revision)
                        .ok_or(RegistryError::Validation)?
                        .clone(),
                    tags: Arc::new(TagAllocation {
                        value: row.tags,
                        _permit: permit,
                    }),
                },
            );
        }
        staged.names = stage_names(self.names, shared, clock)?;
        for row in executions {
            let retention = match row.expires {
                Some(expiry) => until(expiry, clock)?,
                None => after(shared.restore.limits.retention, clock)?,
            };
            let permit = shared
                .restore
                .pools
                .records
                .clone()
                .try_acquire_owned()
                .map_err(|_| RegistryError::Capacity)?;
            let handle = shared.restore.history.terminal(
                row.history,
                row.outcome,
                row.stopped,
                clock.wall,
            )?;
            let arn = row.identity.arn.as_str().to_owned();
            let record = Arc::new(ExecutionRecord {
                lineage: Lineage::default(),
                deadline: None,
                identity: row.identity,
                input: row.input,
                revision: revisions
                    .get(&row.revision)
                    .ok_or(RegistryError::Validation)?
                    .clone(),
                handle,
                _permit: permit,
            });
            staged.records.insert(arn, Entry { record, retention });
        }
        Ok(staged)
    }
}
pub(super) fn prepare(
    document: RegistrySnapshot,
    shared: &Shared,
    limits: SnapshotLimits,
) -> Result<Staged, RegistryError> {
    budget(&document, limits)?;
    let context = Context::new(&document, shared)?;
    context.compatible(shared)?;
    let resources = resources(document.resources, &context)?;
    let revisions = revisions(document.revisions, &context, &resources)?;
    let names = names(document.names, &context)?;
    let machines = machines(document.machines, &resources, &revisions)?;
    let executions = executions(
        document.executions,
        &context,
        &resources,
        &revisions,
        &names,
    )?;
    let data = Validated {
        context,
        resources,
        revisions,
        machines,
        executions,
        names,
    };
    references(&data)?;
    data.stage(shared)
}
