//! Domain and actual registry/execution lifecycle regressions.

use std::{error::Error, sync::Arc, time::Duration};

use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;
use tokio::{
    sync::mpsc,
    time::{Instant, timeout},
};

use super::*;
use crate::{
    AslCompiler, ExecutionError, ExecutionOutcome, FrameCapacity, FrameLimits, HistoryLimits,
    LambdaTaskCommand, LambdaTaskPort, RuntimeLimits, StateSupport, StateView, TaskCall,
    TaskFailure, TaskResponder, TaskSupport,
};

#[path = "tests/children.rs"]
mod children;
#[path = "tests/nested.rs"]
mod nested;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn role() -> RoleArn {
    "arn:aws:iam::000000000000:role/service-role/Worker"
        .parse()
        .unwrap()
}
fn arn() -> StateMachineArn {
    "arn:aws:states:us-east-1:000000000000:stateMachine:Machine"
        .parse()
        .unwrap()
}
fn definition(value: i64, wait: bool) -> PreparedDefinition {
    let states = if wait {
        json!({"StartAt":"Wait","States":{"Wait":{"Type":"Wait","Seconds":60,"Next":"Done"},"Done":{"Type":"Pass","Result":value,"End":true}}})
    } else {
        json!({"StartAt":"Done","States":{"Done":{"Type":"Pass","Result":value,"End":true}}})
    };
    PreparedDefinition::new(
        AslCompiler::default()
            .compile(&serde_json::to_vec(&states).unwrap())
            .unwrap()
            .machine()
            .clone(),
    )
}
fn registry(
    limits: RegistryLimits,
    port: Option<Arc<dyn LambdaTaskPort>>,
) -> (Registry, RegistrySupervisor) {
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        port,
    )
    .unwrap();
    Registry::new(
        TenantScope::default(),
        limits,
        runtime,
        ExecutionLimits::default(),
    )
    .unwrap()
}
async fn result(reg: &Registry, command: RegistryCommand) -> Result<RegistryResult, RegistryError> {
    reg.submit(command)?.wait(deadline()).await
}
async fn machine(reg: &Registry, value: i64, wait: bool) -> MachineSnapshot {
    match result(
        reg,
        RegistryCommand::Create(
            "Machine".parse().unwrap(),
            definition(value, wait),
            role(),
            Tags::default(),
        ),
    )
    .await
    .unwrap()
    {
        RegistryResult::Machine(machine) => machine,
        _ => panic!("expected machine"),
    }
}
async fn start(
    reg: &Registry,
    name: &str,
    input: &str,
) -> Result<ExecutionSnapshot, RegistryError> {
    match result(
        reg,
        RegistryCommand::Start(
            arn(),
            Some(name.parse().unwrap()),
            Payload::parse(input.as_bytes()).unwrap(),
            None,
        ),
    )
    .await?
    {
        RegistryResult::Execution(execution) => Ok(execution),
        _ => Err(RegistryError::Unavailable),
    }
}
async fn observe(mut snapshot: ExecutionSnapshot) -> ExecutionSnapshot {
    snapshot.view = snapshot.handle().wait(deadline()).await.unwrap();
    snapshot
}
#[rstest]
#[case("")]
#[case("a b")]
#[case("a:b")]
#[case("中文")]
#[case("a/b")]
#[case("a\n")]
fn test_should_reject_invalid_domain_names(#[case] value: &str) {
    assert!(value.parse::<StateMachineName>().is_err());
    assert!(value.parse::<ExecutionName>().is_err());
}
#[rstest]
#[case("arn:aws:states:us-east-1:000000000000:stateMachine:M:alias")]
#[case("arn:aws:states:us-east-1:000000000000:execution:M:N")]
#[case("arn:aws:s3:us-east-1:000000000000:stateMachine:M")]
#[case("arn:aws:states:us-east-1:0:stateMachine:M")]
fn test_should_reject_wrong_machine_arn_shapes(#[case] value: &str) {
    assert!(value.parse::<StateMachineArn>().is_err());
}
#[rstest]
#[case("arn:aws:iam:us-east-1:000000000000:role/Worker")]
#[case("arn:aws:lambda::000000000000:role/Worker")]
#[case("arn:aws:iam::000000000000:user/Worker")]
#[case("arn:aws:iam::000000000000:role//Worker")]
#[case("arn:aws:iam::000000000000:role/a b/Worker")]
#[case("arn:aws:iam::000000000000:role/Worker:bad")]
fn test_should_reject_invalid_role_arns(#[case] value: &str) {
    assert!(value.parse::<RoleArn>().is_err());
}
#[test]
fn test_should_bound_arns_names_roles_and_tags_and_redact_debug() {
    assert!("a".repeat(80).parse::<StateMachineName>().is_ok());
    assert!("a".repeat(81).parse::<StateMachineName>().is_err());
    let execution = ExecutionArn::new(arn(), "run".parse().unwrap()).unwrap();
    let reparsed: ExecutionArn = execution.as_str().parse().unwrap();
    assert_eq!(reparsed.as_str(), execution.as_str());
    assert!(
        format!("{}:v", execution.as_str())
            .parse::<ExecutionArn>()
            .is_err()
    );
    let unicode = Tags::new(vec![("团队".into(), "數值 １２".into())]).unwrap();
    assert_eq!(unicode.values().len(), 1);
    for values in [
        vec![("aws:key".into(), String::new())],
        vec![("AWS:key".into(), String::new())],
        vec![("a".into(), "x".into()), ("a".into(), "y".into())],
        vec![("a".into(), "\n".into())],
        vec![("a".repeat(129), String::new())],
        vec![("a".into(), "v".repeat(257))],
    ] {
        assert!(Tags::new(values).is_err());
    }
    assert!(
        Tags::new(
            (0..51)
                .map(|n| (format!("key{n}"), String::new()))
                .collect()
        )
        .is_err()
    );
    assert!(format!("{:?}{:?}{:?}{:?}", arn(), execution, role(), unicode).contains("redacted"));
    assert!(!format!("{:?}{:?}{:?}", arn(), role(), unicode).contains(":stateMachine:Machine"));
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn test_should_round_trip_generated_names_and_scoped_arns(machine in "[A-Za-z0-9_-]{1,80}",name in "[A-Za-z0-9_-]{1,80}"){
        let arn=StateMachineArn::new(&TenantScope::default(),machine.parse().unwrap()).unwrap();
        let parsed=arn.as_str().parse::<StateMachineArn>().unwrap();
        prop_assert_eq!(parsed.as_str(),arn.as_str());
        let execution=ExecutionArn::new(arn,name.parse().unwrap()).unwrap();
        let parsed=execution.as_str().parse::<ExecutionArn>().unwrap();
        prop_assert_eq!(parsed.name().as_str(),name);
    }
}
#[tokio::test]
async fn test_should_preserve_create_idempotency_role_tags_and_update_revision_isolation() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let first = machine(&reg, 1, true).await;
    let mut execution = start(&reg, "run", " {\"n\":1} ").await.unwrap();
    let other_role = "arn:aws:iam::000000000000:role/Other".parse().unwrap();
    let tags = Tags::new(vec![("key".into(), "ignored".into())]).unwrap();
    let RegistryResult::Machine(repeated) = result(
        &reg,
        RegistryCommand::Create(
            "Machine".parse().unwrap(),
            definition(1, true),
            other_role,
            tags,
        ),
    )
    .await
    .unwrap() else {
        panic!("machine")
    };
    assert_eq!(first.revision().id(), repeated.revision().id());
    assert_eq!(
        first.revision().role().as_str(),
        repeated.revision().role().as_str()
    );
    assert!(repeated.tags().values().is_empty());
    assert!(matches!(
        result(
            &reg,
            RegistryCommand::Create(
                "Machine".parse().unwrap(),
                definition(9, false),
                role(),
                Tags::default()
            )
        )
        .await,
        Err(RegistryError::MachineExists)
    ));
    let RegistryResult::Machine(second) = result(
        &reg,
        RegistryCommand::Update(arn(), Some(definition(2, false)), None),
    )
    .await
    .unwrap() else {
        panic!("machine")
    };
    assert_ne!(first.revision().id(), second.revision().id());
    assert_eq!(execution.revision().id(), first.revision().id());
    let replay = start(&reg, "run", " {\"n\":1} ").await.unwrap();
    assert_eq!(replay.start_date(), execution.start_date());
    assert_eq!(replay.revision().id(), first.revision().id());
    assert!(matches!(
        start(&reg, "run", "{\"n\":1}").await,
        Err(RegistryError::ExecutionExists)
    ));
    let new = observe(start(&reg, "new", "{}").await.unwrap()).await;
    assert!(
        matches!(new.view().outcome().map(Arc::as_ref),Some(ExecutionOutcome::Succeeded(p)) if p.value()==&json!(2))
    );
    execution.view = reg
        .stop(execution.arn().clone(), None, None, deadline())
        .await
        .unwrap()
        .view;
    assert!(matches!(
        start(&reg, "run", " {\"n\":1} ").await,
        Err(RegistryError::ExecutionExists)
    ));
    owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_linearize_concurrent_running_names_and_preserve_accepted_commands() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    machine(&reg, 1, true).await;
    let mut receipts = Vec::new();
    for _ in 0..16 {
        receipts.push(
            reg.submit(RegistryCommand::Start(
                arn(),
                Some("same".parse().unwrap()),
                Payload::parse(b"{}").unwrap(),
                None,
            ))
            .unwrap(),
        );
    }
    let mut start_date = None;
    for receipt in &mut receipts {
        let RegistryResult::Execution(e) = receipt.wait(deadline()).await.unwrap() else {
            panic!("execution")
        };
        if let Some(time) = start_date {
            assert_eq!(e.start_date(), time);
        }
        start_date = Some(e.start_date());
    }
    let dropped = reg
        .submit(RegistryCommand::Tag(
            arn(),
            Tags::new(vec![("committed".into(), "yes".into())]).unwrap(),
        ))
        .unwrap();
    drop(dropped);
    let RegistryResult::Machine(m) = result(&reg, RegistryCommand::Machine(arn())).await.unwrap()
    else {
        panic!("machine")
    };
    assert_eq!(
        m.tags().values().get("committed").map(String::as_str),
        Some("yes")
    );
    let RegistryResult::Executions(list) = result(&reg, RegistryCommand::Executions(arn()))
        .await
        .unwrap()
    else {
        panic!("list")
    };
    assert_eq!(list.len(), 1);
    owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_delete_after_last_business_completion_and_preserve_recreated_arn_names() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let old = machine(&reg, 1, true).await;
    let execution = start(&reg, "old", "{}").await.unwrap();
    result(&reg, RegistryCommand::Delete(arn())).await.unwrap();
    result(&reg, RegistryCommand::Delete(arn())).await.unwrap();
    assert!(matches!(
        start(&reg, "rejected", "{}").await,
        Err(RegistryError::Deleting)
    ));
    assert!(matches!(
        result(
            &reg,
            RegistryCommand::Update(arn(), Some(definition(2, false)), None)
        )
        .await,
        Err(RegistryError::Deleting)
    ));
    let stopped = reg
        .stop(execution.arn().clone(), None, None, deadline())
        .await
        .unwrap();
    assert_eq!(stopped.revision().id(), old.revision().id());
    assert!(matches!(
        result(&reg, RegistryCommand::Machine(arn())).await,
        Err(RegistryError::MachineMissing)
    ));
    assert!(matches!(
        result(&reg, RegistryCommand::Delete(arn())).await,
        Err(RegistryError::MachineMissing)
    ));
    let recreated = machine(&reg, 2, false).await;
    assert_ne!(old.resource_id(), recreated.resource_id());
    assert!(matches!(
        start(&reg, "old", "{}").await,
        Err(RegistryError::ExecutionExists)
    ));
    let RegistryResult::Execution(captured) =
        result(&reg, RegistryCommand::Execution(execution.arn().clone()))
            .await
            .unwrap()
    else {
        panic!("execution")
    };
    assert_eq!(captured.resource_id(), old.resource_id());
    assert_eq!(captured.revision().id(), old.revision().id());
    owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_preserve_optional_empty_stop_fields_and_terminal_history() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    machine(&reg, 1, true).await;
    for (name, error, cause) in [
        ("omitted", None, None),
        ("empty", Some(String::new()), Some(String::new())),
    ] {
        let execution = start(&reg, name, "{}").await.unwrap();
        let stopped = reg
            .stop(
                execution.arn().clone(),
                error.clone(),
                cause.clone(),
                deadline(),
            )
            .await
            .unwrap();
        let Some(ExecutionOutcome::Aborted(failure)) = stopped.view().outcome().map(Arc::as_ref)
        else {
            panic!("aborted")
        };
        assert_eq!(failure.details().error(), error.as_deref());
        assert_eq!(failure.details().cause(), cause.as_deref());
        let repeated = reg
            .stop(
                execution.arn().clone(),
                Some("ignored".into()),
                None,
                deadline(),
            )
            .await
            .unwrap();
        assert_eq!(stopped.view().stop_date(), repeated.view().stop_date());
        assert!(Arc::ptr_eq(
            stopped.view().outcome().unwrap(),
            repeated.view().outcome().unwrap()
        ));
        let page = stopped.view().history().page(1, 100, true).unwrap();
        assert_eq!(
            page.events().last().unwrap().r#type.as_str(),
            "ExecutionAborted"
        );
    }
    owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_keep_revisions_charged_until_last_snapshot_and_apply_tags_atomically() {
    let caps = RegistryCapacities::new(1, 2, 8, 8).unwrap();
    let limits = RegistryLimits::new(caps, Duration::from_secs(60), 8).unwrap();
    let (reg, mut owner) = registry(limits, None);
    let first = machine(&reg, 1, false).await;
    result(
        &reg,
        RegistryCommand::Update(arn(), Some(definition(2, false)), None),
    )
    .await
    .unwrap();
    assert!(matches!(
        result(
            &reg,
            RegistryCommand::Update(arn(), Some(definition(3, false)), None)
        )
        .await,
        Err(RegistryError::Capacity)
    ));
    drop(first);
    result(
        &reg,
        RegistryCommand::Update(arn(), Some(definition(3, false)), None),
    )
    .await
    .unwrap();
    let fifty = Tags::new((0..50).map(|n| (format!("key{n}"), "v".into())).collect()).unwrap();
    result(&reg, RegistryCommand::Tag(arn(), fifty))
        .await
        .unwrap();
    assert!(matches!(
        result(
            &reg,
            RegistryCommand::Tag(
                arn(),
                Tags::new(vec![("extra".into(), "v".into())]).unwrap()
            )
        )
        .await,
        Err(RegistryError::TagLimit)
    ));
    let RegistryResult::Machine(capture) =
        result(&reg, RegistryCommand::Machine(arn())).await.unwrap()
    else {
        panic!("machine")
    };
    assert_eq!(capture.tags().values().len(), 50);
    result(
        &reg,
        RegistryCommand::Untag(
            arn(),
            Tags::new(vec![("key0".into(), String::new())]).unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(capture.tags().values().len(), 50);
    owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_reject_scope_trace_and_registration_before_publishing_names() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    machine(&reg, 1, false).await;
    let wrong: StateMachineArn = "arn:aws:states:us-west-2:000000000000:stateMachine:Machine"
        .parse()
        .unwrap();
    assert!(matches!(
        reg.submit(RegistryCommand::Machine(wrong)),
        Err(RegistryError::Validation)
    ));
    assert!(matches!(
        reg.submit(RegistryCommand::Start(
            arn(),
            None,
            Payload::parse(b"{}").unwrap(),
            Some("中".into())
        )),
        Err(RegistryError::Validation)
    ));
    let task=PreparedDefinition::new(AslCompiler::new(TenantScope::default(),TaskSupport::Lambda).compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}}"#).unwrap().machine().clone());
    result(&reg, RegistryCommand::Update(arn(), Some(task), None))
        .await
        .unwrap();
    assert!(matches!(
        start(&reg, "free", "{}").await,
        Err(RegistryError::Execution(ExecutionError::Unavailable))
    ));
    result(
        &reg,
        RegistryCommand::Update(arn(), Some(definition(1, false)), None),
    )
    .await
    .unwrap();
    observe(start(&reg, "free", "{}").await.unwrap()).await;
    owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_hold_command_capacity_until_reply_observation_and_resume_receipt() {
    let limits =
        RegistryLimits::new(RegistryCapacities::default(), Duration::from_secs(60), 1).unwrap();
    let (reg, mut owner) = registry(limits, None);
    let mut receipt = reg.submit(RegistryCommand::Machines).unwrap();
    assert!(matches!(
        receipt.wait(Instant::now()).await,
        Err(RegistryError::Deadline)
    ));
    assert!(matches!(
        reg.submit(RegistryCommand::Machines),
        Err(RegistryError::Capacity)
    ));
    assert!(matches!(
        receipt.wait(deadline()).await.unwrap(),
        RegistryResult::Machines(_)
    ));
    assert!(matches!(
        receipt.wait(deadline()).await,
        Err(RegistryError::Unavailable)
    ));
    result(&reg, RegistryCommand::Machines).await.unwrap();
    owner.shutdown(deadline()).await.unwrap();
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
async fn test_should_resume_same_shutdown_while_actual_task_receipt_remains_owned() {
    let (tx, mut rx) = mpsc::channel(1);
    let (reg, mut owner) = registry(RegistryLimits::default(), Some(Arc::new(Port(tx))));
    let prepared=PreparedDefinition::new(AslCompiler::new(TenantScope::default(),TaskSupport::Lambda).compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}}"#).unwrap().machine().clone());
    result(
        &reg,
        RegistryCommand::Create(
            "Machine".parse().unwrap(),
            prepared,
            role(),
            Tags::default(),
        ),
    )
    .await
    .unwrap();
    start(&reg, "run", "{}").await.unwrap();
    let responder = timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        owner
            .shutdown(Instant::now() + Duration::from_millis(5))
            .await,
        Err(RegistryError::Deadline)
    ));
    assert!(matches!(
        reg.submit(RegistryCommand::Machines),
        Err(RegistryError::Unavailable)
    ));
    responder.finish(Ok(Payload::parse(b"{}").unwrap()));
    owner.shutdown(deadline()).await.unwrap();
    owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_preserve_root_allocations_after_abnormal_owner_abort() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let capture = machine(&reg, 1, false).await;
    let revision = Arc::downgrade(capture.revision());
    drop(capture);
    owner.join.as_ref().unwrap().abort();
    assert!(owner.shutdown(deadline()).await.is_err());
    assert!(revision.upgrade().is_some());
    assert!(matches!(
        reg.submit(RegistryCommand::Machines),
        Err(RegistryError::Unavailable)
    ));
}

