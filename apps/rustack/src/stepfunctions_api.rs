//! Real registry-backed M1 API, retained preparation and charged stable cursors.
//!
//! This binary-support layer composes core and HTTP without advertising a main
//! service. async-trait is required by the transport's object-safe handler port.

use std::{collections::HashMap, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use rustack_stepfunctions_core::{Registry, TaskSupport, TenantScope};
use rustack_stepfunctions_http::{Request, RequestContext, Response, StepFunctionsHandler};
use rustack_stepfunctions_model::{StepFunctionsError, StepFunctionsErrorCode as Code};
use tokio::{
    runtime::Handle,
    spawn,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::{Id, JoinError, JoinHandle, JoinSet},
    time::{Instant, MissedTickBehavior, interval, timeout_at},
};

mod error;
mod pages;
mod prepare;
mod render;
#[cfg(test)]
mod tests;
mod worker;

pub use error::ApiError;

/// Validated API, preparation and stable-pagination admission limits.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ApiLimits {
    accepted: usize,
    cpu: usize,
    tokens: usize,
    bytes: usize,
}
impl ApiLimits {
    /// Validate accepted1..128, workers1..32≤accepted, tokens1..100k and8MiB..1GiB.
    ///
    /// # Errors
    /// Rejects zero/excessive or inconsistent trusted limits.
    ///
    /// ```
    /// let limits=rustack_cli::stepfunctions_api::ApiLimits::new(32,8,10000,64*1024*1024)?;
    /// # Ok::<(),rustack_cli::stepfunctions_api::ApiError>(())
    /// ```
    pub fn new(accepted: usize, cpu: usize, tokens: usize, bytes: usize) -> Result<Self, ApiError> {
        if !(1..=128).contains(&accepted)
            || !(1..=32).contains(&cpu)
            || cpu > accepted
            || !(1..=100_000).contains(&tokens)
            || !(8 * 1024 * 1024..=1024 * 1024 * 1024).contains(&bytes)
        {
            return Err(ApiError::Configuration);
        }
        Ok(Self {
            accepted,
            cpu,
            tokens,
            bytes,
        })
    }
}
impl Default for ApiLimits {
    fn default() -> Self {
        Self {
            accepted: 32,
            cpu: 8,
            tokens: 10_000,
            bytes: 64 * 1024 * 1024,
        }
    }
}
struct Admission {
    sender: mpsc::Sender<Envelope>,
    capacity: Arc<Semaphore>,
    closing: watch::Sender<bool>,
    diagnostic: watch::Receiver<Option<Arc<ApiError>>>,
}
struct Context {
    scope: TenantScope,
    tasks: TaskSupport,
    registry: Registry,
    cpu: Arc<Semaphore>,
    pages: mpsc::Sender<pages::Command>,
}
struct Envelope {
    request: Request,
    deadline: Instant,
    reply: oneshot::Sender<Reply>,
    permit: OwnedSemaphorePermit,
}
struct Reply {
    result: Result<Response, StepFunctionsError>,
    permit: Option<OwnedSemaphorePermit>,
}
struct Slot {
    reply: oneshot::Sender<Reply>,
    permit: OwnedSemaphorePermit,
}
struct Io {
    requests: mpsc::Receiver<Envelope>,
    pages: mpsc::Receiver<pages::Command>,
    context: Arc<Context>,
    closing: watch::Sender<bool>,
    diagnostic: watch::Sender<Option<Arc<ApiError>>>,
}
struct State {
    io: Io,
    jobs: JoinSet<Result<Response, StepFunctionsError>>,
    slots: HashMap<Id, Slot>,
    quarantine: Vec<OwnedSemaphorePermit>,
    store: pages::Store,
}
struct Owner {
    state: Option<State>,
}

