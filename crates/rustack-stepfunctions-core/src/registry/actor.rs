//! One linearization lane with retained execution/revision/name ownership.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    future::pending,
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::json;
use tokio::{
    sync::{Semaphore, mpsc, watch},
    task::JoinSet,
    time::{Instant, MissedTickBehavior, interval},
};

use super::{
    DefinitionRevision, Envelope, ExecutionArn, ExecutionName, ExecutionSnapshot, MachineSnapshot,
    MachineStatus, PreparedDefinition, Profile, RegistryCommand, RegistryError, RegistryLimits,
    RegistryResult, RegistryStatus, Reply, RoleArn, StateMachineArn, TenantScope,
    child::{
        ChildError, Lineage, Request,
        owner::{Inbox as ChildInbox, Owner as ChildOwner},
    },
    types::{ExecutionIdentity, ExecutionRecord, MachineIdentity, RevisionData, TagAllocation},
};
use crate::{
    CleanupState, ClockSample, EvaluationContext, ExecutionDriver, ExecutionError, ExecutionLimits,
    ExecutionSupervisor, Payload, StateSupport, SystemUuidSource, UuidSource,
};

mod child;

const NAME_RETENTION: Duration = Duration::from_hours(2160);
struct Resources {
    scope: TenantScope,
    states: StateSupport,
    limits: RegistryLimits,
    machines: BTreeMap<String, MachineSnapshot>,
    revisions: Arc<Semaphore>,
    tag_versions: Arc<Semaphore>,
}
struct Entry {
    record: Arc<ExecutionRecord>,
    closed: Option<Instant>,
}
struct Executions {
    records: BTreeMap<String, Entry>,
    claims: BTreeMap<String, Option<Instant>>,
    capacity: Arc<Semaphore>,
    limits: ExecutionLimits,
}
struct Io {
    children: mpsc::Receiver<Request>,
    receiver: mpsc::Receiver<Envelope>,
    closing: watch::Sender<bool>,
    status: watch::Sender<RegistryStatus>,
    pending: Option<Envelope>,
    ready: VecDeque<String>,
    #[cfg(test)]
    entropy_hook: Option<clock_tests::EntropyHook>,
}
struct State {
    children: ChildOwner,
    resources: Resources,
    executions: Executions,
    runtime: ExecutionSupervisor,
    io: Io,
    entropy: JoinSet<Result<VecDeque<String>, RegistryError>>,
}
pub(super) struct Owner {
    state: Option<State>,
}
impl Owner {
    pub(super) fn new(
        scope: TenantScope,
        limits: RegistryLimits,
        runtime: ExecutionSupervisor,
        profile: Profile,
        channels: (mpsc::Receiver<Envelope>, ChildInbox),
        closing: watch::Sender<bool>,
        status: watch::Sender<RegistryStatus>,
    ) -> Self {
        let (
            receiver,
            ChildInbox {
                requests: children,
                registry,
            },
        ) = channels;
        Self {
            state: Some(State {
                children: ChildOwner::new(registry),
                resources: Resources {
                    scope,
                    states: profile.states,
                    limits,
                    machines: BTreeMap::new(),
                    revisions: Arc::new(Semaphore::new(limits.capacities.revisions)),
                    tag_versions: Arc::new(Semaphore::new(limits.capacities.revisions)),
                },
                executions: Executions {
                    records: BTreeMap::new(),
                    claims: BTreeMap::new(),
                    capacity: Arc::new(Semaphore::new(limits.capacities.records)),
                    limits: profile.execution,
                },
                runtime,
                io: Io {
                    children,
                    receiver,
                    closing,
                    status,
                    pending: None,
                    ready: VecDeque::new(),
                    #[cfg(test)]
                    entropy_hook: None,
                },
                entropy: JoinSet::new(),
            }),
        }
    }
    pub(super) async fn run(mut self, stop: watch::Receiver<bool>) -> Result<(), RegistryError> {
        let result = match self.state.as_mut() {
            Some(state) => state.run(stop).await,
            None => Err(RegistryError::Unavailable),
        };
        if result.is_ok() {
            drop(self.state.take());
        }
        result
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(mut state) = self.state.take() {
            state.io.closing.send_replace(true);
            state.runtime.close();
            state.io.children.close();
            while let Ok(request) = state.io.children.try_recv() {
                let _sent = request.reply.send(Err(ChildError::Unavailable));
            }
            state.io.receiver.close();
            while let Ok(envelope) = state.io.receiver.try_recv() {
                reply(envelope, Err(RegistryError::Unavailable));
            }
            // A root panic/unpolled future/runtime loss is not physical cleanup.
            // Preserve the bounded complete graph, including all permits/receipts.
            let _retained = Box::leak(Box::new(state));
        }
    }
}
fn reply(envelope: Envelope, result: Result<RegistryResult, RegistryError>) {
    let _sent = envelope.reply.send(Reply {
        result,
        permit: envelope.permit,
    });
}
fn uuid() -> Result<String, RegistryError> {
    Ok(SystemUuidSource::default().next_uuid()?.to_string())
}
enum Incoming {
    Command(Option<Envelope>),
    Child(Option<Request>),
}
impl fmt::Debug for Incoming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryIncoming(<redacted>)")
    }
}
async fn receive(
    commands: &mut mpsc::Receiver<Envelope>,
    children: &mut mpsc::Receiver<Request>,
    prefer_child: bool,
    commands_open: bool,
    children_open: bool,
) -> Incoming {
    if !commands_open && !children_open {
        return pending().await;
    }
    if prefer_child {
        tokio::select! {biased;
            request=children.recv(),if children_open=>Incoming::Child(request),
            command=commands.recv(),if commands_open=>Incoming::Command(command),
        }
    } else {
        tokio::select! {biased;
            command=commands.recv(),if commands_open=>Incoming::Command(command),
            request=children.recv(),if children_open=>Incoming::Child(request),
        }
    }
}
impl State {
    async fn run(&mut self, mut stop: watch::Receiver<bool>) -> Result<(), RegistryError> {
        let mut commands_open = true;
        let mut children_open = true;
        let mut prefer_child = false;
        let mut timer = interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            if *stop.borrow() || stop.has_changed().is_err() {
                break;
            }
            tokio::select! {biased;
                _=stop.changed()=>{},
                ()=self.children.registry.child_wake.notified()=>self.maintain(Instant::now()),
                input=receive(&mut self.io.receiver,&mut self.io.children,prefer_child,commands_open,children_open)=>match input {
                    Incoming::Command(envelope)=>{prefer_child=true;match envelope {Some(envelope)=>self.dispatch(envelope).await,None=>commands_open=false}},
                    Incoming::Child(request)=>{prefer_child=false;match request {Some(request)=>self.dispatch_child(request).await,None=>children_open=false}},
                },
                _=timer.tick()=>self.maintain(Instant::now()),
            }
        }
        self.io.closing.send_replace(true);
        self.close_child_intake();
        self.io.receiver.close();
        while let Some(envelope) = self.io.receiver.recv().await {
            self.dispatch(envelope).await;
        }
        loop {
            let deadline = Instant::now()
                .checked_add(Duration::from_secs(30))
                .ok_or(RegistryError::Validation)?;
            match self.runtime.drain(deadline).await {
                Ok(()) => return Ok(()),
                Err(ExecutionError::Deadline) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    #[tracing::instrument(name = "stepfunctions.registry.command", skip_all)]
    async fn dispatch(&mut self, envelope: Envelope) {
        let needs_entropy = matches!(
            envelope.command,
            RegistryCommand::Create(..)
                | RegistryCommand::Update(..)
                | RegistryCommand::Start(_, None, _, _)
        );
        self.io.pending = Some(envelope);
        self.io.ready.clear();
        let prepared = if needs_entropy {
            #[cfg(test)]
            let hook = self.io.entropy_hook.take();
            self.entropy.spawn_blocking(move || {
                #[cfg(test)]
                if let Some(hook) = hook {
                    let _notified = hook.entered.send(());
                    let _released = hook.gate.recv();
                }
                Ok(VecDeque::from([uuid()?, uuid()?]))
            });
            self.await_entropy().await
        } else {
            Ok(())
        };
        if let Some(envelope) = self.io.pending.take() {
            self.maintain(Instant::now());
            let result = prepared
                .and_then(|()| {
                    envelope
                        .command
                        .check(&self.resources.scope, self.resources.states)
                })
                .and_then(|()| self.command(&envelope.command));
            self.maintain(Instant::now());
            reply(envelope, result);
        }
    }
    fn close_child_intake(&mut self) {
        self.io.children.close();
        while let Ok(request) = self.io.children.try_recv() {
            let _sent = request.reply.send(Err(ChildError::Unavailable));
        }
    }
    async fn await_entropy(&mut self) -> Result<(), RegistryError> {
        let mut closing = self.io.closing.subscribe();
        let mut maintenance = interval(Duration::from_secs(1));
        maintenance.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut closed = false;
        loop {
            if !closed && *closing.borrow_and_update() {
                self.runtime.close();
                self.close_child_intake();
                self.maintain(Instant::now());
                closed = true;
            }
            tokio::select! { biased;
                _=closing.changed()=>{},
                ()=self.children.registry.child_wake.notified()=>self.maintain(Instant::now()),
                _=maintenance.tick()=>self.maintain(Instant::now()),
                joined=self.entropy.join_next()=>return match joined {
                    Some(Ok(Ok(values)))=>{self.io.ready=values;Ok(())},
                    Some(Ok(Err(error)))=>Err(error),
                    Some(Err(error))=>Err(RegistryError::Worker(error)),
                    None=>Err(RegistryError::Unavailable),
                },
            }
        }
    }
    fn next_uuid(&mut self) -> Result<String, RegistryError> {
        self.io.ready.pop_front().ok_or(RegistryError::Unavailable)
    }
    fn command(&mut self, command: &RegistryCommand) -> Result<RegistryResult, RegistryError> {
        match command {
            RegistryCommand::Create(name, definition, role, tags) => {
                self.create(name, definition, role, tags)
            }
            RegistryCommand::Update(arn, definition, role) => {
                let current = self.resources.active(arn)?.clone();
                let definition = definition.clone().unwrap_or_else(|| PreparedDefinition {
                    machine: current.revision.data.machine.clone(),
                    hash: current.revision.data.hash,
                });
                let revision_id = self.next_uuid()?;
                let now = Utc::now();
                let revision = self.resources.revision(
                    definition,
                    role.clone()
                        .unwrap_or_else(|| current.revision.data.role.clone()),
                    revision_id,
                    now,
                )?;
                let snapshot = MachineSnapshot {
                    revision,
                    updated: now,
                    ..current
                };
                self.resources
                    .machines
                    .insert(arn.as_str().into(), snapshot.clone());
                Ok(RegistryResult::Machine(snapshot))
            }
            RegistryCommand::Delete(arn) => {
                self.resources
                    .machines
                    .get_mut(arn.as_str())
                    .ok_or(RegistryError::MachineMissing)?
                    .status = MachineStatus::Deleting;
                Ok(RegistryResult::Deleted)
            }
            RegistryCommand::Machine(arn) => Ok(RegistryResult::Machine(
                self.resources
                    .machines
                    .get(arn.as_str())
                    .ok_or(RegistryError::MachineMissing)?
                    .clone(),
            )),
            RegistryCommand::Machines => {
                let mut machines: Vec<_> = self.resources.machines.values().cloned().collect();
                machines.sort_by(|a, b| {
                    a.creation_date()
                        .cmp(&b.creation_date())
                        .then_with(|| a.arn().as_str().cmp(b.arn().as_str()))
                });
                Ok(RegistryResult::Machines(machines))
            }
            RegistryCommand::Start(arn, name, input, trace) => self.start(
                arn,
                name.as_ref(),
                input,
                trace.as_deref(),
                Lineage::default(),
            ),
            RegistryCommand::Execution(arn) => {
                Ok(RegistryResult::Execution(self.snapshot(arn.as_str())?))
            }
            RegistryCommand::Stop(arn, reason) => {
                let snapshot = self.snapshot(arn.as_str())?;
                snapshot.record.handle.request_stop(reason.0.clone());
                Ok(RegistryResult::Execution(snapshot))
            }
            RegistryCommand::Executions(arn) => self.list_executions(arn),
            RegistryCommand::Tag(arn, tags) | RegistryCommand::Untag(arn, tags) => {
                let current = self
                    .resources
                    .machines
                    .get_mut(arn.as_str())
                    .ok_or(RegistryError::MachineMissing)?;
                let value = if matches!(command, RegistryCommand::Tag(..)) {
                    current.tags.value.merge(tags)?
                } else {
                    current.tags.value.remove(tags)
                };
                let permit = self
                    .resources
                    .tag_versions
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| RegistryError::Capacity)?;
                current.tags = Arc::new(TagAllocation {
                    value,
                    _permit: permit,
                });
                Ok(RegistryResult::Tags(current.tags.value.clone()))
            }
        }
    }
    fn list_executions(&self, arn: &StateMachineArn) -> Result<RegistryResult, RegistryError> {
        if !self.resources.machines.contains_key(arn.as_str()) {
            return Err(RegistryError::MachineMissing);
        }

        let mut executions: Vec<_> = self
            .executions
            .records
            .values()
            .filter(|entry| entry.record.identity.arn.machine().as_str() == arn.as_str())
            .map(|entry| ExecutionSnapshot {
                record: entry.record.clone(),
                view: entry.record.handle.snapshot(),
            })
            .collect();
        executions.sort_by(|a, b| {
            b.start_date()
                .cmp(&a.start_date())
                .then_with(|| a.arn().as_str().cmp(b.arn().as_str()))
        });
        Ok(RegistryResult::Executions(executions))
    }
    fn create(
        &mut self,
        name: &super::StateMachineName,
        definition: &PreparedDefinition,
        role: &RoleArn,
        tags: &super::Tags,
    ) -> Result<RegistryResult, RegistryError> {
        let arn = StateMachineArn::new(&self.resources.scope, name.clone())?;
        if let Some(existing) = self.resources.machines.get(arn.as_str()) {
            if existing.status == MachineStatus::Deleting {
                return Err(RegistryError::Deleting);
            }
            if existing.revision.machine().original_bytes() == definition.machine.original_bytes() {
                return Ok(RegistryResult::Machine(existing.clone()));
            }
            return Err(RegistryError::MachineExists);
        }
        if self.resources.machines.len() >= self.resources.limits.capacities.machines {
            return Err(RegistryError::Capacity);
        }
        let revision_id = self.next_uuid()?;
        let resource_id = self.next_uuid()?;
        let now = Utc::now();
        let revision =
            self.resources
                .revision(definition.clone(), role.clone(), revision_id, now)?;
        let snapshot = MachineSnapshot {
            identity: Arc::new(MachineIdentity {
                arn: arn.clone(),
                resource: resource_id,
                created: now,
            }),
            updated: now,
            status: MachineStatus::Active,
            revision,
            tags: Arc::new(TagAllocation {
                value: tags.clone(),
                _permit: self
                    .resources
                    .tag_versions
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| RegistryError::Capacity)?,
            }),
        };
        self.resources
            .machines
            .insert(arn.as_str().into(), snapshot.clone());
        Ok(RegistryResult::Machine(snapshot))
    }
    fn snapshot(&self, arn: &str) -> Result<ExecutionSnapshot, RegistryError> {
        let entry = self
            .executions
            .records
            .get(arn)
            .ok_or(RegistryError::ExecutionMissing)?;
        Ok(ExecutionSnapshot {
            record: entry.record.clone(),
            view: entry.record.handle.snapshot(),
        })
    }
    fn start(
        &mut self,
        arn: &StateMachineArn,
        name: Option<&ExecutionName>,
        input: &Payload,
        trace: Option<&str>,
        lineage: Lineage,
    ) -> Result<RegistryResult, RegistryError> {
        let machine = self.resources.active(arn)?.clone();
        let name = match name {
            Some(name) => name.clone(),
            None => self.next_uuid()?.parse()?,
        };
        let arn = ExecutionArn::new(arn.clone(), name)?;
        if self.executions.claims.contains_key(arn.as_str()) {
            if let Ok(snapshot) = self.snapshot(arn.as_str()) {
                if snapshot.view.outcome().is_none()
                    && snapshot.input().original_bytes() == input.original_bytes()
                {
                    return Ok(RegistryResult::Execution(snapshot));
                }
            }
            return Err(RegistryError::ExecutionExists);
        }
        if self.executions.claims.len() >= self.resources.limits.capacities.names {
            return Err(RegistryError::Capacity);
        }
        let permit = self
            .executions
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| RegistryError::Capacity)?;
        let now = Utc::now();
        let context = EvaluationContext::from_value(
            &json!({"Execution":{"Id":arn.as_str(),"Name":arn.name().as_str(),"Input":input.value(),"RoleArn":machine.revision.role().as_str(),"StartTime":now.to_rfc3339_opts(SecondsFormat::Millis,true)},"StateMachine":{"Id":machine.arn().as_str(),"Name":machine.arn().name().as_str()}}),
        )?;
        let structured = machine.revision.machine().requires_structured_execution()
            || machine.revision.machine().uses_children();
        let construct = if structured {
            ExecutionDriver::for_structured_owner
        } else {
            ExecutionDriver::new
        };
        let driver = construct(
            machine.revision.machine().clone(),
            input.clone(),
            context,
            ClockSample::new(Instant::now().into_std(), now),
            self.executions.limits,
        )?;
        let absolute_deadline = driver.deadline();
        // No await between runtime registration and the atomic record/name commit.
        let registered = self.runtime.start_registered(
            driver,
            structured,
            super::child::port(&self.children.registry),
        )?;
        let handle = registered.handle;
        let view = handle.snapshot();
        let record = Arc::new(ExecutionRecord {
            lineage,
            deadline: absolute_deadline,
            identity: ExecutionIdentity {
                arn: arn.clone(),
                machine: machine.identity,
                started: now,
                trace: trace.map(str::to_owned),
            },
            input: input.clone(),
            revision: machine.revision,
            handle,
            _permit: permit,
        });
        self.executions.claims.insert(arn.as_str().into(), None);
        self.executions.records.insert(
            arn.as_str().into(),
            Entry {
                record: record.clone(),
                closed: None,
            },
        );
        let snapshot = ExecutionSnapshot { record, view };
        if let Some(parent) = registered.parent {
            let _ = parent.send(snapshot.child_parent());
        }
        Ok(RegistryResult::Execution(snapshot))
    }
    fn maintain(&mut self, now: Instant) {
        self.children.maintain(now);
        self.runtime.reap_ready();
        self.io.status.send_replace(RegistryStatus {
            ready: self.runtime.is_ready(),
            owned_executions: self.runtime.owned_executions(),
        });
        for (arn, entry) in &mut self.executions.records {
            if entry.closed.is_none() && entry.record.handle.snapshot().outcome().is_some() {
                entry.closed = Some(now);
                if let Some(closed) = self.executions.claims.get_mut(arn) {
                    *closed = Some(now);
                }
            }
        }
        self.executions.records.retain(|_, entry| {
            entry.closed.is_none_or(|closed| {
                now.saturating_duration_since(closed) < self.resources.limits.retention
            }) || entry.record.handle.snapshot().cleanup() != CleanupState::Clean
        });
        let records = &self.executions.records;
        self.executions.claims.retain(|arn, closed| {
            closed.is_none_or(|closed| now.saturating_duration_since(closed) < NAME_RETENTION)
                || records.get(arn).is_some_and(|entry| {
                    entry.record.handle.snapshot().cleanup() != CleanupState::Clean
                })
        });
        let running: BTreeSet<_> = self
            .executions
            .records
            .values()
            .filter(|entry| entry.record.handle.snapshot().outcome().is_none())
            .map(|entry| entry.record.identity.machine.resource.as_str())
            .collect();
        self.resources.machines.retain(|_, machine| {
            machine.status == MachineStatus::Active
                || running.contains(machine.identity.resource.as_str())
        });
    }
}
impl Resources {
    fn active(&self, arn: &StateMachineArn) -> Result<&MachineSnapshot, RegistryError> {
        let machine = self
            .machines
            .get(arn.as_str())
            .ok_or(RegistryError::MachineMissing)?;
        if machine.status == MachineStatus::Deleting {
            Err(RegistryError::Deleting)
        } else {
            Ok(machine)
        }
    }
    fn revision(
        &self,
        definition: PreparedDefinition,
        role: RoleArn,
        id: String,
        published: DateTime<Utc>,
    ) -> Result<Arc<DefinitionRevision>, RegistryError> {
        let permit = self
            .revisions
            .clone()
            .try_acquire_owned()
            .map_err(|_| RegistryError::Capacity)?;
        Ok(Arc::new(DefinitionRevision {
            data: RevisionData {
                id,
                machine: definition.machine,
                role,
                hash: definition.hash,
                published,
            },
            _permit: permit,
        }))
    }
}

