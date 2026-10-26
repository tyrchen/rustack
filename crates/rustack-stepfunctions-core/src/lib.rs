//! Bounded ASL compilation and supervised local workflow execution.
//!
//! Immutable payloads and scoped IR feed actor-owned execution/history. Concrete
//! task ports keep Lambda/runtime ownership outside this crate; there is no SDK,
//! Lambda-core, Docker or HTTP dependency. Registry and main-service registration
//! remain the embedding application's responsibility.

#![forbid(unsafe_code)]

mod choice;
mod compiler;
mod dataflow;
mod engine;
mod error;
mod expression;
mod path;
mod payload;
mod task;
mod template;
mod wait;

pub use choice::Predicate;
pub use compiler::{
    AslCompiler, CatchResult, Catcher, Compilation, CompileError, CompiledMachine, Diagnostic,
    DiagnosticSeverity, FailureView, FunctionArn, Integration, Jitter, RecoveryPolicy, Retrier,
    RetryTiming, StateId, StateView, TaskSupport, TaskTimeout, TenantScope, Transition,
};
pub use dataflow::DataFlow;
pub use engine::{
    AttemptId, CleanupState, ClockSample, DriverStep, DriverUpdate, ExecutionDriver,
    ExecutionError, ExecutionFailure, ExecutionHandle, ExecutionHistory, ExecutionLimits,
    ExecutionOutcome, ExecutionSource, ExecutionSupervisor, ExecutionView, HistoryLimits,
    HistoryPage, RuntimeLimits, TaskDispatch, TaskFailure,
};
pub use error::AslError;
pub use expression::{Expression, SystemUuidSource, UuidSource};
pub use path::{MatchKind, Path, PathMatches, ReferencePath, ResultPath};
pub use payload::{EvaluationContext, MAX_PAYLOAD_BYTES, Payload};
pub use task::{
    LambdaTaskCommand, LambdaTaskPort, QuarantinedTask, TaskCall, TaskChannelError, TaskEvent,
    TaskResponder, TaskSettlement,
};
pub use template::Template;
pub use wait::WaitSource;
