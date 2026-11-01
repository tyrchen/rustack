//! Reject executable publication until its actual callback owner is installed.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::{
    ExecutionError, ExecutionSupervisor, HistoryLimits, LambdaTaskCommand, LambdaTaskPort,
    PreparedDefinition, Registry, RegistryCommand, RegistryError, RegistryLimits, RegistryResult,
    RoleArn, RuntimeLimits, Tags, TaskCall,
};

#[derive(Debug, Default)]
struct Port(AtomicUsize);
impl LambdaTaskPort for Port {
    fn start(&self, _command: LambdaTaskCommand) -> Result<TaskCall, TaskFailure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(TaskFailure::service_exception())
    }
}
#[tokio::test]
async fn test_should_reject_callback_runtime_and_registry_admission_before_any_issuance() {
    let port = Arc::new(Port::default());
    let mut runtime = ExecutionSupervisor::new(
        RuntimeLimits::default(),
        HistoryLimits::default(),
        256 * 1024 * 1024,
        Some(port.clone()),
    )
    .unwrap();
    let source = definition(json!({}));
    let (driver, _) = driver(&source, &json!({"n":21}));
    assert!(matches!(
        runtime.start(driver.clone()),
        Err(ExecutionError::Unavailable)
    ));
    assert!(matches!(
        runtime.start_structured(driver),
        Err(ExecutionError::Unavailable)
    ));
    let (registry, mut owner) = Registry::new_with_state_support(
        TenantScope::default(),
        RegistryLimits::default(),
        runtime,
        ExecutionLimits::default(),
        StateSupport::Structured,
    )
    .unwrap();
    let role: RoleArn = "arn:aws:iam::000000000000:role/Test".parse().unwrap();
    for compiled in [
        compile(&source),
        compile(
            &json!({"StartAt":"M","States":{"M":{"Type":"Map","ItemProcessor":source,"End":true}}}),
        ),
    ] {
        assert!(matches!(
            registry.submit(RegistryCommand::Create(
                "Rejected".parse().unwrap(),
                PreparedDefinition::new(compiled),
                role.clone(),
                Tags::default()
            )),
            Err(RegistryError::Validation)
        ));
    }
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    let RegistryResult::Machine(original) = registry
        .submit(RegistryCommand::Create(
            "Present".parse().unwrap(),
            PreparedDefinition::new(compile(
                &json!({"StartAt":"Done","States":{"Done":{"Type":"Succeed"}}}),
            )),
            role,
            Tags::default(),
        ))
        .unwrap()
        .wait(until)
        .await
        .unwrap()
    else {
        panic!("machine");
    };
    assert!(matches!(
        registry.submit(RegistryCommand::Update(
            original.arn().clone(),
            Some(PreparedDefinition::new(compile(&source))),
            None
        )),
        Err(RegistryError::Validation)
    ));
    let RegistryResult::Machine(after) = registry
        .submit(RegistryCommand::Machine(original.arn().clone()))
        .unwrap()
        .wait(until)
        .await
        .unwrap()
    else {
        panic!("machine");
    };
    assert_eq!(original.revision().id(), after.revision().id());
    assert_eq!(port.0.load(Ordering::SeqCst), 0);
    owner.shutdown(until).await.unwrap();
}
