//! Read-only OS entropy jobs retain bounded permits through observer loss/results.

use std::time::Duration;

use tokio::{
    runtime::Handle,
    sync::{Semaphore, SemaphorePermit},
    task::{JoinHandle, spawn_blocking},
    time::{Instant, timeout_at},
};

use super::types::GatewayInstanceId;
use crate::probe::ProbeError;

static WORKERS: Semaphore = Semaphore::const_new(4);
type IdentityResult = (
    Result<GatewayInstanceId, ProbeError>,
    SemaphorePermit<'static>,
);

pub(super) async fn generate(deadline: Instant) -> Result<GatewayInstanceId, ProbeError> {
    owned(deadline, &WORKERS, GatewayInstanceId::generate).await
}
async fn owned(
    deadline: Instant,
    pool: &'static Semaphore,
    work: impl FnOnce() -> Result<GatewayInstanceId, ProbeError> + Send + 'static,
) -> Result<GatewayInstanceId, ProbeError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or(ProbeError::Deadline)?;
    if remaining > Duration::from_secs(60) {
        return Err(ProbeError::Configuration);
    }
    Handle::try_current().map_err(|_| ProbeError::Worker)?;
    let permit = timeout_at(deadline, pool.acquire())
        .await
        .map_err(|_| ProbeError::Deadline)?
        .map_err(|_| ProbeError::Worker)?;
    let job = spawn_identity(permit, deadline, work);
    // Expired/dropped observers discard their read-only result; the blocking job
    // retains its physical permit until OS return and ready-result disposal.
    let (result, _permit) = timeout_at(deadline, job)
        .await
        .map_err(|_| ProbeError::Deadline)?
        .map_err(|_| ProbeError::Worker)?;
    if Instant::now() >= deadline {
        return Err(ProbeError::Deadline);
    }
    result
}
fn spawn_identity(
    permit: SemaphorePermit<'static>,
    deadline: Instant,
    work: impl FnOnce() -> Result<GatewayInstanceId, ProbeError> + Send + 'static,
) -> JoinHandle<IdentityResult> {
    spawn_blocking(move || {
        let result = if Instant::now() >= deadline {
            Err(ProbeError::Deadline)
        } else {
            work()
        };
        let result = if Instant::now() >= deadline {
            Err(ProbeError::Deadline)
        } else {
            result
        };
        (result, permit)
    })
}
#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use tokio::{
        spawn,
        sync::{Semaphore, oneshot},
        task::yield_now,
        time::{Instant, timeout},
    };

    use super::{owned, spawn_identity};
    use crate::probe::{GatewayInstanceId, ProbeError};
    fn id() -> GatewayInstanceId {
        "aaaaaaaa-1111-4111-8111-111111111111".parse().unwrap()
    }
    #[tokio::test]
    async fn test_should_keep_entropy_work_charged_after_waiter_abandonment_and_queue_expiry() {
        static POOL: Semaphore = Semaphore::const_new(1);
        let (release, held) = mpsc::sync_channel(1);
        let (entered, saw) = oneshot::channel();
        let job = spawn(owned(
            Instant::now() + Duration::from_secs(5),
            &POOL,
            move || {
                entered.send(()).unwrap();
                held.recv().unwrap();
                Ok(id())
            },
        ));
        saw.await.unwrap();
        job.abort();
        assert!(job.await.unwrap_err().is_cancelled());
        assert_eq!(POOL.available_permits(), 0);
        assert!(matches!(
            owned(Instant::now() + Duration::from_millis(25), &POOL, || Ok(
                id()
            ))
            .await,
            Err(ProbeError::Deadline)
        ));
        assert_eq!(POOL.available_permits(), 0);
        release.send(()).unwrap();
        timeout(Duration::from_secs(2), async {
            while POOL.available_permits() != 1 {
                yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn test_should_hold_ready_entropy_result_until_observed_and_reject_expired_budget() {
        static POOL: Semaphore = Semaphore::const_new(1);
        let job = spawn_identity(
            POOL.acquire().await.unwrap(),
            Instant::now() + Duration::from_secs(5),
            || Ok(id()),
        );
        timeout(Duration::from_secs(2), async {
            while !job.is_finished() {
                yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(POOL.available_permits(), 0);
        let (result, permit) = job.await.unwrap();
        assert_eq!(result.unwrap(), id());
        drop(permit);
        assert_eq!(POOL.available_permits(), 1);
        assert!(matches!(
            owned(Instant::now(), &POOL, || Ok(id())).await,
            Err(ProbeError::Deadline)
        ));
        assert!(matches!(
            owned(Instant::now() + Duration::from_secs(61), &POOL, || Ok(id())).await,
            Err(ProbeError::Configuration)
        ));
    }
}
