//! Pool registry, FIFO dispatch and retained emergency retirement.

use std::{
    collections::{HashMap, VecDeque},
    mem,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rustack_lambda_runtime::session::RuntimeResult;
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot, watch},
    task::{Id, JoinSet},
    time::{Instant, interval, sleep, sleep_until},
};
use typed_builder::TypedBuilder;

use super::{Call, Offer, PoolConfig, PoolStatus};
use crate::{
    docker::{
        EnvironmentError, EnvironmentHandle, EnvironmentPhase, EnvironmentPort,
        connection::stopped, environment::Retirement,
    },
    executor::ExecutorError,
    invocation::CleanupTask,
};

pub(super) struct Warm {
    key: String,
    environment: EnvironmentHandle,
}
pub(super) struct Completion {
    warm: Option<Warm>,
    failed: bool,
}
pub(super) struct Record {
    retirement: Retirement,
    settlement: Option<watch::Sender<Option<Result<(), EnvironmentError>>>>,
    failed: bool,
}
impl Record {
    fn maintenance(retirement: Retirement) -> Self {
        Self {
            retirement,
            settlement: None,
            failed: false,
        }
    }
    fn publish(self, failed: bool) {
        if let Some(settlement) = self.settlement {
            settlement.send_replace(Some(if self.failed || failed {
                Err(EnvironmentError::Worker)
            } else {
                Ok(())
            }));
        }
    }
}
#[derive(TypedBuilder)]
pub(super) struct Actor {
    environments: EnvironmentPort,
    config: PoolConfig,
    inbox: mpsc::Receiver<Call>,
    stop: watch::Sender<bool>,
    failed: Arc<AtomicBool>,
    status: watch::Sender<PoolStatus>,
    emergency: Arc<CleanupTask>,
    runtime: Handle,
    #[builder(default)]
    pending: VecDeque<Call>,
    #[builder(default)]
    warm: VecDeque<Warm>,
    #[builder(default)]
    jobs: JoinSet<Completion>,
    #[builder(default)]
    receipts: HashMap<Id, Record>,
    #[builder(default)]
    settled: bool,
    #[builder(default)]
    eviction: Option<Id>,
}
impl Actor {
    pub(super) async fn run(mut self) -> Result<(), EnvironmentError> {
        let mut stop = self.stop.subscribe();
        let mut tick = interval(Duration::from_millis(100));
        let mut stopping = false;
        loop {
            if *stop.borrow() || self.inbox.is_closed() {
                stopping = true;
            }
            if stopping {
                self.inbox.close();
                while let Ok(call) = self.inbox.try_recv() {
                    call.reject(self.closed_error());
                }
                while let Some(call) = self.pending.pop_front() {
                    call.reject(self.closed_error());
                }
                while self.jobs.len() < 32 {
                    let Some(warm) = self.warm.pop_front() else {
                        break;
                    };
                    self.retire(warm.environment);
                }
            } else {
                self.expire();
                self.dispatch();
            }
            self.status.send_replace(PoolStatus {
                active: self.jobs.len(),
                warm: self.warm.len(),
                ..PoolStatus::default()
            });
            if stopping && self.jobs.is_empty() && self.warm.is_empty() {
                self.settled = true;
                return if self.failed.load(Ordering::Acquire) {
                    Err(EnvironmentError::Worker)
                } else {
                    Ok(())
                };
            }
            tokio::select! { biased;
                () = stopped(&mut stop), if !stopping => { stopping = true; },
                completion = self.jobs.join_next_with_id(), if !self.jobs.is_empty() => { if let Some(completion) = completion { self.completed(completion); } },
                call = self.inbox.recv(), if !stopping => { match call { Some(call) => self.pending.push_back(call), None => stopping = true } },
                _ = tick.tick() => {},
            }
        }
    }
    fn fail(&self) {
        self.failed.store(true, Ordering::Release);
        self.stop.send_replace(true);
    }
    fn closed_error(&self) -> EnvironmentError {
        if self.failed.load(Ordering::Acquire) {
            EnvironmentError::Worker
        } else {
            EnvironmentError::Stopped
        }
    }
    fn expire(&mut self) {
        let mut index = 0;
        while index < self.pending.len() {
            let error = self.pending.get(index).and_then(Call::error);
            if let Some(error) = error {
                if let Some(call) = self.pending.remove(index) {
                    call.reject(error);
                }
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.warm.len() && self.jobs.len() < 32 {
            if self
                .warm
                .get(index)
                .is_some_and(|warm| warm.environment.status().phase != EnvironmentPhase::Idle)
            {
                if let Some(warm) = self.warm.remove(index) {
                    self.retire(warm.environment);
                }
            } else {
                index += 1;
            }
        }
    }
    fn dispatch(&mut self) {
        if self.eviction.is_some() {
            return;
        }
        while self.jobs.len() < 32 {
            let Some(mut call) = self.pending.pop_front() else {
                return;
            };
            if let Some(error) = call.error() {
                call.reject(error);
                continue;
            }
            if let Err(error) = call.plan.live() {
                call.reject(error.into());
                continue;
            }
            let key = call.plan.warm_key().to_owned();
            let warm = self
                .warm
                .iter()
                .position(|warm| warm.key == key)
                .and_then(|index| self.warm.remove(index));
            let environment = match warm {
                Some(warm) => Ok(warm.environment),
                None => self
                    .environments
                    .spawn_shared(call.plan.clone(), call.deadline()),
            };
            match environment {
                Ok(environment) => {
                    call.queue.take();
                    let record = Record {
                        retirement: environment.retirement(),
                        settlement: Some(call.settlement.clone()),
                        failed: false,
                    };
                    let stop = self.stop.subscribe();
                    let job = self.jobs.spawn(invoke(environment, key, call, stop));
                    self.receipts.insert(job.id(), record);
                }
                Err(EnvironmentError::Busy) => {
                    self.pending.push_front(call);
                    if let Some(warm) = self.warm.pop_front() {
                        self.eviction = Some(self.retire(warm.environment));
                    }
                    return;
                }
                Err(error) => {
                    call.reject(error);
                }
            }
        }
    }
    fn retire(&mut self, environment: EnvironmentHandle) -> Id {
        let receipt = environment.retirement();
        receipt.cancel();
        drop(environment);
        self.retire_receipt(receipt)
    }
    fn retire_receipt(&mut self, receipt: Retirement) -> Id {
        self.retire_record(Record::maintenance(receipt))
    }
    fn retire_record(&mut self, record: Record) -> Id {
        record.retirement.cancel();
        let job = self.jobs.spawn(retire(record.retirement.clone()));
        self.receipts.insert(job.id(), record);
        job.id()
    }
    fn completed(&mut self, result: Result<(Id, Completion), tokio::task::JoinError>) {
        let id = match &result {
            Ok((id, _)) => *id,
            Err(error) => error.id(),
        };
        let receipt = self.receipts.remove(&id);
        if self.eviction == Some(id) {
            self.eviction = None;
        }
        if let Ok((_, completion)) = result {
            if completion.failed {
                self.fail();
            }
            if let Some(warm) = completion.warm {
                let count = self.warm.iter().filter(|idle| idle.key == warm.key).count();
                if !*self.stop.borrow()
                    && warm.environment.status().phase == EnvironmentPhase::Idle
                    && self.warm.len() < usize::from(self.config.0.warm)
                    && count < usize::from(self.config.0.warm_per_revision)
                {
                    self.warm.push_back(warm);
                } else {
                    self.retire(warm.environment);
                }
            }
            if let Some(record) = receipt {
                record.publish(completion.failed);
            }
        } else {
            self.fail();
            if let Some(mut record) = receipt {
                record.failed = true;
                self.retire_record(record);
            }
        }
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        if !self.settled {
            self.fail();
        }
        while let Ok(call) = self.inbox.try_recv() {
            call.reject(self.closed_error());
        }
        for call in self.pending.drain(..) {
            call.reject(EnvironmentError::Worker);
        }
        if self.receipts.is_empty() && self.warm.is_empty() {
            return;
        }
        let mut jobs = mem::take(&mut self.jobs);
        let mut receipts: Vec<_> = mem::take(&mut self.receipts).into_values().collect();
        for warm in self.warm.drain(..) {
            receipts.push(Record::maintenance(warm.environment.retirement()));
            drop(warm);
        }
        for record in &mut receipts {
            record.failed = true;
            record.retirement.cancel();
        }
        jobs.abort_all();
        self.emergency.start_on(&self.runtime, move || async move {
            while jobs.join_next().await.is_some() {}
            recover(receipts).await
        });
    }
}
async fn recover(receipts: Vec<Record>) -> Result<(), ExecutorError> {
    let mut cleanup = JoinSet::new();
    let mut index = HashMap::new();
    let mut failed = false;
    for record in receipts {
        let job = cleanup.spawn(retire(record.retirement.clone()));
        index.insert(job.id(), record);
    }
    while let Some(result) = cleanup.join_next_with_id().await {
        let id = match &result {
            Ok((id, _)) => *id,
            Err(error) => error.id(),
        };
        let Some(mut record) = index.remove(&id) else {
            continue;
        };
        if let Ok((_, completion)) = result {
            failed |= completion.failed;
            if record.retirement.removed() {
                record.publish(completion.failed);
                continue;
            }
        } else {
            failed = true;
            record.failed = true;
        }
        let job = cleanup.spawn(retire(record.retirement.clone()));
        index.insert(job.id(), record);
    }
    if failed {
        Err(ExecutorError::CleanupFailed(
            "Pool recovery supervision failed".into(),
        ))
    } else {
        Ok(())
    }
}
async fn retire(receipt: Retirement) -> Completion {
    let mut failed = false;
    loop {
        let result = receipt.clone().settle().await;
        failed |= result.is_err();
        if receipt.removed() {
            return Completion { warm: None, failed };
        }
        sleep(Duration::from_secs(1)).await;
    }
}
async fn invoke(
    mut environment: EnvironmentHandle,
    key: String,
    call: Call,
    mut stop: watch::Receiver<bool>,
) -> Completion {
    let deadline = call.deadline();
    let mut cancellation = call.cancellation.clone();
    let operation = async {
        if let Some(error) = call.error() {
            return Err(error);
        }
        environment
            .wait_ready(deadline.min(Instant::now() + Duration::from_secs(60)))
            .await?;
        environment
            .invoke(call.invocation.clone(), call.capture_logs)
            .await
    };
    let result = tokio::select! { biased;
        () = stopped(&mut stop) => Err(EnvironmentError::Stopped),
        () = stopped(&mut cancellation) => Err(EnvironmentError::Cancelled),
        () = sleep_until(deadline) => Err(EnvironmentError::Deadline),
        result = operation => result,
    };
    if matches!(&result, Ok(response) if matches!(response.result, RuntimeResult::Success(_)))
        && call.error().is_none()
        && !*stop.borrow()
    {
        let (receipt, consumed) = oneshot::channel();
        let sent = call
            .reply
            .send(Offer {
                result,
                receipt: Some(receipt),
            })
            .is_ok();
        if sent {
            let accepted = tokio::select! { biased;
                () = stopped(&mut stop) => false,
                () = sleep_until(deadline) => false,
                result = consumed => result.is_ok() && Instant::now() < deadline,
                () = stopped(&mut cancellation) => false,
            };
            if accepted {
                return Completion {
                    warm: Some(Warm { key, environment }),
                    failed: false,
                };
            }
        }
        let receipt = environment.retirement();
        drop(environment);
        retire(receipt).await
    } else {
        let receipt = environment.retirement();
        drop(environment);
        let completion = retire(receipt).await;
        let _ = call.reply.send(Offer {
            result,
            receipt: None,
        });
        completion
    }
}

#[cfg(test)]
pub(super) fn testing_start(
    environments: EnvironmentPort,
    config: PoolConfig,
    warm: Vec<(String, EnvironmentHandle)>,
) -> super::EnvironmentPool {
    testing_start_inner(environments, config, warm, false, None)
}
#[cfg(test)]
pub(super) fn testing_failed_worker_start(
    environments: EnvironmentPort,
    warm: Vec<(String, EnvironmentHandle)>,
) -> super::EnvironmentPool {
    testing_start_inner(environments, PoolConfig::default(), warm, true, None)
}
#[cfg(test)]
pub(super) fn testing_failed_call_start(
    environments: EnvironmentPort,
    plan: crate::docker::EnvironmentPlan,
    invocation: rustack_lambda_runtime::session::Invocation,
    environment: EnvironmentHandle,
) -> (super::EnvironmentPool, super::PoolInvocation) {
    let deadline = invocation.parts().1;
    let (stop, cancellation) = watch::channel(false);
    let (reply, response) = oneshot::channel();
    let (settlement, observed) = watch::channel(None);
    let key = plan.warm_key().to_owned();
    let call = Call {
        plan: Arc::new(plan),
        invocation,
        capture_logs: false,
        cancellation,
        reply,
        queue: None,
        settlement,
    };
    let pool = testing_start_inner(
        environments,
        PoolConfig::default(),
        vec![(key, environment)],
        true,
        Some(call),
    );
    let invocation = super::PoolInvocation {
        deadline,
        stop,
        response: Some(response),
        settlement: observed,
        failed: pool.port.failed.clone(),
        consumed: false,
    };
    (pool, invocation)
}
#[cfg(test)]
fn testing_start_inner(
    environments: EnvironmentPort,
    config: PoolConfig,
    warm: Vec<(String, EnvironmentHandle)>,
    failed_worker: bool,
    failed_call: Option<Call>,
) -> super::EnvironmentPool {
    let runtime = Handle::current();
    let maximum = usize::from(config.0.queue);
    let (sender, inbox) = mpsc::channel(maximum);
    let (stop, _) = watch::channel(false);
    let failed = Arc::new(AtomicBool::new(false));
    let (status, observed) = watch::channel(PoolStatus::default());
    let emergency = Arc::new(CleanupTask::default());
    let mut actor = Actor::builder()
        .environments(environments)
        .config(config)
        .inbox(inbox)
        .stop(stop.clone())
        .failed(failed.clone())
        .status(status)
        .emergency(emergency.clone())
        .runtime(runtime.clone())
        .warm(
            warm.into_iter()
                .map(|(key, environment)| Warm { key, environment })
                .collect(),
        )
        .build();
    if failed_worker {
        let warm = actor.warm.pop_front().unwrap();
        let receipt = warm.environment.retirement();
        let settlement = failed_call.as_ref().map(|call| call.settlement.clone());
        let job = actor.jobs.spawn(async move {
            let _owned = warm;
            let _call = failed_call;
            std::future::pending::<Completion>().await
        });
        actor.receipts.insert(
            job.id(),
            Record {
                retirement: receipt,
                settlement,
                failed: false,
            },
        );
        job.abort();
    }
    let port = super::PoolPort {
        sender,
        queue: Arc::new(tokio::sync::Semaphore::new(maximum)),
        maximum,
        stop,
        failed,
        status: observed,
    };
    super::EnvironmentPool {
        port,
        task: Some(runtime.spawn(actor.run())),
        primary: None,
        settled: None,
        emergency,
    }
}