#[tokio::test]
async fn test_should_keep_accepted_stop_after_lost_reply_and_preserve_context_metadata() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let machine = machine(&reg, 1, true).await;
    let execution = start(&reg, "stop-lost", "{}").await.unwrap();
    let receipt = reg
        .submit(RegistryCommand::Stop(
            execution.arn().clone(),
            StopReason::new(Some("caller-stop".into()), None).unwrap(),
        ))
        .unwrap();
    drop(receipt);
    let stopped = observe(execution).await;
    assert!(
        matches!(stopped.view().outcome().map(Arc::as_ref),Some(ExecutionOutcome::Aborted(f)) if f.details().error()==Some("caller-stop"))
    );
    let custom=PreparedDefinition::new(AslCompiler::default().compile(br#"{"StartAt":"P","States":{"P":{"Type":"Pass","Parameters":{"execution.$":"$$.Execution.Id","name.$":"$$.Execution.Name","machine.$":"$$.StateMachine.Id","role.$":"$$.Execution.RoleArn","input.$":"$$.Execution.Input"},"End":true}}}"#).unwrap().machine().clone());
    result(&reg, RegistryCommand::Update(arn(), Some(custom), None))
        .await
        .unwrap();
    let context = observe(start(&reg, "context", "{\"n\":7}").await.unwrap()).await;
    let Some(ExecutionOutcome::Succeeded(payload)) = context.view().outcome().map(Arc::as_ref)
    else {
        panic!("output")
    };
    assert_eq!(
        payload.value(),
        &json!({"execution":context.arn().as_str(),"name":"context","machine":machine.arn().as_str(),"role":role().as_str(),"input":{"n":7}})
    );
    owner.shutdown(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_require_present_machine_for_list_but_keep_deleted_execution_reads() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    assert!(matches!(
        result(&reg, RegistryCommand::Executions(arn())).await,
        Err(RegistryError::MachineMissing)
    ));
    machine(&reg, 1, false).await;
    let old = observe(start(&reg, "old", "{}").await.unwrap()).await;
    result(&reg, RegistryCommand::Delete(arn())).await.unwrap();
    assert!(matches!(
        result(&reg, RegistryCommand::Executions(arn())).await,
        Err(RegistryError::MachineMissing)
    ));
    assert!(matches!(
        result(&reg, RegistryCommand::Execution(old.arn().clone()))
            .await
            .unwrap(),
        RegistryResult::Execution(_)
    ));
    machine(&reg, 2, false).await;
    let RegistryResult::Executions(list) = result(&reg, RegistryCommand::Executions(arn()))
        .await
        .unwrap()
    else {
        panic!("list")
    };
    assert_eq!(list.len(), 1);
    assert_eq!(list.first().unwrap().resource_id(), old.resource_id());
    owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_bound_retained_tag_versions_without_copying_each_snapshot() {
    let caps = RegistryCapacities::new(1, 2, 8, 8).unwrap();
    let limits = RegistryLimits::new(caps, Duration::from_secs(60), 8).unwrap();
    let (reg, mut owner) = registry(limits, None);
    let old = machine(&reg, 1, false).await;
    let tags = |value: &str| Tags::new(vec![("key".into(), value.into())]).unwrap();
    result(&reg, RegistryCommand::Tag(arn(), tags("v2")))
        .await
        .unwrap();
    assert!(matches!(
        result(&reg, RegistryCommand::Tag(arn(), tags("v3"))).await,
        Err(RegistryError::Capacity)
    ));
    assert!(old.tags().values().is_empty());
    drop(old);
    result(&reg, RegistryCommand::Tag(arn(), tags("v3")))
        .await
        .unwrap();
    owner.shutdown(deadline()).await.unwrap();
}
#[tokio::test]
async fn test_should_signal_active_execution_when_registry_root_is_aborted() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    machine(&reg, 1, true).await;
    let execution = start(&reg, "cancel-root", "{}").await.unwrap();
    owner.join.as_ref().unwrap().abort();
    assert!(owner.shutdown(deadline()).await.is_err());
    let stopped = observe(execution).await;
    assert!(matches!(
        stopped.view().outcome().map(Arc::as_ref),
        Some(ExecutionOutcome::Aborted(_))
    ));
}

#[tokio::test]
async fn test_should_publish_role_only_revision_and_compare_exact_definition_source() {
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let first = machine(&reg, 1, false).await;
    let other: RoleArn = "arn:aws:iam::000000000000:role/Other".parse().unwrap();
    let RegistryResult::Machine(updated) = result(
        &reg,
        RegistryCommand::Update(arn(), None, Some(other.clone())),
    )
    .await
    .unwrap() else {
        panic!("machine")
    };
    assert_ne!(first.revision().id(), updated.revision().id());
    assert_eq!(
        first.revision().source_hash(),
        updated.revision().source_hash()
    );
    assert_eq!(updated.revision().role().as_str(), other.as_str());
    assert_ne!(first.revision().role().as_str(), other.as_str());
    let source = serde_json::to_vec_pretty(
        &serde_json::from_slice::<serde_json::Value>(first.revision().machine().original_bytes())
            .unwrap(),
    )
    .unwrap();
    let prepared = PreparedDefinition::new(
        AslCompiler::default()
            .compile(&source)
            .unwrap()
            .machine()
            .clone(),
    );
    assert!(matches!(
        result(
            &reg,
            RegistryCommand::Create(
                "Machine".parse().unwrap(),
                prepared,
                role(),
                Tags::default()
            )
        )
        .await,
        Err(RegistryError::MachineExists)
    ));
    let wrong: RoleArn = "arn:aws:iam::111111111111:role/Other".parse().unwrap();
    assert!(matches!(
        reg.submit(RegistryCommand::Update(arn(), None, Some(wrong))),
        Err(RegistryError::Validation)
    ));
    let prepared = PreparedDefinition::new(
        AslCompiler::new(
            TenantScope::new("aws", "us-west-2", "000000000000").unwrap(),
            TaskSupport::Disabled,
        )
        .compile(first.revision().machine().original_bytes())
        .unwrap()
        .machine()
        .clone(),
    );
    assert!(matches!(
        reg.submit(RegistryCommand::Update(arn(), Some(prepared), None)),
        Err(RegistryError::Validation)
    ));
    owner.shutdown(deadline()).await.unwrap();
}
#[test]
fn test_should_bound_operator_admission_limits_and_combined_execution_arn_length() {
    for values in [
        (0, 1, 1, 1),
        (1, 0, 1, 1),
        (1, 1, 0, 1),
        (1, 1, 1, 0),
        (4097, 1, 1, 1),
        (1, 4097, 1, 1),
        (1, 1, 100_001, 1),
        (1, 1, 1, 1_000_001),
    ] {
        assert!(RegistryCapacities::new(values.0, values.1, values.2, values.3).is_err());
    }
    for (seconds, commands) in [(59, 1), (90 * 86400 + 1, 1), (60, 0), (60, 1025)] {
        assert!(
            RegistryLimits::new(
                RegistryCapacities::default(),
                Duration::from_secs(seconds),
                commands
            )
            .is_err()
        );
    }
    let scope = TenantScope::new("aws", &"a".repeat(64), "000000000000").unwrap();
    let machine = StateMachineArn::new(&scope, "m".repeat(80).parse().unwrap()).unwrap();
    assert!(ExecutionArn::new(machine, "n".repeat(80).parse().unwrap()).is_err());
}

#[tokio::test]
async fn test_should_return_typed_failure_on_repeated_observation_of_closed_reply_lane() {
    let (sender, receiver) = oneshot::channel::<Reply>();
    drop(sender);
    let mut receipt = RegistryReceipt {
        receiver: Some(receiver),
    };
    for _ in 0..2 {
        assert!(matches!(
            receipt.wait(deadline()).await,
            Err(RegistryError::Unavailable)
        ));
    }
}

#[tokio::test]
async fn test_should_finish_accepted_task_after_all_registry_command_handles_are_dropped() {
    let (tx, mut rx) = mpsc::channel(1);
    let (reg, mut owner) = registry(RegistryLimits::default(), Some(Arc::new(Port(tx))));
    let prepared=PreparedDefinition::new(AslCompiler::new(TenantScope::default(),TaskSupport::Lambda).compile(br#"{"StartAt":"T","States":{"T":{"Type":"Task","Resource":"arn:aws:lambda:us-east-1:000000000000:function:Worker","End":true}}}"#).unwrap().machine().clone());
    result(
        &reg,
        RegistryCommand::Create(
            "Machine".parse().unwrap(),
            prepared,
            role(),
            Tags::default(),
        ),
    )
    .await
    .unwrap();
    let snapshot = start(&reg, "caller-independent", "{}").await.unwrap();
    let mut responder = timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    responder.started().unwrap();
    drop(reg);
    tokio::task::yield_now().await;
    assert!(!*owner.closing.borrow());
    responder.finish(Ok(Payload::parse(b"42").unwrap()));
    let finished = observe(snapshot).await;
    assert!(
        matches!(finished.view().outcome().map(Arc::as_ref),Some(ExecutionOutcome::Succeeded(p)) if p.value()==&json!(42))
    );
    owner.shutdown(deadline()).await.unwrap();
}

#[tokio::test]
async fn test_should_reject_nested_artifact_promotion_during_create_and_update()
-> Result<(), Box<dyn Error>> {
    let compiler = AslCompiler::default().with_state_support(StateSupport::Structured);
    let (reg, mut owner) = registry(RegistryLimits::default(), None);
    let first = machine(&reg, 1, false).await;
    for state in [
        json!({"Type":"Parallel","Branches":[{"StartAt":"Child","States":{"Child":{"Type":"Succeed"}}}],"End":true}),
        json!({"Type":"Map","ItemProcessor":{"StartAt":"Child","States":{"Child":{"Type":"Succeed"}}},"End":true}),
    ] {
        let root = compiler.compile(&serde_json::to_vec(
            &json!({"StartAt":"Run","States":{"Run":state}}),
        )?)?;
        assert!(root.machine().is_root_definition());
        let child = match root.machine().state(root.machine().start())? {
            StateView::Parallel { branches, .. } => {
                branches.first().ok_or("missing branch")?.clone()
            }
            StateView::Map { processor, .. } => processor.clone(),
            _ => return Err("missing structured state".into()),
        };
        assert!(!child.is_root_definition());
        assert!(!child.requires_structured_execution());
        assert_eq!(child.original_bytes(), root.machine().original_bytes());
        let prepared = PreparedDefinition::new(child);
        assert!(matches!(
            reg.submit(RegistryCommand::Create(
                "Injected".parse()?,
                prepared.clone(),
                role(),
                Tags::default(),
            )),
            Err(RegistryError::Validation)
        ));
        assert!(matches!(
            reg.submit(RegistryCommand::Update(arn(), Some(prepared), None)),
            Err(RegistryError::Validation)
        ));
    }
    assert!(matches!(
        result(
            &reg,
            RegistryCommand::Machine(
                "arn:aws:states:us-east-1:000000000000:stateMachine:Injected".parse()?
            )
        )
        .await,
        Err(RegistryError::MachineMissing)
    ));
    let RegistryResult::Machine(current) = result(&reg, RegistryCommand::Machine(arn())).await?
    else {
        return Err("missing current machine".into());
    };
    assert_eq!(current.revision().id(), first.revision().id());
    assert!(Arc::ptr_eq(current.revision(), first.revision()));
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_bind_structured_publication_and_start_to_installed_profile()
-> Result<(), Box<dyn Error>> {
    let compiled = AslCompiler::default().with_state_support(StateSupport::Structured)
        .compile(br#"{"StartAt":"P","States":{"P":{"Type":"Parallel","Branches":[{"StartAt":"D","States":{"D":{"Type":"Pass","Result":1,"End":true}}},{"StartAt":"D","States":{"D":{"Type":"Pass","Result":2,"End":true}}}],"End":true}}}"#)?;
    let prepared = PreparedDefinition::new(compiled.machine().clone());
    let (sequential, mut old_owner) = registry(RegistryLimits::default(), None);
    assert_eq!(sequential.state_support(), StateSupport::Sequential);
    assert!(matches!(
        sequential.submit(RegistryCommand::Create(
            "Machine".parse()?,
            prepared.clone(),
            role(),
            Tags::default()
        )),
        Err(RegistryError::Validation)
    ));
    machine(&sequential, 7, false).await;
    assert!(matches!(
        sequential.submit(RegistryCommand::Update(arn(), Some(prepared.clone()), None)),
        Err(RegistryError::Validation)
    ));
    let old = start(&sequential, "unchanged", "{}").await?;
    let old = observe(old).await;
    assert!(
        matches!(old.view().outcome().map(Arc::as_ref),Some(ExecutionOutcome::Succeeded(value)) if value.value()==&json!(7))
    );
    old_owner.shutdown(deadline()).await?;
    let runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
    )?;
    let (structured, mut owner) = Registry::new_with_state_support(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
        StateSupport::Structured,
    )?;
    assert_eq!(structured.state_support(), StateSupport::Structured);
    result(
        &structured,
        RegistryCommand::Create("Machine".parse()?, prepared, role(), Tags::default()),
    )
    .await?;
    let completed = observe(start(&structured, "structured", "{}").await?).await;
    assert!(
        matches!(completed.view().outcome().map(Arc::as_ref),Some(ExecutionOutcome::Succeeded(value)) if value.value()==&json!([1,2]))
    );
    owner.shutdown(deadline()).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_rollback_record_and_name_when_shared_root_frame_admission_is_full()
-> Result<(), Box<dyn Error>> {
    let capacity = Arc::new(FrameCapacity::new(1, 1024 * 1024)?);
    let runtime = ExecutionSupervisor::new_with_frames(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        None,
        capacity.clone(),
        FrameLimits::new(1, 1024 * 1024)?,
    )?;
    let (reg, mut owner) = Registry::new_with_state_support(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
        StateSupport::Structured,
    )?;
    machine(&reg, 1, true).await;
    let first = start(&reg, "first", "{}").await?;
    assert_eq!(capacity.available_frames(), 0);
    result(
        &reg,
        RegistryCommand::Update(arn(), Some(definition(7, false)), None),
    )
    .await?;
    assert!(matches!(
        start(&reg, "retry-same-name", "{}").await,
        Err(RegistryError::Execution(ExecutionError::Capacity))
    ));
    let RegistryResult::Executions(records) =
        result(&reg, RegistryCommand::Executions(arn())).await?
    else {
        return Err("missing records".into());
    };
    assert_eq!(records.len(), 1);
    reg.stop(first.arn().clone(), None, None, deadline())
        .await?;
    reg.wait_idle(deadline()).await?;
    let completed = observe(start(&reg, "retry-same-name", "{}").await?).await;
    assert!(
        matches!(completed.view().outcome().map(Arc::as_ref),Some(ExecutionOutcome::Succeeded(value)) if value.value()==&json!(7))
    );
    owner.shutdown(deadline()).await?;
    assert_eq!(capacity.available_frames(), 1);
    Ok(())
}
