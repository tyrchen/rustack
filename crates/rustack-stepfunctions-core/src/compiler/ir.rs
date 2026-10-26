//! Immutable typed state storage and checked borrowed execution views.

use std::{
    fmt,
    num::{NonZeroU8, NonZeroU32},
    sync::Arc,
};

use bytes::Bytes;
use serde_json::Value;

use super::{Catcher, Diagnostic, Integration, RecoveryPolicy, TenantScope};
use crate::{
    AslError, DataFlow, Expression, Predicate, ReferencePath, ResultPath, Template, WaitSource,
};

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

/// Normalized Inline Map concurrency, independent of task/frame capacity.
#[derive(Clone)]
#[non_exhaustive]
pub enum MapConcurrency {
    /// Positive static limit; omission and zero normalize to forty.
    Limit(NonZeroU8),
    /// Singular reference resolved from the effective Map input on entry.
    Path(ReferencePath),
}
impl fmt::Debug for MapConcurrency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MapConcurrency(<redacted>)")
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
    /// Independent branch machines sharing this state's effective input.
    Parallel {
        /// Compiled input and result processing.
        flow: &'a DataFlow,
        /// Branches in definition/result order, each with a lexical state index.
        branches: &'a [Arc<CompiledMachine>],
        /// Ordered enclosing-state recovery.
        recovery: &'a RecoveryPolicy,
        /// Continuation in the enclosing machine.
        next: Transition,
    },
    /// Normalized Inline Map instruction with one reusable processor artifact.
    Map {
        /// Input/output/result processing; excludes legacy selector Parameters.
        flow: &'a DataFlow,
        /// Array source, defaulting to the effective input root.
        items: &'a ReferencePath,
        /// Item input template evaluated with Map.Item context, when supplied.
        selector: Option<&'a Template>,
        /// Immutable processor with an independent lexical index.
        processor: &'a Arc<CompiledMachine>,
        /// Normalized local concurrency plan.
        concurrency: &'a MapConcurrency,
        /// Ordered enclosing-state recovery.
        recovery: &'a RecoveryPolicy,
        /// Continuation in the enclosing machine.
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
    Parallel {
        flow: DataFlow,
        branches: Vec<Arc<CompiledMachine>>,
        recovery: RecoveryPolicy,
        next: Transition,
    },
    Map {
        flow: DataFlow,
        items: ReferencePath,
        selector: Option<Template>,
        processor: Arc<CompiledMachine>,
        concurrency: MapConcurrency,
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
            Self::Parallel {
                flow,
                branches,
                recovery,
                next,
            } => StateView::Parallel {
                flow,
                branches,
                recovery,
                next: *next,
            },
            Self::Map {
                flow,
                items,
                selector,
                processor,
                concurrency,
                recovery,
                next,
            } => StateView::Map {
                flow,
                items,
                selector: selector.as_ref(),
                processor,
                concurrency,
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
                | Self::Parallel {
                    next: Transition::End,
                    ..
                }
                | Self::Map {
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
            Self::Task { next, recovery, .. }
            | Self::Parallel { next, recovery, .. }
            | Self::Map { next, recovery, .. } => {
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
            Self::Task { flow, recovery, .. } | Self::Parallel { flow, recovery, .. } => {
                flow.node_count().saturating_add(recovery.units())
            }
            Self::Map {
                flow,
                selector,
                recovery,
                ..
            } => flow
                .node_count()
                .saturating_add(recovery.units())
                .saturating_add(selector.as_ref().map_or(0, Template::node_count)),
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

/// Immutable root-source bytes and indexed instructions in one lexical scope.
#[non_exhaustive]
pub struct CompiledMachine {
    pub(super) original: Bytes,
    pub(super) definition_root: bool,
    pub(super) rows: Vec<Row>,
    pub(super) start: StateId,
    pub(super) timeout: Option<NonZeroU32>,
    pub(super) scope: TenantScope,
}
impl fmt::Debug for CompiledMachine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompiledMachine(<redacted>)")
    }
}
impl CompiledMachine {
    /// Whether this artifact represents the complete original definition.
    pub(crate) fn is_root_definition(&self) -> bool {
        self.definition_root
    }

    pub(crate) fn uses_lambda(&self) -> bool {
        self.rows.iter().any(|row| match &row.node {
            Node::Task { resource, .. } => matches!(
                resource,
                Integration::DirectLambda(_) | Integration::LambdaInvoke
            ),
            Node::Parallel { branches, .. } => branches.iter().any(|branch| branch.uses_lambda()),
            Node::Map { processor, .. } => processor.uses_lambda(),
            _ => false,
        })
    }

    pub(crate) fn uses_children(&self) -> bool {
        self.rows.iter().any(|row| match &row.node {
            Node::Task { resource, .. } => matches!(resource, Integration::StartExecution(_)),
            Node::Parallel { branches, .. } => branches.iter().any(|branch| branch.uses_children()),
            Node::Map { processor, .. } => processor.uses_children(),
            _ => false,
        })
    }

    /// Whether this scope requires a structured frame owner for execution.
    ///
    /// ```
    /// # fn inspect(machine: &rustack_stepfunctions_core::CompiledMachine) {
    /// let required = machine.requires_structured_execution();
    /// # }
    /// ```
    #[must_use]
    pub fn requires_structured_execution(&self) -> bool {
        self.rows
            .iter()
            .any(|row| matches!(&row.node, Node::Parallel { .. } | Node::Map { .. }))
    }

    read!(CompiledMachine,scope,&TenantScope,s=>&s.scope,"Read the validated scope captured during compilation.");
    read!(CompiledMachine,start,StateId,s=>s.start,"Read the validated entry ID.");
    read!(CompiledMachine,state_count,usize,s=>s.rows.len(),"Read the bounded number of compiled states.");
    read!(CompiledMachine,original_bytes,&[u8],s=>s.original.as_ref(),"Read the exact original root definition bytes, shared by all scopes.");
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
