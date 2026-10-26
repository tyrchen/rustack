//! Persistent bounded official history; retained views retain their global charge.

use std::{
    fmt,
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use rustack_stepfunctions_model::types::HistoryEvent;

use super::runtime::ExecutionError;
use crate::payload::encode_with_limit;

pub(super) mod events;
mod projection;
#[cfg(test)]
mod tests;

const FIXED_OVERHEAD: usize = 1024;
const TERMINAL_RESERVE: usize = 4096 + size_of::<StoredEvent>() + FIXED_OVERHEAD;
const EVENT_BYTES: usize = 2 * 1024 * 1024;
const PAGE_BYTES: usize = 4 * 1024 * 1024;
const PAGE_EVENTS: usize = 64;

/// Local per-execution history limits with one reserved termination event.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct HistoryLimits {
    events: u32,
    bytes: usize,
}
impl HistoryLimits {
    /// Validate2..=25,000 events and1..=256MiB including terminal capacity.
    ///
    /// # Errors
    /// Rejects out-of-range values.
    ///
    /// ```
    /// let limits=rustack_stepfunctions_core::HistoryLimits::new(25_000,32*1024*1024)?;
    /// # Ok::<(),rustack_stepfunctions_core::ExecutionError>(())
    /// ```
    pub fn new(events: u32, bytes: usize) -> Result<Self, ExecutionError> {
        if !(2..=25_000).contains(&events) || !(1024 * 1024..=256 * 1024 * 1024).contains(&bytes) {
            return Err(ExecutionError::Invalid);
        }
        Ok(Self { events, bytes })
    }
}
impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            events: 25_000,
            bytes: 32 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub(super) struct HistoryCapacity {
    used: AtomicUsize,
    limit: usize,
}
impl HistoryCapacity {
    pub(super) fn new(
        limit: usize,
        per_execution: HistoryLimits,
    ) -> Result<Arc<Self>, ExecutionError> {
        if !(8 * 1024 * 1024..=4_usize * 1024 * 1024 * 1024).contains(&limit)
            || limit < per_execution.bytes
        {
            return Err(ExecutionError::Invalid);
        }
        Ok(Arc::new(Self {
            used: AtomicUsize::new(0),
            limit,
        }))
    }
    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Charge, ExecutionError> {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.limit)
            })
            .map_err(|_| ExecutionError::Capacity)?;
        Ok(Charge {
            capacity: self.clone(),
            bytes,
        })
    }
    pub(super) fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
}
struct Charge {
    capacity: Arc<HistoryCapacity>,
    bytes: usize,
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.capacity.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
impl fmt::Debug for Charge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryCharge")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub(super) struct TerminalReserve {
    _charge: Charge,
}
impl TerminalReserve {
    pub(super) fn new(capacity: &Arc<HistoryCapacity>) -> Result<Arc<Self>, ExecutionError> {
        Ok(Arc::new(Self {
            _charge: capacity.reserve(TERMINAL_RESERVE)?,
        }))
    }
}
#[derive(Debug)]
struct StoredEvent {
    event: HistoryEvent,
    encoded: usize,
    _charge: EventCharge,
}
#[derive(Debug)]
enum EventCharge {
    Ordinary {
        _charge: Charge,
    },
    Terminal {
        _reserve: Arc<TerminalReserve>,
        _extra: Option<Charge>,
    },
}
#[derive(Debug)]
struct HistoryChunk {
    previous: Option<Arc<Self>>,
    events: Vec<Arc<StoredEvent>>,
}

