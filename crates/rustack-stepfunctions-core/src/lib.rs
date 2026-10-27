//! Bounded ASL compilation and supervised local workflow execution.
//!
//! Immutable payloads and scoped IR feed actor-owned execution/history. Concrete
//! task ports keep Lambda/runtime ownership outside this crate; there is no SDK,
//! Lambda-core, Docker or HTTP dependency. The actor-owned registry exposes validated domain
//! commands; HTTP preparation, pagination and main-service composition remain the embedding
//! application's responsibility.

#![forbid(unsafe_code)]

mod callback_parameters;
mod choice;
mod compiler;
mod dataflow;
mod engine;
mod error;
mod expression;
mod path;
mod payload;
mod registry;
mod task;
mod template;
mod wait;

pub use callback_parameters::{CallbackHistory, CallbackParameters};
pub use choice::Predicate;
pub use compiler::{
    AslCompiler, CatchResult, Catcher, Compilation, CompileError, CompiledMachine, Diagnostic,
    DiagnosticSeverity, FailureView, FunctionArn, Integration, Jitter, MapConcurrency,
    RecoveryPolicy, Retrier, RetryTiming, StateId, StateSupport, StateView, TaskSupport,
    TaskTimeout, TenantScope, Transition,
};
pub use dataflow::DataFlow;
pub use engine::{
    AttemptId, CallbackDispatch, CleanupState, ClockSample, DriverStep, DriverUpdate,
    ExecutionDriver, ExecutionError, ExecutionFailure, ExecutionHandle, ExecutionHistory,
    ExecutionLimits, ExecutionOutcome, ExecutionSource, ExecutionSupervisor, ExecutionView,
    FanoutBranch, FanoutFailure, FanoutPlan, FrameCallId, FrameCapacity, FrameError, FrameId,
    FrameLease, FrameLimits, FrameParent, FrameScheduler, FrameTransition, FrameUpdate,
    HistoryLimits, HistoryPage, PropagatedFailure, RuntimeLimits, TaskDispatch, TaskFailure,
};
pub use error::AslError;
pub use expression::{Expression, SystemUuidSource, UuidSource};
pub use path::{MatchKind, Path, PathMatches, ReferencePath, ResultPath};
pub use payload::{EvaluationContext, MAX_PAYLOAD_BYTES, Payload};
pub use registry::{
    CallbackAck, CallbackAttempt, CallbackCommand, CallbackError, CallbackInvocation,
    CallbackInvocationMetadata, CallbackInvocationReporter, CallbackLimits, CallbackMint,
    CallbackOutcome, CallbackRegistry, CallbackStatus, CallbackSupervisor, CallbackTask,
    CallbackTimeout, CallbackToken, ChildError, ChildExecution, ChildExecutionPort, ChildParent,
    ChildPattern, ChildStartParameters, ChildSubmission, DefinitionRevision, ExecutionArn,
    ExecutionName, ExecutionSnapshot, MachineSnapshot, MachineStatus, PreparedDefinition,
    PreparedSnapshot, Registry, RegistryCapacities, RegistryCommand, RegistryError, RegistryLimits,
    RegistryReceipt, RegistryResult, RegistrySnapshot, RegistryStatus, RegistrySupervisor, RoleArn,
    SnapshotLimits, StateMachineArn, StateMachineName, StopReason, Tags,
};
pub use task::{
    LambdaTaskCommand, LambdaTaskPort, QuarantinedTask, TaskCall, TaskChannelError, TaskEvent,
    TaskResponder, TaskSettlement,
};
pub use template::Template;
pub use wait::WaitSource;
