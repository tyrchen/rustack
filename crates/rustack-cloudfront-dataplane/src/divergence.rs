//! Divergence signalling helpers.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use tracing::warn;

use crate::error::DataPlaneError;

/// Track divergence warnings to rate-limit repeats.
#[derive(Debug, Default)]
pub struct DivergenceTracker {
    seen: DashMap<String, Instant>,
    interval: Duration,
}

impl DivergenceTracker {
    /// Create a new tracker with the given minimum log interval.
    #[must_use]
    pub fn new(interval: Duration) -> Arc<Self> {
        Arc::new(Self {
            seen: DashMap::new(),
            interval,
        })
    }

    /// Returns true if the caller should emit the warning now.
    pub fn should_log(&self, key: &str) -> bool {
        let now = Instant::now();
        match self.seen.get_mut(key) {
            Some(last) if now.duration_since(*last.value()) < self.interval => false,
            Some(mut last) => {
                *last.value_mut() = now;
                true
            }
            None => {
                self.seen.insert(key.to_owned(), now);
                true
            }
        }
    }
}

/// Emit or fail on Lambda\@Edge association hits.
pub fn handle_lambda_edge(
    tracker: &DivergenceTracker,
    fail: bool,
    distribution_id: &str,
) -> Result<(), DataPlaneError> {
    if fail {
        return Err(DataPlaneError::FunctionExecutionSkipped(format!(
            "Lambda@Edge would have executed but CLOUDFRONT_FAIL_ON_FUNCTION=true \
             ({distribution_id})"
        )));
    }
    let key = format!("{distribution_id}:lambda-edge");
    if tracker.should_log(&key) {
        warn!(
            distribution_id = %distribution_id,
            "Lambda@Edge association skipped — Rustack does not execute edge functions"
        );
    }
    Ok(())
}

/// Emit or fail on CloudFront Function association hits.
pub fn handle_function(
    tracker: &DivergenceTracker,
    fail: bool,
    distribution_id: &str,
) -> Result<(), DataPlaneError> {
    if fail {
        return Err(DataPlaneError::FunctionExecutionSkipped(format!(
            "CloudFront Function would have executed but CLOUDFRONT_FAIL_ON_FUNCTION=true \
             ({distribution_id})"
        )));
    }
    let key = format!("{distribution_id}:cf-function");
    if tracker.should_log(&key) {
        warn!(
            distribution_id = %distribution_id,
            "CloudFront Function association skipped — Rustack does not execute functions"
        );
    }
    Ok(())
}

/// Emit or fail on `ViewerProtocolPolicy` that requires HTTPS — Rustack is HTTP-only.
pub fn handle_viewer_protocol(
    tracker: &DivergenceTracker,
    fail: bool,
    distribution_id: &str,
    policy: &str,
) -> Result<(), DataPlaneError> {
    let requires_https = matches!(policy, "https-only" | "redirect-to-https");
    if !requires_https {
        return Ok(());
    }
    if fail {
        if policy == "https-only" {
            return Err(DataPlaneError::BehaviorResolution(format!(
                "ViewerProtocolPolicy is 'https-only' but Rustack is HTTP-only ({distribution_id})"
            )));
        }
        // redirect-to-https under FAIL_ON_FUNCTION: upgrade is a no-op locally.
    }
    let key = format!("{distribution_id}:vpp-{policy}");
    if tracker.should_log(&key) {
        warn!(
            distribution_id = %distribution_id,
            policy = %policy,
            "ViewerProtocolPolicy requires HTTPS but Rustack is HTTP-only — serving over HTTP"
        );
    }
    Ok(())
}

/// Emit or fail on signed-URL protected behaviors.
pub fn handle_signed_url(
    tracker: &DivergenceTracker,
    fail: bool,
    distribution_id: &str,
) -> Result<(), DataPlaneError> {
    if fail {
        return Err(DataPlaneError::SignedUrlRequired(format!(
            "Behavior requires signed URL but Rustack does not verify signatures \
             ({distribution_id})"
        )));
    }
    let key = format!("{distribution_id}:signed-url");
    if tracker.should_log(&key) {
        warn!(
            distribution_id = %distribution_id,
            "Signed-URL-required behavior — Rustack does not verify signatures"
        );
    }
    Ok(())
}
