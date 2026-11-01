//! Cancellation/deadline/close remain live while unrelated entropy work is gated.

use std::error::Error;

use super::*;
use crate::{ChildError, ChildPattern, ChildStartParameters, Registry};

type TestResult = Result<(), Box<dyn Error>>;
fn setup() -> Result<(Owner, Registry, ExecutionSnapshot), Box<dyn Error>> {
    let mut owned = owner(RegistryCapacities::default(), None);
    let state = owned.state.as_mut().ok_or("state")?;
    let prepared = PreparedDefinition::new(
        AslCompiler::default()
            .compile(br#"{"StartAt":"W","States":{"W":{"Type":"Wait","Seconds":60,"End":true}}}"#)?
            .machine()
            .clone(),
    );
    let machine = create(state, prepared);
    let parent = start(state, &machine, "parent")?;
    let registry = Registry {
        shared: state.children.registry.clone(),
    };
    Ok((owned, registry, parent))
}
fn arguments(
    parent: &ExecutionSnapshot,
    name: &str,
) -> Result<ChildStartParameters, Box<dyn Error>> {
    Ok(ChildStartParameters::parse(
        &TenantScope::default(),
        &Payload::from_value(
            &json!({"StateMachineArn":parent.arn().machine().as_str(),"Name":name,"Input":{}}),
        )?,
    )?)
}
#[tokio::test]
async fn test_should_serve_owned_child_control_deadline_and_close_before_entropy_is_released()
-> TestResult {
    for mode in ["cancel", "deadline", "close"] {
        let (mut owned, registry, parent) = setup()?;
        let (entered, entered_wait) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let state = owned.state.as_mut().ok_or("state")?;
        state.io.entropy_hook = Some(EntropyHook { entered, gate });
        let stop = state.io.closing.subscribe();
        let join = tokio::spawn(owned.run(stop));
        let absolute =
            Instant::now() + Duration::from_secs(if mode == "deadline" { 1 } else { 10 });
        let mut submission = registry.child_port().start(
            parent.child_parent(),
            arguments(&parent, "child")?,
            ChildPattern::Sync2,
            absolute,
        )?;
        submission
            .admitted(Instant::now() + Duration::from_secs(5))
            .await?;
        let mut child = submission.into_execution()?;
        let mut update = registry.submit(RegistryCommand::Update(
            parent.arn().machine().clone(),
            None,
            Some("arn:aws:iam::000000000000:role/New".parse()?),
        ))?;
        tokio::time::timeout(Duration::from_secs(5), entered_wait).await??;
        match mode {
            "cancel" => child.cancel(),
            "close" => {
                registry.shared.closing.send_replace(true);
            }
            _ => {}
        }
        let view = child.wait(Instant::now() + Duration::from_secs(3)).await?;
        assert!(matches!(
            view.outcome().map(Arc::as_ref),
            Some(crate::ExecutionOutcome::Aborted(_))
        ));
        child
            .wait_cleanup(Instant::now() + Duration::from_secs(3))
            .await?;
        assert!(
            !join.is_finished(),
            "entropy join was lost while handling child controls"
        );
        release.send(())?;
        update.wait(Instant::now() + Duration::from_secs(5)).await?;
        registry.shared.closing.send_replace(true);
        tokio::time::timeout(Duration::from_secs(5), join).await???;
    }
    Ok(())
}
#[tokio::test]
async fn test_should_bound_child_lane_burst_with_an_ordinary_read_already_accepted() -> TestResult {
    let (owned, registry, parent) = setup()?;
    let mut owned = owned;
    let mut first = registry.child_port().start(
        parent.child_parent(),
        arguments(&parent, "first")?,
        ChildPattern::RequestResponse,
        Instant::now() + Duration::from_secs(10),
    )?;
    let second = registry.child_port().start(
        parent.child_parent(),
        arguments(&parent, "second")?,
        ChildPattern::RequestResponse,
        Instant::now() + Duration::from_secs(10),
    )?;
    let mut ordinary = registry.submit(RegistryCommand::Machines)?;
    let state = owned.state.as_mut().ok_or("state")?;
    let Incoming::Child(Some(request)) = receive(
        &mut state.io.receiver,
        &mut state.io.children,
        true,
        true,
        true,
    )
    .await
    else {
        return Err("child preference did not take one turn".into());
    };
    drop(request);
    let third = registry.child_port().start(
        parent.child_parent(),
        arguments(&parent, "third")?,
        ChildPattern::RequestResponse,
        Instant::now() + Duration::from_secs(10),
    )?;
    let state = owned.state.as_mut().ok_or("state")?;
    assert!(!state.io.children.is_empty());
    let Incoming::Command(Some(envelope)) = receive(
        &mut state.io.receiver,
        &mut state.io.children,
        false,
        true,
        true,
    )
    .await
    else {
        return Err("ready child stream starved ordinary intake".into());
    };
    reply(envelope, Ok(RegistryResult::Machines(Vec::new())));
    assert!(matches!(
        ordinary
            .wait(Instant::now() + Duration::from_secs(1))
            .await?,
        RegistryResult::Machines(_)
    ));
    let failure = first
        .admitted(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        matches!(failure,ChildError::Admission(source) if matches!(source.as_ref(),ChildError::OwnerLost))
    );
    registry.shared.closing.send_replace(true);
    let stop = registry.shared.closing.subscribe();
    owned.run(stop).await?;
    drop((second, third));
    Ok(())
}
