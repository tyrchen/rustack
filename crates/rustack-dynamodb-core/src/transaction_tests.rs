//! Atomicity, isolation and full-window idempotency regressions.
use std::sync::{
    Barrier,
    atomic::{AtomicUsize, Ordering},
};

use super::*;

#[derive(Default)]
struct CountingEmitter(AtomicUsize);
impl crate::stream::StreamEmitter for CountingEmitter {
    fn emit(&self, _: crate::stream::ChangeEvent) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn provider() -> (RustackDynamoDB, Arc<CountingEmitter>) {
    let mut provider = RustackDynamoDB::new(DynamoDBConfig::default());
    let emitter = Arc::new(CountingEmitter::default());
    provider.set_emitter(emitter.clone());
    provider
        .handle_create_table(
            serde_json::from_value(serde_json::json!({
                "TableName":"TestTable", "KeySchema":[{"AttributeName":"pk","KeyType":"HASH"}],
                "AttributeDefinitions":[{"AttributeName":"pk","AttributeType":"S"}],
                "BillingMode":"PAY_PER_REQUEST",
                "StreamSpecification":{"StreamEnabled":true,"StreamViewType":"NEW_AND_OLD_IMAGES"}
            }))
            .unwrap(),
        )
        .unwrap();
    (provider, emitter)
}

#[test]
fn test_should_reserve_token_capacity_through_inflight_commit_and_replay() {
    struct PausedEmitter {
        entered: Barrier,
        release: Barrier,
        count: AtomicUsize,
    }
    impl crate::stream::StreamEmitter for PausedEmitter {
        fn emit(&self, _: crate::stream::ChangeEvent) {
            if self.count.fetch_add(1, Ordering::Relaxed) == 0 {
                self.entered.wait();
                self.release.wait();
            }
        }
    }
    let (mut provider, _) = provider();
    provider.token_capacity = 1;
    let emitter = Arc::new(PausedEmitter {
        entered: Barrier::new(2),
        release: Barrier::new(2),
        count: AtomicUsize::new(0),
    });
    provider.set_emitter(emitter.clone());
    std::thread::scope(|scope| {
        let first = scope.spawn(|| provider.handle_transact_write_items(increment("first")));
        emitter.entered.wait();
        let retry = scope.spawn(|| provider.handle_transact_write_items(increment("first")));
        let excess = scope.spawn(|| provider.handle_transact_write_items(increment("excess")));
        emitter.release.wait();
        first.join().unwrap().unwrap();
        retry.join().unwrap().unwrap();
        assert_eq!(
            excess.join().unwrap().unwrap_err().code,
            rustack_dynamodb_model::error::DynamoDBErrorCode::RequestLimitExceeded
        );
    });
    assert_eq!(emitter.count.load(Ordering::Relaxed), 1);
    assert_eq!(
        get(&provider, "counter").get("n"),
        Some(&AttributeValue::N("1".into()))
    );
}

#[test]
fn test_should_allow_only_one_competing_conditional_transaction() {
    let (provider, emitter) = provider();
    let start = Barrier::new(2);
    let winners = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| {
                start.wait();
                let request = transaction(serde_json::json!({"TransactItems":[{"Put":{
                    "TableName":"TestTable", "Item":{"pk":{"S":"unique"}},
                    "ConditionExpression":"attribute_not_exists(pk)"
                }}]}));
                if provider.handle_transact_write_items(request).is_ok() {
                    winners.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    assert_eq!(winners.load(Ordering::Relaxed), 1);
    assert_eq!(emitter.0.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_drain_cancelled_http_blocking_operation_before_snapshot() {
    use bytes::Bytes;
    use rustack_dynamodb_http::dispatch::DynamoDBHandler;
    use rustack_dynamodb_model::operations::DynamoDBOperation;

    use crate::handler::RustackDynamoDBHandler;
    struct PausedEmitter {
        entered: tokio::sync::Notify,
        release: Barrier,
    }
    impl crate::stream::StreamEmitter for PausedEmitter {
        fn emit(&self, _: crate::stream::ChangeEvent) {
            self.entered.notify_one();
            self.release.wait();
        }
    }
    let (mut provider, _) = provider();
    let emitter = Arc::new(PausedEmitter {
        entered: tokio::sync::Notify::new(),
        release: Barrier::new(2),
    });
    provider.set_emitter(emitter.clone());
    let provider = Arc::new(provider);
    let handler = RustackDynamoDBHandler::new(provider.clone());
    let request = tokio::spawn(handler.handle_operation(
        DynamoDBOperation::PutItem,
        Bytes::from_static(br#"{"TableName":"TestTable","Item":{"pk":{"S":"accepted"}}}"#),
    ));
    emitter.entered.notified().await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(
        tokio::time::timeout(Duration::from_millis(10), provider.quiesce())
            .await
            .is_err()
    );
    assert!(provider.requests.admit().is_err());
    emitter.release.wait();
    tokio::time::timeout(Duration::from_secs(1), provider.quiesce())
        .await
        .unwrap()
        .unwrap();
    let snapshot = provider.export_snapshot();
    assert_eq!(snapshot.tables[0].items.len(), 1);
    assert_eq!(
        snapshot.tables[0].items[0].get("pk"),
        Some(&AttributeValue::S("accepted".into()))
    );
    assert!(
        provider
            .handle_get_item(GetItemInput {
                table_name: "TestTable".into(),
                ..Default::default()
            })
            .is_err()
    );
}

#[test]
fn test_should_reject_late_batch_value_error_without_data_or_stream_commit() {
    let (provider, emitter) = provider();
    let input = serde_json::from_value(serde_json::json!({"RequestItems":{"TestTable":[
        {"PutRequest":{"Item":{"pk":{"S":"first"}}}},
        {"PutRequest":{"Item":{"pk":{"S":"second"},"n":{"N":"NaN"}}}}
    ]}}))
    .unwrap();
    assert!(provider.handle_batch_write_item(input).is_err());
    assert!(get(&provider, "first").is_empty());
    assert_eq!(emitter.0.load(Ordering::Relaxed), 0);
}

#[test]
fn test_should_discard_prepared_prefix_when_later_item_exceeds_size_limit() {
    let (provider, emitter) = provider();
    let input = transaction(serde_json::json!({"TransactItems":[
        {"Put":{"TableName":"TestTable","Item":{"pk":{"S":"first"}}}},
        {"Put":{"TableName":"TestTable","Item":{"pk":{"S":"second"},"payload":{"S":"x".repeat(400 * 1024)}}}}
    ]}));
    assert!(provider.handle_transact_write_items(input).is_err());
    assert!(get(&provider, "first").is_empty());
    assert_eq!(emitter.0.load(Ordering::Relaxed), 0);
}

fn transaction(value: serde_json::Value) -> TransactWriteItemsInput {
    serde_json::from_value(value).unwrap()
}

fn increment(token: &str) -> TransactWriteItemsInput {
    transaction(
        serde_json::json!({"ClientRequestToken":token,"TransactItems":[{"Update":{
            "TableName":"TestTable", "Key":{"pk":{"S":"counter"}},
            "UpdateExpression":"ADD n :one", "ExpressionAttributeValues":{":one":{"N":"1"}}
        }}]}),
    )
}

fn get(provider: &RustackDynamoDB, key: &str) -> HashMap<String, AttributeValue> {
    provider
        .handle_get_item(GetItemInput {
            table_name: "TestTable".into(),
            key: HashMap::from([("pk".into(), AttributeValue::S(key.into()))]),
            ..Default::default()
        })
        .unwrap()
        .item
        .unwrap_or_default()
}

#[test]
fn test_should_discard_all_prepared_writes_and_streams_on_late_errors() {
    for update in [
        "SET",
        "SET pk = :one",
        "SET n = missing + :one",
        "ADD n :one",
    ] {
        let (provider, emitter) = provider();
        let values = if update == "ADD n :one" {
            serde_json::json!({":one":{"S":"bad"}})
        } else if update == "SET" {
            serde_json::json!({})
        } else {
            serde_json::json!({":one":{"N":"1"}})
        };
        let request = transaction(serde_json::json!({"TransactItems":[
            {"Put":{"TableName":"TestTable","Item":{"pk":{"S":"first"}}}},
            {"Update":{"TableName":"TestTable","Key":{"pk":{"S":"second"}},
              "UpdateExpression":update,"ExpressionAttributeValues":values}}
        ]}));
        assert!(
            provider.handle_transact_write_items(request).is_err(),
            "{update}"
        );
        assert!(get(&provider, "first").is_empty());
        assert_eq!(emitter.0.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn test_should_preserve_successful_tokens_at_capacity_until_full_window_expires() {
    let (mut provider, emitter) = provider();
    provider.token_capacity = 1;
    provider
        .handle_transact_write_items(increment("first"))
        .unwrap();
    let err = provider
        .handle_transact_write_items(increment("second"))
        .unwrap_err();
    assert_eq!(
        err.code,
        rustack_dynamodb_model::error::DynamoDBErrorCode::RequestLimitExceeded
    );
    provider
        .handle_transact_write_items(increment("first"))
        .unwrap();
    assert_eq!(
        get(&provider, "counter").get("n"),
        Some(&AttributeValue::N("1".into()))
    );
    let mut mismatch = increment("first");
    mismatch.return_consumed_capacity =
        Some(rustack_dynamodb_model::types::ReturnConsumedCapacity::Total);
    assert_eq!(
        provider
            .handle_transact_write_items(mismatch)
            .unwrap_err()
            .code,
        rustack_dynamodb_model::error::DynamoDBErrorCode::IdempotentParameterMismatchException
    );
    assert_eq!(emitter.0.load(Ordering::Relaxed), 1);
    provider.tokens.get_mut("first").unwrap().completed_at =
        Instant::now().checked_sub(Duration::from_mins(10)).unwrap();
    provider
        .handle_transact_write_items(increment("first"))
        .unwrap();
    assert_eq!(
        get(&provider, "counter").get("n"),
        Some(&AttributeValue::N("2".into()))
    );
}

#[test]
fn test_should_execute_concurrent_same_token_only_once() {
    let (provider, emitter) = provider();
    let provider = Arc::new(provider);
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let provider = &provider;
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                provider
                    .handle_transact_write_items(increment("same"))
                    .unwrap();
            });
        }
    });
    assert_eq!(
        get(&provider, "counter").get("n"),
        Some(&AttributeValue::N("1".into()))
    );
    assert_eq!(emitter.0.load(Ordering::Relaxed), 1);
}

#[test]
fn test_should_coordinate_ordinary_conditional_writes_with_transactions() {
    let (provider, emitter) = provider();
    let barrier = Barrier::new(2);
    let winners = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            let result = provider.handle_transact_write_items(transaction(
                serde_json::json!({"TransactItems":[
                    {"Put":{"TableName":"TestTable","Item":{"pk":{"S":"unique"}},
                    "ConditionExpression":"attribute_not_exists(pk)"}}
                ]}),
            ));
            if result.is_ok() {
                winners.fetch_add(1, Ordering::Relaxed);
            }
        });
        scope.spawn(|| {
            barrier.wait();
            let result = provider.handle_put_item(
                serde_json::from_value(serde_json::json!({
                    "TableName":"TestTable","Item":{"pk":{"S":"unique"}},
                    "ConditionExpression":"attribute_not_exists(pk)"
                }))
                .unwrap(),
            );
            if result.is_ok() {
                winners.fetch_add(1, Ordering::Relaxed);
            }
        });
    });
    assert_eq!(winners.load(Ordering::Relaxed), 1);
    assert_eq!(emitter.0.load(Ordering::Relaxed), 1);
}

