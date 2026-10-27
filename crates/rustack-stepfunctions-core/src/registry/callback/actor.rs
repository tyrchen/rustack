//! Unique hash-only callback table, retained entropy and removable timer entries.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};

use tokio::{
    sync::{OwnedSemaphorePermit, mpsc, watch},
    task::{JoinError, JoinSet, yield_now},
    time::{Instant, MissedTickBehavior, interval, sleep_until},
};

use super::{
    AckReply, Action, CallbackAttempt, CallbackError, CallbackInvocation,
    CallbackInvocationReporter, CallbackLimits, CallbackOutcome, CallbackStatus, CallbackTask,
    CallbackTimeout, CallbackToken, Control, InvocationIdentity, MintReply, MintRequest, Request,
    Shared, TokenHash,
};
use crate::{FrameCallId, Payload, TaskFailure};

type Completion = Result<Payload, TaskFailure>;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AttemptKey {
    execution: String,
    call: FrameCallId,
}
impl AttemptKey {
    fn of(attempt: &CallbackAttempt) -> Self {
        Self {
            execution: attempt.parent.arn().as_str().into(),
            call: attempt.call,
        }
    }
}
#[derive(Debug)]
enum Progress {
    Invoking(Option<Provisional>),
    Waiting(Option<Instant>),
    Closed,
}
#[derive(Debug)]
struct Provisional {
    result: Completion,
    received: Instant,
}
#[derive(Debug)]
struct Entry {
    digest: TokenHash,
    attempt: CallbackAttempt,
    control: Arc<Control>,
    invoked: Option<watch::Receiver<Option<Arc<CallbackInvocation>>>>,
    view: Option<watch::Sender<Arc<CallbackOutcome>>>,
    progress: Progress,
    live: Option<OwnedSemaphorePermit>,
    _retained: OwnedSemaphorePermit,
    timer: Option<Instant>,
    retire: Option<Instant>,
}
impl Entry {
    fn close(&mut self, outcome: CallbackOutcome) {
        if !matches!(self.progress, Progress::Closed) {
            self.progress = Progress::Closed;
            self.control.revoke();
            self.live = None;
            self.invoked = None;
            if let Some(view) = self.view.take() {
                view.send_replace(Arc::new(outcome));
            }
        }
    }
    fn refresh(&mut self, now: Instant) -> Result<(), CallbackError> {
        if matches!(self.progress, Progress::Closed) {
            return Ok(());
        }
        if now >= self.attempt.deadline {
            self.close(CallbackOutcome::TimedOut(CallbackTimeout::Task));
        } else if self.control.revoked()
            || self.attempt.parent.handle().snapshot().outcome().is_some()
        {
            self.close(CallbackOutcome::Revoked);
        } else if matches!(self.progress,Progress::Waiting(Some(until)) if now>=until) {
            self.close(CallbackOutcome::TimedOut(CallbackTimeout::Heartbeat));
        } else if matches!(self.progress, Progress::Invoking(_)) {
            let invoked = self.invoked.as_mut().ok_or(CallbackError::Progress)?;
            let report = invoked.borrow_and_update().clone();
            let owner_lost = invoked.has_changed().is_err();
            match report.as_deref() {
                Some(CallbackInvocation {
                    outcome: Err(failure),
                    ..
                }) => self.close(CallbackOutcome::InvocationFailed(failure.clone())),
                Some(CallbackInvocation {
                    outcome: Ok(_),
                    observed,
                }) => {
                    let Progress::Invoking(provisional) = &mut self.progress else {
                        return Err(CallbackError::Progress);
                    };
                    let provisional = provisional.take();
                    let heartbeat = self
                        .attempt
                        .heartbeat
                        .map(|period| {
                            observed
                                .checked_add(period)
                                .ok_or(CallbackError::Validation)
                        })
                        .transpose()?
                        .map(|until| until.min(self.attempt.deadline));
                    if let Some(Provisional { result, received }) = provisional {
                        if heartbeat.is_some_and(|until| received >= until) {
                            self.close(CallbackOutcome::TimedOut(CallbackTimeout::Heartbeat));
                        } else {
                            self.complete(result);
                        }
                    } else if heartbeat.is_some_and(|until| now >= until) {
                        self.close(CallbackOutcome::TimedOut(CallbackTimeout::Heartbeat));
                    } else {
                        self.progress = Progress::Waiting(heartbeat);
                        self.view
                            .as_ref()
                            .ok_or(CallbackError::Progress)?
                            .send_replace(Arc::new(CallbackOutcome::Waiting {
                                heartbeat_deadline: heartbeat,
                            }));
                    }
                }
                None if owner_lost => self.close(CallbackOutcome::Revoked),
                None => {}
            }
        }
        Ok(())
    }
    fn complete(&mut self, result: Completion) {
        self.close(match result {
            Ok(output) => CallbackOutcome::Succeeded(output),
            Err(failure) => CallbackOutcome::Failed(failure),
        });
    }
    fn mutate(&mut self, action: Action, now: Instant) -> Result<(), CallbackError> {
        self.refresh(now)?;
        match (&mut self.progress, action) {
            (Progress::Closed | Progress::Invoking(Some(_)), _) => Err(CallbackError::TaskTimedOut),
            (Progress::Invoking(provisional), Action::Complete(result)) => {
                *provisional = Some(Provisional {
                    result,
                    received: now,
                });
                Ok(())
            }
            (Progress::Waiting(_), Action::Complete(result)) => {
                self.complete(result);
                Ok(())
            }
            (Progress::Invoking(None), Action::Heartbeat) => Ok(()),
            (Progress::Waiting(heartbeat), Action::Heartbeat) => {
                *heartbeat = self
                    .attempt
                    .heartbeat
                    .map(|period| now.checked_add(period).ok_or(CallbackError::Validation))
                    .transpose()?
                    .map(|until| until.min(self.attempt.deadline));
                self.view
                    .as_ref()
                    .ok_or(CallbackError::Progress)?
                    .send_replace(Arc::new(CallbackOutcome::Waiting {
                        heartbeat_deadline: *heartbeat,
                    }));
                Ok(())
            }
        }
    }
    fn next(&self) -> Option<Instant> {
        match self.progress {
            Progress::Closed => self.retire,
            Progress::Invoking(_) => Some(self.attempt.deadline),
            Progress::Waiting(heartbeat) => {
                Some(heartbeat.map_or(self.attempt.deadline, |until| {
                    until.min(self.attempt.deadline)
                }))
            }
        }
    }
}
#[derive(Debug)]
struct Minted {
    id: u64,
    request: Box<MintRequest>,
    token: Result<CallbackToken, CallbackError>,
}
#[derive(Debug)]
struct State {
    shared: Arc<Shared>,
    limits: CallbackLimits,
    receiver: mpsc::Receiver<Request>,
    status: watch::Sender<CallbackStatus>,
    entries: BTreeMap<u64, Entry>,
    hashes: HashMap<TokenHash, u64>,
    attempts: BTreeMap<AttemptKey, u64>,
    active: BTreeSet<u64>,
    timers: BTreeSet<(Instant, u64)>,
    workers: JoinSet<Minted>,
    next: u64,
    closing: bool,
}
#[derive(Debug)]
pub(super) struct Owner {
    state: Option<Box<State>>,
}
impl Owner {
    pub(super) fn new(
        shared: Arc<Shared>,
        limits: CallbackLimits,
        receiver: mpsc::Receiver<Request>,
        status: watch::Sender<CallbackStatus>,
    ) -> Self {
        Self {
            state: Some(Box::new(State {
                shared,
                limits,
                receiver,
                status,
                entries: BTreeMap::new(),
                hashes: HashMap::new(),
                attempts: BTreeMap::new(),
                active: BTreeSet::new(),
                timers: BTreeSet::new(),
                workers: JoinSet::new(),
                next: 0,
                closing: false,
            })),
        }
    }
    pub(super) async fn run(mut self, stop: watch::Receiver<bool>) -> Result<(), CallbackError> {
        let result = match self.state.as_mut() {
            Some(state) => state.run(stop).await,
            None => Err(CallbackError::Unavailable),
        };
        if result.is_ok() {
            drop(self.state.take());
        }
        result
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(mut state) = self.state.take() {
            state.close();
            let _retained = Box::leak(state);
        }
    }
}
fn rejected(request: MintRequest, error: CallbackError) {
    let _sent = request.reply.send(MintReply {
        result: Err(Arc::new(error)),
        _permit: request.reservations.command,
    });
}
fn reject(request: Request) {
    match request {
        #[cfg(test)]
        Request::Inspect(_) | Request::Panic => {}
        Request::Mint(request) => rejected(*request, CallbackError::Unavailable),
        Request::Api(request) => {
            let _sent = request.reply.send(AckReply {
                result: Err(Arc::new(CallbackError::Unavailable)),
                _permit: request.permit,
            });
        }
    }
}
impl State {
    fn publish(&self) {
        self.status.send_replace(CallbackStatus {
            ready: !self.closing,
            live: self.active.len(),
            retained: self.entries.len(),
        });
    }
    fn close(&mut self) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.shared.closing.send_replace(true);
        self.receiver.close();
        while let Ok(request) = self.receiver.try_recv() {
            reject(request);
        }
        for entry in self.entries.values_mut() {
            entry.close(CallbackOutcome::Unavailable);
        }
        self.active.clear();
        self.publish();
    }
    fn schedule(&mut self, id: u64) {
        if let Some(entry) = self.entries.get_mut(&id) {
            if let Some(until) = entry.timer.take() {
                self.timers.remove(&(until, id));
            }
            entry.timer = entry.next();
            if let Some(until) = entry.timer {
                self.timers.insert((until, id));
            }
            if matches!(entry.progress, Progress::Closed) {
                self.active.remove(&id);
            }
        }
    }
    fn refresh(&mut self, id: u64, now: Instant) -> Result<(), CallbackError> {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.refresh(now)?;
        }
        self.schedule(id);
        Ok(())
    }
    fn controls(&mut self, now: Instant) -> Result<(), CallbackError> {
        let ids: Vec<_> = self.active.iter().copied().collect();
        for id in ids {
            self.refresh(id, now)?;
        }
        Ok(())
    }
    fn retention(&mut self, now: Instant) -> Result<(), CallbackError> {
        let mut changed = Vec::new();
        for (id, entry) in &mut self.entries {
            if matches!(entry.progress, Progress::Closed)
                && entry.retire.is_none()
                && entry.attempt.parent.handle().snapshot().outcome().is_some()
            {
                entry.retire = Some(
                    now.checked_add(self.limits.retention)
                        .ok_or(CallbackError::Validation)?,
                );
                changed.push(*id);
            }
        }
        for id in changed {
            self.schedule(id);
        }
        Ok(())
    }
    fn timers(&mut self, now: Instant) -> Result<(), CallbackError> {
        while let Some((until, id)) = self.timers.first().copied() {
            if until > now {
                break;
            }
            self.timers.remove(&(until, id));
            if let Some(entry) = self.entries.get_mut(&id) {
                entry.timer = None;
            }
            let retire = self.entries.get(&id).is_some_and(|entry| {
                matches!(entry.progress, Progress::Closed)
                    && entry.retire.is_some_and(|until| now >= until)
            });
            if retire {
                if let Some(entry) = self.entries.remove(&id) {
                    self.hashes.remove(&entry.digest);
                    self.attempts.remove(&AttemptKey::of(&entry.attempt));
                }
            } else {
                self.refresh(id, now)?;
            }
        }
        Ok(())
    }
    fn mint(&mut self, request: Box<MintRequest>) {
        let error = if self.closing {
            Some(CallbackError::Unavailable)
        } else if request.control.revoked() {
            Some(CallbackError::TaskTimedOut)
        } else if let Err(error) = request.attempt.live(Instant::now()) {
            Some(error)
        } else if self.workers.len() >= self.limits.workers {
            Some(CallbackError::Capacity)
        } else if self
            .attempts
            .contains_key(&AttemptKey::of(&request.attempt))
        {
            Some(CallbackError::AttemptExists)
        } else {
            None
        };
        if let Some(error) = error {
            rejected(*request, error);
            return;
        }
        let Some(id) = self.next.checked_add(1) else {
            rejected(*request, CallbackError::Capacity);
            return;
        };
        self.next = id;
        self.attempts.insert(AttemptKey::of(&request.attempt), id);
        #[cfg(test)]
        let hook = self.shared.entropy_hook.get().cloned();
        self.workers.spawn_blocking(move || {
            #[cfg(test)]
            if let Some(hook) = hook {
                hook.run();
            }
            Minted {
                id,
                request,
                token: CallbackToken::generate(),
            }
        });
    }
    fn minted(&mut self, result: Result<Minted, JoinError>) -> Result<(), CallbackError> {
        let Minted { id, request, token } = result.map_err(CallbackError::Worker)?;
        let token = match token {
            Ok(token) => token,
            Err(error) => {
                self.attempts.remove(&AttemptKey::of(&request.attempt));
                rejected(*request, error);
                return Ok(());
            }
        };
        let reject = if self.closing {
            Some(CallbackError::Unavailable)
        } else if request.control.revoked() {
            Some(CallbackError::TaskTimedOut)
        } else {
            request.attempt.live(Instant::now()).err()
        };
        if let Some(error) = reject {
            self.attempts.remove(&AttemptKey::of(&request.attempt));
            rejected(*request, error);
            return Ok(());
        }
        let digest = token.digest();
        if self.hashes.contains_key(&digest) {
            self.attempts.remove(&AttemptKey::of(&request.attempt));
            rejected(*request, CallbackError::Capacity);
            return Ok(());
        }
        let (view, reader) = watch::channel(Arc::new(CallbackOutcome::Invoking));
        let task = CallbackTask {
            token,
            control: request.control.clone(),
            reporter: Some(CallbackInvocationReporter::new(
                InvocationIdentity {
                    scope: request.attempt.parent.revision().machine().scope().clone(),
                    call: request.attempt.call,
                    deadline: request.attempt.deadline,
                    issued: Instant::now(),
                },
                request.control.clone(),
                request.invocation,
            )),
            invocation: request.invoked.clone(),
            view: reader,
            deadline: request.attempt.deadline,
        };
        self.hashes.insert(digest, id);
        self.active.insert(id);
        self.entries.insert(
            id,
            Entry {
                digest,
                attempt: request.attempt,
                control: request.control,
                invoked: Some(request.invoked),
                view: Some(view),
                progress: Progress::Invoking(None),
                live: Some(request.reservations.live),
                _retained: request.reservations.retained,
                timer: None,
                retire: None,
            },
        );
        self.schedule(id);
        let _sent = request.reply.send(MintReply {
            result: Ok(task),
            _permit: request.reservations.command,
        });
        Ok(())
    }
    fn api(&mut self, request: super::ApiRequest) {
        let result = (|| {
            let id = self
                .hashes
                .get(&request.digest)
                .copied()
                .ok_or(CallbackError::InvalidToken)?;
            let entry = self
                .entries
                .get_mut(&id)
                .ok_or(CallbackError::InvalidToken)?;
            let result = entry.mutate(request.action, Instant::now());
            self.schedule(id);
            result
        })();
        let _sent = request.reply.send(AckReply {
            result: result.map_err(Arc::new),
            _permit: request.permit,
        });
    }
    async fn run(&mut self, mut stop: watch::Receiver<bool>) -> Result<(), CallbackError> {
        let mut maintenance = interval(Duration::from_secs(1));
        maintenance.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut ticks = 0_u8;
        loop {
            if *stop.borrow() || stop.has_changed().is_err() {
                self.close();
            }
            while let Some(result) = self.workers.try_join_next() {
                self.minted(result)?;
            }
            let now = Instant::now();
            self.timers(now)?;
            self.publish();
            if self.closing && self.workers.is_empty() {
                return Ok(());
            }
            ticks = ticks.saturating_add(1);
            if ticks >= 64 {
                yield_now().await;
                ticks = 0;
            }
            let until = self
                .timers
                .first()
                .map_or_else(|| now + Duration::from_secs(3600), |(until, _)| *until);
            tokio::select! {
                biased;
                _=stop.changed(),if !self.closing=>self.close(),
                ()=self.shared.wake.notified(),if !self.closing=>self.controls(Instant::now())?,
                ()=sleep_until(until),if !self.closing=>self.timers(Instant::now())?,
                _=maintenance.tick(),if !self.closing=>{let now=Instant::now();self.controls(now)?;self.retention(now)?;},
                result=self.workers.join_next(),if !self.workers.is_empty()=>if let Some(result)=result{self.minted(result)?;},
                request=self.receiver.recv(),if !self.closing=>match request{#[cfg(test)] Some(Request::Inspect(reply))=>{let _sent=reply.send((self.entries.len(),self.active.len(),self.timers.len()));},#[cfg(test)] Some(Request::Panic)=>panic!("controlled callback root failure"),Some(Request::Mint(request))=>self.mint(request),Some(Request::Api(request))=>self.api(request),None=>self.close()},
            }
        }
    }
}
