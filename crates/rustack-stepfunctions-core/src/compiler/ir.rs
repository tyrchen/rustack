//! Immutable typed state storage and checked borrowed execution views.

use std::{fmt, num::NonZeroU32, sync::Arc};

use bytes::Bytes;
use serde_json::Value;

use super::{Catcher, Diagnostic, Integration, RecoveryPolicy};
use crate::{AslError, DataFlow, Expression, Predicate, ReferencePath, ResultPath, WaitSource};

/// State index constructible only by the compiler; interpreted in its machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct StateId(pub(super) usize);

/// Pre-resolved sequential control transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transition {
    /// Continue within the compiled scope.
    Next(StateId),
    /// Complete this machine successfully.
    End,
}

/// Prevalidated per-attempt timeout source.
#[derive(Clone)]
#[non_exhaustive]
pub enum TaskTimeout {
    /// Bounded only by the execution/operator deadline.
    Omitted,
    /// Positive static seconds.
    Seconds(NonZeroU32),
    /// Singular dynamic seconds resolved on entry.
    Path(ReferencePath),
}
impl fmt::Debug for TaskTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TaskTimeout(<redacted>)")
    }
}

/// Failure field view preserving omission, literal text or compiled expression.
#[non_exhaustive]
#[derive(Clone, Copy)]
pub enum FailureView<'a> {
    /// No value was supplied.
    Omitted,
    /// Bounded literal field.
    Literal(&'a str),
    /// Must evaluate to a bounded string on entry.
    Dynamic(&'a Expression),
}
impl fmt::Debug for FailureView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FailureView(<redacted>)")
    }
}

/// Catch error-object merge behavior.
#[non_exhaustive]
pub enum CatchResult<'a> {
    /// Replace the failed state's raw input with its error object.
    Replace,
    /// Preserve its original raw input.
    Discard,
    /// Merge into the original input through a validated data-root reference.
    Path(&'a ResultPath),
}
impl fmt::Debug for CatchResult<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CatchResult(<redacted>)")
    }
}

/// Borrowed validated state instruction; execution does not interpret ASL maps.
#[non_exhaustive]
pub enum StateView<'a> {
    /// Pass with a possible static result and resolved continuation.
    Pass {
        /// Compiled data processing.
        flow: &'a DataFlow,
        /// Literal result, if supplied.
        result: Option<&'a Value>,
        /// Pre-resolved continuation.
        next: Transition,
    },
    /// Actual Lambda call plan; the executor owns invocation lifetime.
    Task {
        /// Compiled data processing.
        flow: &'a DataFlow,
        /// Validated integration.
        resource: &'a Integration,
        /// Per-attempt timeout source.
        timeout: &'a TaskTimeout,
        /// Ordered recovery rules.
        recovery: &'a RecoveryPolicy,
        /// Pre-resolved continuation.
        next: Transition,
    },
    /// Ordered conditional branches with optional fallback.
    Choice {
        /// Input/output processing only.
        flow: &'a DataFlow,
        /// Compiled predicates and targets.
        branches: &'a [(Predicate, StateId)],
        /// Optional default target.
        default: Option<StateId>,
    },
    /// Wait plan; timers remain actor-owned.
    Wait {
        /// Input/output processing.
        flow: &'a DataFlow,
        /// Compiled wait source.
        source: &'a WaitSource,
        /// Pre-resolved continuation.
        next: Transition,
    },
    /// Successful terminal with input/output transforms.
    Succeed {
        /// Compiled input/output processing.
        flow: &'a DataFlow,
    },
    /// Failed terminal with independent bounded fields.
    Fail {
        /// Error field.
        error: FailureView<'a>,
        /// Cause field.
        cause: FailureView<'a>,
    },
}
impl fmt::Debug for StateView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StateView(<redacted>)")
    }
}

pub(super) enum Failure {
    Omitted,
    Literal(String),
    Dynamic(Expression),
}
impl Failure {
    fn view(&self) -> FailureView<'_> {
        match self {
            Self::Omitted => FailureView::Omitted,
            Self::Literal(v) => FailureView::Literal(v),
            Self::Dynamic(v) => FailureView::Dynamic(v),
        }
    }
    fn units(&self) -> usize {
        match self {
            Self::Dynamic(v) => v.node_count(),
            Self::Omitted | Self::Literal(_) => 1,
        }
    }
}