#[test]
fn test_should_never_observe_torn_transaction_reads() {
    let (provider, _) = provider();
    let write = |n| {
        transaction(serde_json::json!({"TransactItems":[
            {"Put":{"TableName":"TestTable","Item":{"pk":{"S":"a"},"n":{"N":format!("{n}")}}}},
            {"Put":{"TableName":"TestTable","Item":{"pk":{"S":"b"},"n":{"N":format!("{n}")}}}}
        ]}))
    };
    provider.handle_transact_write_items(write(0)).unwrap();
    let barrier = Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            for n in 1..100 {
                provider.handle_transact_write_items(write(n)).unwrap();
            }
        });
        scope.spawn(|| {
            barrier.wait();
            for _ in 0..100 {
                let output = provider
                    .handle_transact_get_items(
                        serde_json::from_value(serde_json::json!({"TransactItems":[
                            {"Get":{"TableName":"TestTable","Key":{"pk":{"S":"a"}}}},
                            {"Get":{"TableName":"TestTable","Key":{"pk":{"S":"b"}}}}
                        ]}))
                        .unwrap(),
                    )
                    .unwrap()
                    .responses
                    .unwrap();
                assert_eq!(
                    output[0].item.as_ref().unwrap().get("n"),
                    output[1].item.as_ref().unwrap().get("n")
                );
            }
        });
    });
}