#[cfg(test)]
mod clock_tests {
    mod children;
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };

    use super::{
        super::{Admission, RegistryCapacities, Tags, admission},
        *,
    };
    use crate::{
        AslCompiler, HistoryLimits, LambdaTaskCommand, LambdaTaskPort, RuntimeLimits, TaskCall,
        TaskFailure, TaskResponder, TaskSupport,
    };

    pub(super) struct EntropyHook {
        pub(super) entered: tokio::sync::oneshot::Sender<()>,
        pub(super) gate: std::sync::mpsc::Receiver<()>,
    }
    fn definition() -> PreparedDefinition {
        PreparedDefinition::new(
            AslCompiler::default()
                .compile(br#"{"StartAt":"D","States":{"D":{"Type":"Succeed"}}}"#)
                .unwrap()
                .machine()
                .clone(),
        )
    }
    fn owner(capacities: RegistryCapacities, port: Option<Arc<dyn LambdaTaskPort>>) -> Owner {
        let runtime = ExecutionSupervisor::new(
            RuntimeLimits::default(),
            HistoryLimits::default(),
            256 * 1024 * 1024,
            port,
        )
        .unwrap();
        let limits = RegistryLimits::new(capacities, Duration::from_secs(60), 8).unwrap();
        let Admission { shared, channels } = admission(
            TenantScope::default(),
            limits,
            StateSupport::Sequential,
            runtime.open_limit(),
        );
        Owner::new(
            TenantScope::default(),
            limits,
            runtime,
            Profile::default(),
            channels,
            shared.closing.clone(),
            shared.status.clone(),
        )
    }
    fn create(state: &mut State, definition: PreparedDefinition) -> StateMachineArn {
        state.io.ready = VecDeque::from([uuid().unwrap(), uuid().unwrap()]);
        let RegistryResult::Machine(machine) = state
            .command(&RegistryCommand::Create(
                "Machine".parse().unwrap(),
                definition,
                "arn:aws:iam::000000000000:role/Worker".parse().unwrap(),
                Tags::default(),
            ))
            .unwrap()
        else {
            panic!("machine")
        };
        machine.arn().clone()
    }
    fn start(
        state: &mut State,
        arn: &StateMachineArn,
        name: &str,
    ) -> Result<ExecutionSnapshot, RegistryError> {
        let RegistryResult::Execution(snapshot) = state.start(
            arn,
            Some(&name.parse().unwrap()),
            &Payload::parse(b"{}").unwrap(),
            None,
            Lineage::default(),
        )?
        else {
            panic!("execution")
        };
        Ok(snapshot)
    }
    async fn settled(snapshot: &ExecutionSnapshot) {
        let mut handle = snapshot.handle();
        handle
            .wait(Instant::now() + Duration::from_secs(5))
            .await
            .unwrap();
        // Read terminal publication is independent of the final cleanup publication.
        let end = Instant::now() + Duration::from_secs(5);
        while handle.snapshot().cleanup() != CleanupState::Clean {
            assert!(Instant::now() < end);
            tokio::task::yield_now().await;
        }
    }
    #[tokio::test]
    async fn test_should_expire_records_independently_of_names_and_retained_snapshot_slots() {
        let mut owner = owner(RegistryCapacities::new(1, 4, 1, 4).unwrap(), None);
        let mut state = owner.state.take().unwrap();
        let arn = create(&mut state, definition());
        let old = start(&mut state, &arn, "old").unwrap();
        settled(&old).await;
        let closed = Instant::now();
        state.maintain(closed);
        state.maintain(closed + Duration::from_secs(60));
        assert!(state.executions.records.is_empty());
        assert!(state.executions.claims.contains_key(old.arn().as_str()));
        assert!(matches!(
            start(&mut state, &arn, "old"),
            Err(RegistryError::ExecutionExists)
        ));
        assert!(matches!(
            start(&mut state, &arn, "new"),
            Err(RegistryError::Capacity)
        ));
        drop(old);
        let new = start(&mut state, &arn, "new").unwrap();
        settled(&new).await;
        state.maintain(closed + Duration::from_secs(61));
        drop(new);
        state.maintain(closed + NAME_RETENTION + Duration::from_secs(61));
        assert!(state.executions.claims.is_empty());
        let reused = start(&mut state, &arn, "old").unwrap();
        settled(&reused).await;
        state
            .runtime
            .drain(Instant::now() + Duration::from_secs(5))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn test_should_reject_name_capacity_without_evicting_an_unexpired_closed_name() {
        let mut owner = owner(RegistryCapacities::new(1, 4, 4, 1).unwrap(), None);
        let mut state = owner.state.take().unwrap();
        let arn = create(&mut state, definition());
        let old = start(&mut state, &arn, "old").unwrap();
        settled(&old).await;
        let closed = Instant::now();
        state.maintain(closed);
        drop(old);
        state.maintain(closed + Duration::from_secs(120));
        assert!(state.executions.records.is_empty());
        assert!(matches!(
            start(&mut state, &arn, "new"),
            Err(RegistryError::Capacity)
        ));
        assert!(matches!(
            start(&mut state, &arn, "old"),
            Err(RegistryError::ExecutionExists)
        ));
        state.maintain(closed + NAME_RETENTION);
        let reused = start(&mut state, &arn, "old").unwrap();
        settled(&reused).await;
        state
            .runtime
            .drain(Instant::now() + Duration::from_secs(5))
            .await
            .unwrap();
    }
    #[derive(Debug)]
    struct Port(mpsc::Sender<TaskResponder>);
    impl LambdaTaskPort for Port {
        fn start(&self, _command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
            let (call, owner) = TaskCall::channel();
            self.0
                .try_send(owner)
                .map_err(|_| TaskFailure::throttled())?;
            Ok(call)
        }
    }
    #[tokio::test]
    async fn test_should_retain_revision_record_and_name_beyond_all_ttls_until_cleanup_is_clean() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut owner = owner(
            RegistryCapacities::new(1, 1, 1, 1).unwrap(),
            Some(Arc::new(Port(tx))),
        );
        let mut state = owner.state.take().unwrap();
        let prepared=PreparedDefinition::new(AslCompiler::new(TenantScope::default(),TaskSupport::Lambda).compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}}"#).unwrap().machine().clone());
        let arn = create(&mut state, prepared);
        let execution = start(&mut state, &arn, "uncertain").unwrap();
        let mut handle = execution.handle();
        let responder = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let receipt = responder.quarantine(TaskFailure::service_exception());
        handle
            .wait(Instant::now() + Duration::from_secs(5))
            .await
            .unwrap();
        drop(execution);
        let closed = Instant::now();
        state.maintain(closed);
        state
            .command(&RegistryCommand::Delete(arn.clone()))
            .unwrap();
        state.maintain(closed + NAME_RETENTION + Duration::from_secs(1));
        assert!(state.resources.machines.is_empty());
        assert_eq!(state.executions.records.len(), 1);
        assert_eq!(state.executions.claims.len(), 1);
        assert_eq!(state.resources.revisions.available_permits(), 0);
        assert_eq!(state.executions.capacity.available_permits(), 0);
        receipt.settle();
        let end = Instant::now() + Duration::from_secs(5);
        while handle.snapshot().cleanup() != CleanupState::Clean {
            assert!(Instant::now() < end);
            tokio::task::yield_now().await;
        }
        state.maintain(closed + NAME_RETENTION + Duration::from_secs(2));
        assert!(state.executions.records.is_empty());
        assert!(state.executions.claims.is_empty());
        assert_eq!(state.resources.revisions.available_permits(), 1);
        assert_eq!(state.executions.capacity.available_permits(), 1);
        state.runtime.drain(end).await.unwrap();
    }
    #[tokio::test]
    async fn test_should_retain_pending_command_and_entropy_worker_across_root_abort() {
        let mut owner = owner(RegistryCapacities::default(), None);
        let (sender, receiver) = mpsc::channel(1);
        let (closing, stop) = watch::channel(false);
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let state = owner.state.as_mut().unwrap();
        state.io.receiver = receiver;
        state.io.closing = closing.clone();
        state.io.entropy_hook = Some(EntropyHook { entered, gate });
        let capacity = Arc::new(Semaphore::new(1));
        let (reply, mut observed) = tokio::sync::oneshot::channel();
        let envelope = Envelope {
            command: RegistryCommand::Create(
                "Machine".parse().unwrap(),
                definition(),
                "arn:aws:iam::000000000000:role/Worker".parse().unwrap(),
                Tags::default(),
            ),
            reply,
            permit: capacity.clone().try_acquire_owned().unwrap(),
        };
        assert!(sender.try_send(envelope).is_ok());
        let root = tokio::spawn(owner.run(stop));
        ready.await.unwrap();
        root.abort();
        assert!(root.await.unwrap_err().is_cancelled());
        assert!(*closing.borrow());
        assert_eq!(capacity.available_permits(), 0);
        assert!(observed.try_recv().is_err());
        drop(release); // Receiver disconnection releases even if any assertion unwinds.
        assert_eq!(capacity.available_permits(), 0);
    }
    #[tokio::test]
    async fn test_should_keep_registry_owner_live_after_last_command_sender_is_dropped() {
        let mut owner = owner(RegistryCapacities::default(), None);
        let mut state = owner.state.take().unwrap();
        // The helper's only command sender was dropped when it returned.
        let closing = state.io.closing.clone();
        let stop = closing.subscribe();
        let mut run = Box::pin(state.run(stop));
        poll_fn(|cx| {
            assert!(matches!(run.as_mut().poll(cx), Poll::Pending));
            Poll::Ready(())
        })
        .await;
        assert!(!*closing.borrow());
        closing.send_replace(true);
        run.await.unwrap();
    }
}
