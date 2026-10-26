//! Actor-owned opaque cursors over shared charged immutable captures.

use std::{
    collections::HashMap,
    fmt::{self, Write},
    mem::size_of,
    sync::Arc,
    time::Duration,
};

use getrandom::fill;
use rustack_stepfunctions_core::{ExecutionSnapshot, MachineSnapshot, RegistryResult};
use rustack_stepfunctions_http::Response;
use rustack_stepfunctions_model::{
    StepFunctionsError, StepFunctionsErrorCode as Code, StepFunctionsOperation as Op, output,
    types::ExecutionStatus,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, oneshot},
    time::Instant,
};

use super::{
    ApiLimits,
    error::{check, error, source},
    render,
};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Flags {
    pub(super) size: usize,
    pub(super) status: Option<ExecutionStatus>,
    pub(super) reverse: bool,
    pub(super) data: bool,
}
impl Flags {
    pub(super) fn new(
        size: Option<i32>,
        status: Option<ExecutionStatus>,
        reverse: bool,
        data: bool,
    ) -> Result<Self, StepFunctionsError> {
        let size = match size {
            None | Some(0) => 100,
            Some(n) => usize::try_from(n)
                .ok()
                .filter(|n| *n <= 1000)
                .ok_or_else(|| error(Code::ValidationException))?,
        };
        Ok(Self {
            size,
            status,
            reverse,
            data,
        })
    }
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Binding {
    pub(super) op: Op,
    pub(super) resource: String,
    pub(super) flags: Flags,
}
pub(super) enum Data {
    Machines(Vec<MachineSnapshot>),
    Executions(Vec<ExecutionSnapshot>),
    History(ExecutionSnapshot),
}
impl Data {
    pub(super) fn capture(
        binding: &Binding,
        result: RegistryResult,
    ) -> Result<Self, StepFunctionsError> {
        match (binding.op, result) {
            (Op::ListStateMachines, RegistryResult::Machines(values)) => Ok(Self::Machines(values)),
            (Op::ListExecutions, RegistryResult::Executions(values)) => {
                let mut captured = Vec::new();
                for value in values {
                    let status = render::status(&value)?;
                    if binding
                        .flags
                        .status
                        .as_ref()
                        .is_none_or(|filter| filter == &status)
                    {
                        captured.push(value);
                    }
                }
                Ok(Self::Executions(captured))
            }
            (Op::GetExecutionHistory, RegistryResult::Execution(value)) => Ok(Self::History(value)),
            _ => Err(error(Code::InternalFailure)),
        }
    }
    fn bytes(&self) -> Result<u32, StepFunctionsError> {
        let bytes = match self {
            Self::Machines(v) => v.capacity().checked_mul(size_of::<MachineSnapshot>()),
            Self::Executions(v) => v.capacity().checked_mul(size_of::<ExecutionSnapshot>()),
            Self::History(v) => Some(v.view().history().charged_bytes()),
        }
        .and_then(|n| n.checked_add(512))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| error(Code::ThrottlingException))?;
        Ok(bytes)
    }
}
pub(super) struct Snapshot {
    data: Data,
    _permit: OwnedSemaphorePermit,
}
#[derive(Clone)]
pub(super) struct Cursor {
    pub(super) binding: Binding,
    pub(super) snapshot: Arc<Snapshot>,
    pub(super) position: u32,
}
struct Token {
    cursor: Cursor,
    expires: Instant,
}
pub(super) struct Store {
    tokens: HashMap<String, Token>,
    capacity: usize,
    bytes: Arc<Semaphore>,
}
impl Store {
    pub(super) fn new(limits: ApiLimits) -> Self {
        Self {
            tokens: HashMap::new(),
            capacity: limits.tokens,
            bytes: Arc::new(Semaphore::new(limits.bytes)),
        }
    }
    pub(super) fn expire(&mut self, now: Instant) {
        self.tokens.retain(|_, token| token.expires > now);
    }
    fn reserve(&mut self, data: Data) -> Result<Arc<Snapshot>, StepFunctionsError> {
        self.expire(Instant::now());
        let permit = self
            .bytes
            .clone()
            .try_acquire_many_owned(data.bytes()?)
            .map_err(|_| error(Code::ThrottlingException))?;
        Ok(Arc::new(Snapshot {
            data,
            _permit: permit,
        }))
    }
    fn read(&mut self, token: &str, binding: &Binding) -> Result<Cursor, StepFunctionsError> {
        self.expire(Instant::now());
        let value = self
            .tokens
            .get(token)
            .filter(|v| v.cursor.binding == *binding)
            .ok_or_else(|| error(Code::InvalidToken))?;
        Ok(value.cursor.clone())
    }
    fn save(&mut self, token: String, cursor: Cursor) -> Result<(), StepFunctionsError> {
        self.expire(Instant::now());
        if self.tokens.len() >= self.capacity {
            return Err(error(Code::ThrottlingException));
        }
        if self.tokens.contains_key(&token) {
            return Err(error(Code::InternalFailure));
        }
        let expires = Instant::now()
            .checked_add(Duration::from_hours(24))
            .ok_or_else(|| error(Code::InternalFailure))?;
        self.tokens.insert(token, Token { cursor, expires });
        Ok(())
    }
    pub(super) fn dispatch(&mut self, command: Command) {
        let deadline = command.deadline;
        match command.request {
            Request::Reserve(data, reply) => {
                let result = check(deadline).and_then(|()| self.reserve(data));
                let _sent = reply.send(result);
            }
            Request::Read(token, binding, reply) => {
                let result = check(deadline).and_then(|()| self.read(&token, &binding));
                let _sent = reply.send(result);
            }
            Request::Save(token, cursor, reply) => {
                let result = check(deadline).and_then(|()| self.save(token, cursor));
                let _sent = reply.send(result);
            }
        }
    }
}
pub(super) struct Command {
    pub(super) deadline: Instant,
    pub(super) request: Request,
}
pub(super) enum Request {
    Reserve(
        Data,
        oneshot::Sender<Result<Arc<Snapshot>, StepFunctionsError>>,
    ),
    Read(
        String,
        Binding,
        oneshot::Sender<Result<Cursor, StepFunctionsError>>,
    ),
    Save(
        String,
        Cursor,
        oneshot::Sender<Result<(), StepFunctionsError>>,
    ),
}
pub(super) fn first(binding: Binding, snapshot: Arc<Snapshot>) -> Cursor {
    let position = match &snapshot.data {
        Data::History(e) if binding.flags.reverse => e.view().history().len(),
        Data::History(_) => 1,
        _ => 0,
    };
    Cursor {
        binding,
        snapshot,
        position,
    }
}
pub(super) struct Page {
    pub(super) response: Response,
    pub(super) next: Option<Cursor>,
    pub(super) token: String,
}
pub(super) fn page(cursor: Cursor) -> Result<Page, StepFunctionsError> {
    let flags = &cursor.binding.flags;
    let (response, next) = match &cursor.snapshot.data {
        Data::Machines(values) => {
            let (slice, next) = slice(values, cursor.position, flags.size)?;
            let mut out = output::ListStateMachinesOutput::default();
            out.state_machines = slice.iter().map(render::machine_item).collect();
            (out.into(), next)
        }
        Data::Executions(values) => {
            let (slice, next) = slice(values, cursor.position, flags.size)?;
            let mut out = output::ListExecutionsOutput::default();
            out.executions = slice
                .iter()
                .map(render::execution_item)
                .collect::<Result<Vec<_>, _>>()?;
            (out.into(), next)
        }
        Data::History(value) => {
            let history = value.view().history();
            let page = if flags.reverse {
                history.page_reverse(cursor.position, flags.size, flags.data)
            } else {
                history.page(cursor.position, flags.size, flags.data)
            }
            .map_err(|e| source(Code::InternalFailure, e))?;
            let mut out = output::GetExecutionHistoryOutput::default();
            out.events = page.events().to_vec();
            (out.into(), page.next_id())
        }
    };
    let token = if next.is_some() {
        nonce()?
    } else {
        String::new()
    };
    Ok(Page {
        response,
        next: next.map(|position| Cursor { position, ..cursor }),
        token,
    })
}
fn slice<T>(
    values: &[T],
    position: u32,
    size: usize,
) -> Result<(&[T], Option<u32>), StepFunctionsError> {
    let start = usize::try_from(position).map_err(|e| source(Code::InternalFailure, e))?;
    let end = start
        .checked_add(size)
        .ok_or_else(|| error(Code::InternalFailure))?
        .min(values.len());
    let slice = values
        .get(start..end)
        .ok_or_else(|| error(Code::InternalFailure))?;
    let next = if end < values.len() {
        Some(u32::try_from(end).map_err(|e| source(Code::InternalFailure, e))?)
    } else {
        None
    };
    Ok((slice, next))
}
fn nonce() -> Result<String, StepFunctionsError> {
    let mut bytes = [0_u8; 32];
    fill(&mut bytes).map_err(|e| source(Code::InternalFailure, e))?;
    let mut out = String::with_capacity(64);
    for byte in bytes {
        write!(out, "{byte:02x}").map_err(|e| source(Code::InternalFailure, e))?;
    }
    Ok(out)
}
pub(super) fn attach(response: &mut Response, token: String) -> Result<(), StepFunctionsError> {
    match response {
        Response::ListStateMachines(v) => v.next_token = Some(token),
        Response::ListExecutions(v) => v.next_token = Some(token),
        Response::GetExecutionHistory(v) => v.next_token = Some(token),
        _ => return Err(error(Code::InternalFailure)),
    }
    Ok(())
}
impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PageStore(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> Binding {
        Binding {
            op: Op::ListStateMachines,
            resource: String::new(),
            flags: Flags::new(Some(1), None, false, true).unwrap(),
        }
    }
    #[test]
    fn test_should_charge_sparse_vector_capacity_and_release_only_after_final_snapshot() {
        let mut store = Store::new(ApiLimits::new(1, 1, 100, 8 * 1024 * 1024).unwrap());
        let mut snapshots = Vec::new();
        for _ in 0..5 {
            snapshots.push(
                store
                    .reserve(Data::Executions(Vec::with_capacity(100_000)))
                    .unwrap(),
            );
        }
        assert!(
            store
                .reserve(Data::Executions(Vec::with_capacity(100_000)))
                .is_err()
        );
        let held = snapshots.pop().unwrap();
        let clone = held.clone();
        drop(held);
        assert!(
            store
                .reserve(Data::Executions(Vec::with_capacity(100_000)))
                .is_err()
        );
        drop(clone);
        assert!(
            store
                .reserve(Data::Executions(Vec::with_capacity(100_000)))
                .is_ok()
        );
    }
    #[test]
    fn test_should_preserve_live_tokens_at_capacity_and_expire_them_before_read() {
        let mut store = Store::new(ApiLimits::new(1, 1, 1, 8 * 1024 * 1024).unwrap());
        let snapshot = store.reserve(Data::Machines(Vec::new())).unwrap();
        let cursor = first(binding(), snapshot);
        store.save("a".repeat(64), cursor.clone()).unwrap();
        assert!(store.save("b".repeat(64), cursor.clone()).is_err());
        assert!(store.read(&"a".repeat(64), &binding()).is_ok());
        let expired = Instant::now() + Duration::from_hours(24);
        store.expire(expired);
        assert!(store.read(&"a".repeat(64), &binding()).is_err());
        store.save("b".repeat(64), cursor).unwrap();
        assert!(store.read(&"b".repeat(64), &binding()).is_ok());
    }
    #[test]
    fn test_should_reject_binding_changes_and_random_handle_collisions_without_overwriting() {
        let mut store = Store::new(ApiLimits::default());
        let snapshot = store.reserve(Data::Machines(Vec::new())).unwrap();
        let cursor = first(binding(), snapshot);
        store.save("a".repeat(64), cursor.clone()).unwrap();
        assert!(store.save("a".repeat(64), cursor).is_err());
        let mut changed = binding();
        changed.flags.data = false;
        assert_eq!(
            store.read(&"a".repeat(64), &changed).err().unwrap().code,
            Code::InvalidToken
        );
        assert!(store.read(&"a".repeat(64), &binding()).is_ok());
    }
}
