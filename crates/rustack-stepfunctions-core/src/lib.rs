//! Pure, bounded ASL payload and compiled-path semantics.
//!
//! This crate has no network, SDK, Lambda or HTTP dependency. Immutable payloads
//! preserve source bytes and compiled paths distinguish missing values from null.
//! No registry or execution-service capability is implied by these data APIs.

#![forbid(unsafe_code)]

mod choice;
mod compiler;
mod dataflow;
mod engine;
mod error;
mod expression;
mod path;
mod payload;
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
    AttemptId, ClockSample, DriverStep, DriverUpdate, ExecutionDriver, ExecutionFailure,
    ExecutionLimits, ExecutionOutcome, ExecutionSource, TaskDispatch, TaskFailure,
};
pub use error::AslError;
pub use expression::{Expression, SystemUuidSource, UuidSource};
pub use path::{MatchKind, Path, PathMatches, ReferencePath, ResultPath};
pub use payload::{EvaluationContext, MAX_PAYLOAD_BYTES, Payload};
pub use template::Template;
pub use wait::WaitSource;