/// Real typed handler with bounded caller-independent work ownership.
#[non_exhaustive]
pub struct StepFunctionsApi {
    admission: Arc<Admission>,
    scope: TenantScope,
}
impl fmt::Debug for StepFunctionsApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StepFunctionsApi(<redacted>)")
    }
}
impl StepFunctionsApi {
    /// Compose the real registry with compiler capability supplied by its owner.
    ///
    /// # Errors
    /// Requires active Tokio and matching trusted registry/operator scope.
    ///
    /// ```no_run
    /// # fn compose(registry:rustack_stepfunctions_core::Registry)->Result<(),rustack_cli::stepfunctions_api::ApiError>{
    /// let(handler,owner)=rustack_cli::stepfunctions_api::StepFunctionsApi::new(Default::default(),rustack_stepfunctions_core::TaskSupport::Disabled,registry,Default::default())?;
    /// # Ok(()) }
    /// ```
    pub fn new(
        scope: TenantScope,
        tasks: TaskSupport,
        registry: Registry,
        limits: ApiLimits,
    ) -> Result<(Arc<Self>, ApiSupervisor), ApiError> {
        let (api, owner, stop) = Self::compose(scope, tasks, registry, limits)?;
        let supervisor = ApiSupervisor {
            closing: api.admission.closing.clone(),
            join: Some(spawn(owner.run(stop))),
            result: None,
        };
        Ok((api, supervisor))
    }
    fn compose(
        scope: TenantScope,
        tasks: TaskSupport,
        registry: Registry,
        limits: ApiLimits,
    ) -> Result<(Arc<Self>, Owner, watch::Receiver<bool>), ApiError> {
        Handle::try_current().map_err(|_| ApiError::Configuration)?;
        let other = registry.scope();
        if scope.partition() != other.partition()
            || scope.region() != other.region()
            || scope.account() != other.account()
        {
            return Err(ApiError::Configuration);
        }
        let (sender, requests) = mpsc::channel(limits.accepted);
        let (page_tx, pages) = mpsc::channel(limits.accepted);
        let (closing, stop) = watch::channel(false);
        let (diagnostic, diagnostics) = watch::channel(None);
        let context = Arc::new(Context {
            scope: scope.clone(),
            tasks,
            registry,
            cpu: Arc::new(Semaphore::new(limits.cpu)),
            pages: page_tx,
        });
        let owner = Owner {
            state: Some(State {
                io: Io {
                    requests,
                    pages,
                    context,
                    closing: closing.clone(),
                    diagnostic,
                },
                jobs: JoinSet::new(),
                slots: HashMap::new(),
                quarantine: Vec::new(),
                store: pages::Store::new(limits),
            }),
        };
        let api = Arc::new(Self {
            scope,
            admission: Arc::new(Admission {
                sender,
                capacity: Arc::new(Semaphore::new(limits.accepted)),
                closing: closing.clone(),
                diagnostic: diagnostics,
            }),
        });
        Ok((api, owner, stop))
    }
    /// Inspect a retained typed failure without exposing source text in Debug.
    ///
    /// ```
    /// # fn inspect(api:&rustack_cli::stepfunctions_api::StepFunctionsApi){let diagnostic=api.diagnostic();}
    /// ```
    #[must_use]
    pub fn diagnostic(&self) -> Option<Arc<ApiError>> {
        self.admission.diagnostic.borrow().clone()
    }
    /// Inspect admission/root health without exposing request data.
    ///
    /// ```
    /// # fn inspect(api:&rustack_cli::stepfunctions_api::StepFunctionsApi){let ready=api.is_ready();}
    /// ```
    #[must_use]
    pub fn is_ready(&self) -> bool {
        !*self.admission.closing.borrow() && self.admission.diagnostic.borrow().is_none()
    }
}
#[async_trait]
impl StepFunctionsHandler for StepFunctionsApi {
    #[tracing::instrument(name = "stepfunctions.api", skip_all)]
    async fn handle(
        &self,
        context: RequestContext,
        request: Request,
    ) -> Result<Response, StepFunctionsError> {
        let scope = context.scope();
        if scope.partition() != self.scope.partition()
            || scope.region() != self.scope.region()
            || scope.account() != self.scope.account()
        {
            return Err(error::error(Code::AccessDeniedException));
        }
        let deadline = context.deadline();
        error::check(deadline)?;
        if *self.admission.closing.borrow() {
            return Err(error::error(Code::ServiceUnavailable));
        }
        let permit = self
            .admission
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| error::error(Code::ThrottlingException))?;
        let (reply, receiver) = oneshot::channel();
        self.admission
            .sender
            .try_send(Envelope {
                request,
                deadline,
                reply,
                permit,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => error::error(Code::ThrottlingException),
                mpsc::error::TrySendError::Closed(_) => error::error(Code::ServiceUnavailable),
            })?;
        let reply = timeout_at(deadline, receiver)
            .await
            .map_err(|_| error::error(Code::RequestTimeoutException))?
            .map_err(|_| error::error(Code::ServiceUnavailable))?;
        let result = reply.result;
        drop(reply.permit);
        error::check(deadline)?;
        result
    }
}
impl Owner {
    async fn run(mut self, stop: watch::Receiver<bool>) -> Result<(), ApiError> {
        let result = match self.state.as_mut() {
            Some(state) => state.run(stop).await,
            None => Err(ApiError::Quarantined),
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
            state.io.closing.send_replace(true);
            state.io.diagnostic.send_if_modified(|current| {
                if current.is_some() {
                    false
                } else {
                    *current = Some(Arc::new(ApiError::Quarantined));
                    true
                }
            });
            state.io.requests.close();
            while let Ok(envelope) = state.io.requests.try_recv() {
                let _sent = envelope.reply.send(Reply {
                    result: Err(error::error(Code::ServiceUnavailable)),
                    permit: Some(envelope.permit),
                });
            }
            let _retained = Box::leak(Box::new(state));
        }
    }
}
impl State {
    async fn run(&mut self, mut stop: watch::Receiver<bool>) -> Result<(), ApiError> {
        let mut closed = false;
        let mut incoming = true;
        let mut timer = interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            if !closed && (*stop.borrow() || stop.has_changed().is_err()) {
                closed = true;
                self.io.closing.send_replace(true);
                self.io.requests.close();
            }
            if !incoming && self.jobs.is_empty() {
                return if self.quarantine.is_empty() {
                    Ok(())
                } else {
                    Err(ApiError::Quarantined)
                };
            }
            tokio::select! {biased;
                _=stop.changed(),if !closed=>{},
                joined=self.jobs.join_next_with_id(),if !self.jobs.is_empty()=>if let Some(joined)=joined{self.finish(joined);},
                command=self.io.pages.recv()=>if let Some(command)=command{self.store.dispatch(command);},
                envelope=self.io.requests.recv(),if incoming=>if let Some(envelope)=envelope{self.start(envelope);}else{incoming=false;closed=true;self.io.closing.send_replace(true);},
                _=timer.tick()=>self.store.expire(Instant::now()),
            }
        }
    }
    fn start(&mut self, envelope: Envelope) {
        let context = self.io.context.clone();
        let job = self
            .jobs
            .spawn(worker::run(context, envelope.request, envelope.deadline));
        self.slots.insert(
            job.id(),
            Slot {
                reply: envelope.reply,
                permit: envelope.permit,
            },
        );
    }
    fn finish(&mut self, joined: Result<(Id, Result<Response, StepFunctionsError>), JoinError>) {
        match joined {
            Ok((id, result)) => {
                if let Some(slot) = self.slots.remove(&id) {
                    let _sent = slot.reply.send(Reply {
                        result,
                        permit: Some(slot.permit),
                    });
                }
            }
            Err(error) => {
                let id = error.id();
                let diagnostic = Arc::new(ApiError::Worker(error));
                self.io.diagnostic.send_replace(Some(diagnostic.clone()));
                if let Some(slot) = self.slots.remove(&id) {
                    let _sent = slot.reply.send(Reply {
                        result: Err(error::source(Code::InternalFailure, diagnostic)),
                        permit: None,
                    });
                    self.quarantine.push(slot.permit);
                }
            }
        }
    }
}
/// Unique resumable API-root shutdown controller.
#[derive(Debug)]
#[non_exhaustive]
pub struct ApiSupervisor {
    closing: watch::Sender<bool>,
    join: Option<JoinHandle<Result<(), ApiError>>>,
    result: Option<Result<(), Arc<ApiError>>>,
}
impl ApiSupervisor {
    /// Close admission; all accepted work remains owned until joined.
    ///
    /// ```
    /// # fn close(owner:&rustack_cli::stepfunctions_api::ApiSupervisor){owner.close();}
    /// ```
    pub fn close(&self) {
        self.closing.send_replace(true);
    }
    /// Join the same root through an absolute observation deadline.
    ///
    /// # Errors
    /// Deadline preserves its handle; worker/quarantine errors remain cached.
    ///
    /// ```no_run
    /// # async fn join(owner:&mut rustack_cli::stepfunctions_api::ApiSupervisor,deadline:tokio::time::Instant){let result=owner.shutdown(deadline).await;}
    /// ```
    pub async fn shutdown(&mut self, deadline: Instant) -> Result<(), ApiError> {
        self.close();
        if self.result.is_none() {
            let join = self.join.as_mut().ok_or(ApiError::Quarantined)?;
            let joined = timeout_at(deadline, join)
                .await
                .map_err(|_| ApiError::Deadline)?;
            self.join = None;
            self.result = Some(match joined {
                Ok(result) => result.map_err(Arc::new),
                Err(error) => Err(Arc::new(ApiError::Worker(error))),
            });
        }
        match self.result.as_ref() {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(ApiError::Owner(error.clone())),
            None => Err(ApiError::Quarantined),
        }
    }
}
impl Drop for ApiSupervisor {
    fn drop(&mut self) {
        self.close(); /* Detach only the owning root; it joins its accepted jobs. */
    }
}
