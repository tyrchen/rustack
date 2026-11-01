//! CPU proposals never own resource effects; the actor commits after control checks.

use chrono::Utc;
use tokio::time::Instant;

use super::{
    super::{ClockSample, DriverStep},
    ExecutionError,
    actor::Actor,
};
use crate::{AttemptId, ExecutionDriver, Payload, SystemUuidSource, TaskFailure};

#[derive(Debug)]
pub(super) enum Operation {
    Advance,
    Complete {
        attempt: AttemptId,
        result: Result<Payload, TaskFailure>,
        observed: ClockSample,
    },
}
#[derive(Debug)]
pub(super) struct Evaluated {
    pub(super) proposed: ExecutionDriver,
    pub(super) step: DriverStep,
    pub(super) sample: ClockSample,
}
impl Actor {
    pub(super) async fn evaluate(
        &mut self,
        operation: Operation,
    ) -> Result<Option<DriverStep>, ExecutionError> {
        let Some(permit) = self.evaluation_permit().await else {
            return Ok(None);
        };
        let mut proposed = self.driver.clone();
        #[cfg(test)]
        let hook = self.worker_hook();
        self.start_worker(move || {
            #[cfg(test)]
            if let Some(hook) = &hook {
                hook.run(true);
            }
            let _permit = permit;
            let sample = ClockSample::new(Instant::now().into_std(), Utc::now());
            let mut source = SystemUuidSource::default();
            let update = match operation {
                Operation::Advance => proposed.advance(sample, &mut source),
                Operation::Complete {
                    attempt,
                    result,
                    observed,
                } => proposed.complete(attempt, result, observed, &mut source),
            };
            #[cfg(test)]
            if let Some(hook) = &hook {
                hook.run(false);
            }
            Evaluated {
                proposed,
                step: update.step,
                sample,
            }
        });
        Ok(self
            .evaluated()
            .await?
            .and_then(|result| self.commit(result)))
    }
}
