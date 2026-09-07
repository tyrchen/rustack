//! Bounded acceptance, real worker lifecycle, and explicit failure tests.
use super::*;
use crate::{config::EventsConfig, provider::RustackEvents};

#[derive(Debug)]
struct Recording(mpsc::Sender<String>);
#[async_trait]
impl TargetDelivery for Recording {
    fn validate(&self, _: &Target) -> Result<(), DeliveryError> {
        Ok(())
    }
    async fn deliver(&self, _: &Target, body: &str) -> Result<(), DeliveryError> {
        self.0
            .send(body.to_owned())
            .await
            .map_err(|_| DeliveryError::Unavailable("Recorder closed".into()))
    }
}
fn target() -> Target {
    serde_json::from_value(serde_json::json!({"Id":"target","Arn":"arn:aws:sqs:us-east-1:000000000000:q.fifo","SqsParameters":{"MessageGroupId":"g"}})).unwrap()
}
fn job(body: &str) -> Vec<DeliveryJob> {
    vec![DeliveryJob {
        target: target(),
        body: body.into(),
    }]
}

#[tokio::test]
async fn test_should_resume_waiting_for_drain_after_quiesce_future_is_cancelled() {
    let (sender, mut receiver) = mpsc::channel(1);
    sender.send("occupied".into()).await.unwrap();
    let queue = DeliveryQueue::new(Arc::new(Recording(sender)));
    queue.submit(job("accepted")).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(1), queue.quiesce())
            .await
            .is_err()
    );
    assert!(queue.submit(job("late")).is_err());
    assert_eq!(receiver.recv().await.as_deref(), Some("occupied"));
    tokio::time::timeout(Duration::from_secs(1), queue.quiesce())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receiver.recv().await.as_deref(), Some("accepted"));
    assert_eq!(queue.stats().delivered, 1);
}

#[tokio::test]
async fn test_should_deliver_in_order_and_reject_after_quiesce() {
    let (sender, mut receiver) = mpsc::channel(8);
    let queue = DeliveryQueue::new(Arc::new(Recording(sender)));
    for n in 0..5 {
        queue.submit(job(&n.to_string())).unwrap();
    }
    queue.quiesce().await.unwrap();
    queue.quiesce().await.unwrap();
    for n in 0..5 {
        assert_eq!(receiver.recv().await.unwrap(), n.to_string());
    }
    assert!(queue.submit(job("late")).is_err());
    assert_eq!(queue.stats().delivered, 5);
    assert_eq!(queue.stats().failed, 0);
}

#[tokio::test]
async fn test_should_bound_acceptance_and_quiesce_even_when_channel_was_full() {
    let (sender, _receiver) = mpsc::channel(1);
    let queue = DeliveryQueue::new(Arc::new(Recording(sender)));
    // This current-thread task does not yield, so the worker has not consumed a slot.
    for _ in 0..128 {
        queue.submit(Vec::new()).unwrap();
    }
    assert!(queue.submit(Vec::new()).is_err());
    tokio::time::timeout(Duration::from_secs(1), queue.quiesce())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queue.stats().rejected, 1);
}

#[tokio::test]
async fn test_should_record_unavailable_delivery_as_failed_not_delivered() {
    let queue = DeliveryQueue::new(Arc::new(UnavailableTargetDelivery));
    queue.submit(job("event")).unwrap();
    queue.quiesce().await.unwrap();
    assert_eq!(queue.stats().accepted, 1);
    assert_eq!(queue.stats().delivered, 0);
    assert_eq!(queue.stats().failed, 1);
}

#[test]
fn test_should_allow_metadata_targets_without_runtime_dependency() {
    let provider = RustackEvents::new(EventsConfig::default(), Arc::new(UnavailableTargetDelivery));
    provider
        .handle_put_rule(
            serde_json::from_value(serde_json::json!({"Name":"rule","EventPattern":"{}"})).unwrap(),
        )
        .unwrap();
    let output = provider
        .handle_put_targets(
            serde_json::from_value(serde_json::json!({"Rule":"rule","Targets":[target()]}))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        output.failed_entry_count, 0,
        "metadata configuration must not require a runtime delivery bridge"
    );
    let listed = provider
        .handle_list_targets_by_rule(
            &serde_json::from_value(serde_json::json!({"Rule":"rule"})).unwrap(),
        )
        .unwrap();
    assert_eq!(listed.targets.len(), 1);
    // Execution support is the bridge's contract: unavailable delivery is an
    // explicit terminal failure, never a silent configuration success.
    let unavailable = UnavailableTargetDelivery;
    assert!(
        unavailable
            .validate(
                &serde_json::from_value(
                    serde_json::json!({"Id":"t","Arn":"arn:aws:sqs:us-east-1:000000000000:q"})
                )
                .unwrap()
            )
            .is_err()
    );
}

#[derive(Debug)]
struct Panicking;
#[async_trait]
impl TargetDelivery for Panicking {
    fn validate(&self, _: &Target) -> Result<(), DeliveryError> {
        Ok(())
    }
    async fn deliver(&self, _: &Target, _: &str) -> Result<(), DeliveryError> {
        panic!("injected bridge panic")
    }
}
#[tokio::test]
async fn test_should_observe_bridge_panic_and_keep_worker_drainable() {
    let queue = DeliveryQueue::new(Arc::new(Panicking));
    queue.submit(job("event")).unwrap();
    queue.quiesce().await.unwrap();
    assert_eq!(queue.stats().failed, 1);
}

#[tokio::test]
async fn test_should_observe_worker_crash_and_report_not_ready() {
    let queue = DeliveryQueue::new(Arc::new(Panicking));
    queue.submit(Vec::new()).unwrap();
    queue.inject_crash();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while queue.is_ready() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!queue.is_ready(), "crashed worker must report not ready");
    assert!(queue.submit(job("late")).is_err());
    let error = tokio::time::timeout(Duration::from_secs(2), queue.quiesce())
        .await
        .unwrap()
        .expect_err("quiesce must surface the worker crash");
    assert!(matches!(error, DeliveryError::TargetError(_)));
}

#[tokio::test]
async fn test_should_timeout_a_stalled_target_with_observable_terminal_failure() {
    let (sender, _receiver) = mpsc::channel(1);
    sender.send("occupy".into()).await.unwrap();
    let queue = DeliveryQueue::new(Arc::new(Recording(sender)));
    queue.submit(job("event")).unwrap();
    tokio::time::timeout(Duration::from_secs(6), queue.quiesce())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queue.stats().failed, 1);
    assert_eq!(queue.stats().delivered, 0);
}