/// Immutable stable history boundary; append shares prior pages and business data.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct ExecutionHistory {
    head: Option<Arc<HistoryChunk>>,
    count: u32,
    charged: usize,
}
impl fmt::Debug for ExecutionHistory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionHistory")
            .field("count", &self.count)
            .field("charged", &self.charged)
            .finish_non_exhaustive()
    }
}
/// A complete bounded page; pagination source is this history's fixed high water.
#[derive(Debug)]
#[non_exhaustive]
pub struct HistoryPage {
    events: Vec<HistoryEvent>,
    next: Option<u32>,
}
impl HistoryPage {
    /// Read complete official DTOs in ascending event order.
    ///
    /// ```
    /// # fn inspect(page:&rustack_stepfunctions_core::HistoryPage) { let events=page.events(); }
    /// ```
    #[must_use]
    pub fn events(&self) -> &[HistoryEvent] {
        &self.events
    }
    /// Read the next inclusive ID, if this captured boundary has more records.
    ///
    /// ```
    /// # fn inspect(page:&rustack_stepfunctions_core::HistoryPage) { let next=page.next_id(); }
    /// ```
    #[must_use]
    pub fn next_id(&self) -> Option<u32> {
        self.next
    }
}
impl ExecutionHistory {
    /// Read the captured number of events.
    ///
    /// ```
    /// assert_eq!(rustack_stepfunctions_core::ExecutionHistory::default().len(),0);
    /// ```
    #[must_use]
    pub fn len(&self) -> u32 {
        self.count
    }
    /// Test whether this view contains no events.
    ///
    /// ```
    /// assert!(rustack_stepfunctions_core::ExecutionHistory::default().is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    /// Read global/per-execution charged bytes, including reserved terminal space.
    ///
    /// ```
    /// assert_eq!(rustack_stepfunctions_core::ExecutionHistory::default().charged_bytes(),0);
    /// ```
    #[must_use]
    pub fn charged_bytes(&self) -> usize {
        self.charged
    }
    /// Read at most1,000 records with a4MiB complete-JSON page allowance.
    ///
    /// `first` is an inclusive ID in1..=len+1. Data-disabled projection removes
    /// input/output and parameter-bearing scheduled details; linkage stays intact.
    /// This is not an authenticated API pagination token.
    ///
    /// # Errors
    /// Rejects invalid bounds or an impossible event encoding invariant.
    ///
    /// ```
    /// let page=rustack_stepfunctions_core::ExecutionHistory::default().page(1,100,false)?;
    /// assert!(page.events().is_empty());
    /// # Ok::<(),rustack_stepfunctions_core::ExecutionError>(())
    /// ```
    pub fn page(
        &self,
        first: u32,
        limit: usize,
        include_data: bool,
    ) -> Result<HistoryPage, ExecutionError> {
        if first == 0 || first > self.count.saturating_add(1) || !(1..=1000).contains(&limit) {
            return Err(ExecutionError::Invalid);
        }
        let mut records = Vec::new();
        let mut head = self.head.as_deref();
        while let Some(chunk) = head {
            records.extend(chunk.events.iter().rev());
            head = chunk.previous.as_deref();
        }
        let mut events = Vec::new();
        let mut bytes = 256_usize;
        let mut next = None;
        for record in records
            .into_iter()
            .rev()
            .filter(|r| r.event.id >= i64::from(first))
        {
            if events.len() >= limit {
                next = u32::try_from(record.event.id).ok();
                break;
            }
            if include_data && bytes.saturating_add(record.encoded + 1) > PAGE_BYTES {
                next = u32::try_from(record.event.id).ok();
                break;
            }
            let mut event = record.event.clone();
            let encoded = if include_data {
                record.encoded
            } else {
                projection::omit_data(&mut event);
                encode_with_limit(&event, EVENT_BYTES)
                    .map_err(ExecutionError::from)?
                    .len()
            };
            if bytes.saturating_add(encoded + 1) > PAGE_BYTES {
                next = u32::try_from(record.event.id).ok();
                break;
            }
            bytes = bytes.saturating_add(encoded + 1);
            events.push(event);
        }
        Ok(HistoryPage { events, next })
    }
    fn push(&mut self, event: Arc<StoredEvent>) {
        let (previous, mut events) = match &self.head {
            Some(head) if head.events.len() < PAGE_EVENTS => {
                (head.previous.clone(), head.events.clone())
            }
            _ => (self.head.clone(), Vec::new()),
        };
        events.push(event);
        self.head = Some(Arc::new(HistoryChunk { previous, events }));
        self.count = self.count.saturating_add(1);
    }
}
#[derive(Debug)]
pub(super) struct Journal {
    pub(super) view: ExecutionHistory,
    limits: HistoryLimits,
    capacity: Arc<HistoryCapacity>,
    reserve: Arc<TerminalReserve>,
}
impl Journal {
    pub(super) fn new(
        limits: HistoryLimits,
        capacity: Arc<HistoryCapacity>,
        reserve: Arc<TerminalReserve>,
    ) -> Self {
        Self {
            view: ExecutionHistory {
                charged: TERMINAL_RESERVE,
                ..ExecutionHistory::default()
            },
            limits,
            capacity,
            reserve,
        }
    }
    pub(super) fn resume(
        view: ExecutionHistory,
        limits: HistoryLimits,
        capacity: Arc<HistoryCapacity>,
        reserve: Arc<TerminalReserve>,
    ) -> Self {
        Self {
            view,
            limits,
            capacity,
            reserve,
        }
    }
    pub(super) fn append(
        &mut self,
        mut event: HistoryEvent,
        terminal: bool,
    ) -> Result<(), ExecutionError> {
        let max_count = if terminal {
            self.limits.events
        } else {
            self.limits.events.saturating_sub(1)
        };
        if self.view.count >= max_count {
            return Err(ExecutionError::Capacity);
        }
        event.id = i64::from(self.view.count.saturating_add(1));
        event.previous_event_id = Some(i64::from(self.view.count));
        let encoded = encode_with_limit(&event, EVENT_BYTES)
            .map_err(ExecutionError::from)?
            .len();
        let charge = encoded
            .saturating_add(size_of::<StoredEvent>())
            .saturating_add(FIXED_OVERHEAD);
        let charge = if terminal {
            let extra = charge.saturating_sub(TERMINAL_RESERVE);
            if self.view.charged.saturating_add(extra) > self.limits.bytes {
                return Err(ExecutionError::Capacity);
            }
            let extra = if extra > 0 {
                Some(self.capacity.reserve(extra)?)
            } else {
                None
            };
            self.view.charged = self
                .view
                .charged
                .saturating_add(charge.saturating_sub(TERMINAL_RESERVE));
            EventCharge::Terminal {
                _reserve: self.reserve.clone(),
                _extra: extra,
            }
        } else {
            if self.view.charged.saturating_add(charge) > self.limits.bytes {
                return Err(ExecutionError::Capacity);
            }
            let reserved = self.capacity.reserve(charge)?;
            self.view.charged = self.view.charged.saturating_add(charge);
            EventCharge::Ordinary { _charge: reserved }
        };
        self.view.push(Arc::new(StoredEvent {
            event,
            encoded,
            _charge: charge,
        }));
        Ok(())
    }
}