pub(super) enum Node {
    Pass {
        flow: DataFlow,
        result: Option<Arc<Value>>,
        next: Transition,
    },
    Task {
        flow: DataFlow,
        resource: Integration,
        timeout: TaskTimeout,
        recovery: RecoveryPolicy,
        next: Transition,
    },
    Choice {
        flow: DataFlow,
        branches: Vec<(Predicate, StateId)>,
        default: Option<StateId>,
    },
    Wait {
        flow: DataFlow,
        source: WaitSource,
        next: Transition,
    },
    Succeed(DataFlow),
    Fail {
        error: Failure,
        cause: Failure,
    },
}
impl Node {
    fn view(&self) -> StateView<'_> {
        match self {
            Self::Pass { flow, result, next } => StateView::Pass {
                flow,
                result: result.as_deref(),
                next: *next,
            },
            Self::Task {
                flow,
                resource,
                timeout,
                recovery,
                next,
            } => StateView::Task {
                flow,
                resource,
                timeout,
                recovery,
                next: *next,
            },
            Self::Choice {
                flow,
                branches,
                default,
            } => StateView::Choice {
                flow,
                branches,
                default: *default,
            },
            Self::Wait { flow, source, next } => StateView::Wait {
                flow,
                source,
                next: *next,
            },
            Self::Succeed(flow) => StateView::Succeed { flow },
            Self::Fail { error, cause } => StateView::Fail {
                error: error.view(),
                cause: cause.view(),
            },
        }
    }
    pub(super) fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeed(_)
                | Self::Fail { .. }
                | Self::Pass {
                    next: Transition::End,
                    ..
                }
                | Self::Task {
                    next: Transition::End,
                    ..
                }
                | Self::Wait {
                    next: Transition::End,
                    ..
                }
        )
    }
    pub(super) fn edges(&self) -> Vec<StateId> {
        let mut result = Vec::new();
        let transition = match self {
            Self::Pass { next, .. } | Self::Wait { next, .. } => Some(*next),
            Self::Task { next, recovery, .. } => {
                result.extend(recovery.catches().iter().map(Catcher::next));
                Some(*next)
            }
            Self::Choice {
                branches, default, ..
            } => {
                result.extend(branches.iter().map(|(_, id)| *id));
                result.extend(default);
                None
            }
            Self::Succeed(_) | Self::Fail { .. } => None,
        };
        if let Some(Transition::Next(id)) = transition {
            result.push(id);
        }
        result
    }
    pub(super) fn units(&self) -> usize {
        let units = match self {
            Self::Pass { flow, .. } | Self::Wait { flow, .. } | Self::Succeed(flow) => {
                flow.node_count()
            }
            Self::Task { flow, recovery, .. } => flow.node_count().saturating_add(recovery.units()),
            Self::Choice { flow, branches, .. } => branches
                .iter()
                .fold(flow.node_count(), |sum, (predicate, _)| {
                    sum.saturating_add(predicate.node_count()).saturating_add(1)
                }),
            Self::Fail { error, cause } => error.units().saturating_add(cause.units()),
        };
        units.saturating_add(1)
    }
}
pub(super) struct Row {
    pub(super) name: String,
    pub(super) node: Node,
}

/// Immutable original-byte definition and indexed sequential instructions.
#[non_exhaustive]
pub struct CompiledMachine {
    pub(super) original: Bytes,
    pub(super) rows: Vec<Row>,
    pub(super) start: StateId,
    pub(super) timeout: Option<NonZeroU32>,
}
impl fmt::Debug for CompiledMachine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompiledMachine(<redacted>)")
    }
}
impl CompiledMachine {
    read!(CompiledMachine,start,StateId,s=>s.start,"Read the validated entry ID.");
    read!(CompiledMachine,state_count,usize,s=>s.rows.len(),"Read the bounded number of compiled states.");
    read!(CompiledMachine,original_bytes,&[u8],s=>s.original.as_ref(),"Read the exact successfully compiled source bytes.");
    read!(CompiledMachine,timeout_seconds,Option<NonZeroU32>,s=>s.timeout,"Read the optional positive machine timeout.");
    /// Read a validated instruction through a checked machine-local ID.
    ///
    /// # Errors
    /// Returns `InvalidStateId` for an ID outside this machine's index.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{AslCompiler,StateView};
    /// let c=AslCompiler::default().compile(br#"{"StartAt":"D","States":{"D":{"Type":"Succeed"}}}"#)?;
    /// assert!(matches!(c.machine().state(c.machine().start())?,StateView::Succeed {..}));
    /// # Ok::<(),Box<dyn std::error::Error>>(())
    /// ```
    pub fn state(&self, id: StateId) -> Result<StateView<'_>, AslError> {
        self.rows
            .get(id.0)
            .map(|row| row.node.view())
            .ok_or(AslError::InvalidStateId)
    }
    /// Read the state name associated with a checked ID.
    ///
    /// # Errors
    /// Returns `InvalidStateId` when the ID is outside this machine.
    ///
    /// ```
    /// # fn name(machine:&rustack_stepfunctions_core::CompiledMachine,id:rustack_stepfunctions_core::StateId) {
    /// let name=machine.state_name(id);
    /// # }
    /// ```
    pub fn state_name(&self, id: StateId) -> Result<&str, AslError> {
        self.rows
            .get(id.0)
            .map(|row| row.name.as_str())
            .ok_or(AslError::InvalidStateId)
    }
}

/// Successful immutable machine plus nonfatal structural diagnostics.
#[non_exhaustive]
pub struct Compilation {
    pub(super) machine: Arc<CompiledMachine>,
    pub(super) warnings: Vec<Diagnostic>,
}
impl fmt::Debug for Compilation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Compilation(<redacted>)")
    }
}
impl Compilation {
    read!(Compilation,machine,&Arc<CompiledMachine>,s=>&s.machine,"Borrow the immutable artifact for revision capture.");
    read!(Compilation,warnings,&[Diagnostic],s=>s.warnings.as_slice(),"Read bounded unreachable-state warnings.");
}
