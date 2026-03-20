//! Integration tests for `RustStack` server (S3 + DynamoDB + SQS + SNS).
//!
//! These tests require a running `RustStack` server at `localhost:4566`.
//! They are marked `#[ignore]` so they don't run during normal `cargo test`.
//!
//! Run them with:
//! ```text
//! cargo test -p ruststack-integration -- --ignored
//! ```

use std::sync::Once;

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_sns as sns;
use aws_sdk_sqs as sqs;

static INIT: Once = Once::new();

/// Initialize tracing (once).
fn init_tracing() {
    INIT.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .with_test_writer()
            .init();
    });
}

/// Endpoint URL for the server.
fn endpoint_url() -> String {
    std::env::var("S3_ENDPOINT_URL").unwrap_or_else(|_| "http://localhost:4566".to_owned())
}

/// Create a configured S3 client pointing at the local server.
#[must_use]
pub fn s3_client() -> aws_sdk_s3::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .force_path_style(true)
        .build();

    aws_sdk_s3::Client::from_conf(config)
}

/// Create a configured DynamoDB client pointing at the local server.
#[must_use]
pub fn dynamodb_client() -> aws_sdk_dynamodb::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_dynamodb::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_dynamodb::Client::from_conf(config)
}

/// Generate a unique bucket name for a test.
#[must_use]
pub fn test_bucket_name(prefix: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
    format!("test-{prefix}-{id}")
}

/// Generate a unique table name for a DynamoDB test.
#[must_use]
pub fn test_table_name(prefix: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
    format!("test-{prefix}-{id}")
}

/// Create a bucket and return its name. Caller is responsible for cleanup.
pub async fn create_test_bucket(client: &aws_sdk_s3::Client, prefix: &str) -> String {
    let name = test_bucket_name(prefix);
    client
        .create_bucket()
        .bucket(&name)
        .send()
        .await
        .unwrap_or_else(|e| panic!("failed to create bucket {name}: {e}"));
    name
}

/// Delete all objects in a bucket, then delete the bucket.
pub async fn cleanup_bucket(client: &aws_sdk_s3::Client, bucket: &str) {
    // List and delete all objects.
    let mut continuation_token = None;
    loop {
        let mut req = client.list_objects_v2().bucket(bucket);
        if let Some(token) = continuation_token.take() {
            req = req.continuation_token(token);
        }
        let Ok(resp) = req.send().await else {
            return; // Bucket may not exist.
        };

        for obj in resp.contents() {
            if let Some(key) = obj.key() {
                let _ = client.delete_object().bucket(bucket).key(key).send().await;
            }
        }

        if resp.is_truncated() == Some(true) {
            continuation_token = resp.next_continuation_token().map(ToOwned::to_owned);
        } else {
            break;
        }
    }

    // Delete any in-progress multipart uploads.
    if let Ok(uploads) = client.list_multipart_uploads().bucket(bucket).send().await {
        for upload in uploads.uploads() {
            if let (Some(key), Some(id)) = (upload.key(), upload.upload_id()) {
                let _ = client
                    .abort_multipart_upload()
                    .bucket(bucket)
                    .key(key)
                    .upload_id(id)
                    .send()
                    .await;
            }
        }
    }

    let _ = client.delete_bucket().bucket(bucket).send().await;
}

/// Create a configured SQS client pointing at the local server.
#[must_use]
pub fn sqs_client() -> sqs::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = sqs::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    sqs::Client::from_conf(config)
}

/// Generate a unique queue name for an SQS test.
#[must_use]
pub fn test_queue_name(prefix: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
    format!("test-{prefix}-{id}")
}

/// Create a standard queue and return its URL. Caller is responsible for cleanup.
pub async fn create_test_queue(client: &sqs::Client, prefix: &str) -> (String, String) {
    let name = test_queue_name(prefix);
    let url = client
        .create_queue()
        .queue_name(&name)
        .send()
        .await
        .unwrap_or_else(|e| panic!("failed to create queue {name}: {e}"))
        .queue_url()
        .unwrap()
        .to_string();
    (name, url)
}

/// Delete a queue by URL.
pub async fn delete_test_queue(client: &sqs::Client, queue_url: &str) {
    let _ = client.delete_queue().queue_url(queue_url).send().await;
}

/// Create a configured SSM client pointing at the local server.
#[must_use]
pub fn ssm_client() -> aws_sdk_ssm::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_ssm::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_ssm::Client::from_conf(config)
}

/// Create a configured SNS client pointing at the local server.
#[must_use]
pub fn sns_client() -> sns::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = sns::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    sns::Client::from_conf(config)
}

/// Create a configured Lambda client pointing at the local server.
#[must_use]
pub fn lambda_client() -> aws_sdk_lambda::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_lambda::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_lambda::Client::from_conf(config)
}

/// Create a configured EventBridge client pointing at the local server.
#[must_use]
pub fn events_client() -> aws_sdk_eventbridge::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_eventbridge::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_eventbridge::Client::from_conf(config)
}

/// Create a configured CloudWatch Logs client pointing at the local server.
#[must_use]
pub fn logs_client() -> aws_sdk_cloudwatchlogs::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_cloudwatchlogs::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_cloudwatchlogs::Client::from_conf(config)
}

/// Create a configured KMS client pointing at the local server.
#[must_use]
pub fn kms_client() -> aws_sdk_kms::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_kms::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_kms::Client::from_conf(config)
}

/// Create a configured Kinesis client pointing at the local server.
#[must_use]
pub fn kinesis_client() -> aws_sdk_kinesis::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_kinesis::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_kinesis::Client::from_conf(config)
}

/// Generate a unique stream name for a Kinesis test.
#[must_use]
pub fn test_stream_name(prefix: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
    format!("test-{prefix}-{id}")
}

/// Create a configured Secrets Manager client pointing at the local server.
#[must_use]
pub fn secretsmanager_client() -> aws_sdk_secretsmanager::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_secretsmanager::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_secretsmanager::Client::from_conf(config)
}

/// Generate a unique secret name for a Secrets Manager test.
#[must_use]
pub fn test_secret_name(prefix: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
    format!("test/{prefix}/{id}")
}

/// Create a configured SES client pointing at the local server.
#[must_use]
pub fn ses_client() -> aws_sdk_ses::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_ses::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_ses::Client::from_conf(config)
}

/// Create a configured API Gateway v2 client pointing at the local server.
#[must_use]
pub fn apigatewayv2_client() -> aws_sdk_apigatewayv2::Client {
    init_tracing();

    let creds = Credentials::new("test", "test", None, None, "integration-test");

    let config = aws_sdk_apigatewayv2::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .endpoint_url(endpoint_url())
        .build();

    aws_sdk_apigatewayv2::Client::from_conf(config)
}

mod test_apigatewayv2;
mod test_bucket;
mod test_cors;
mod test_dynamodb;
mod test_error;
mod test_events;
mod test_health;
mod test_kinesis;
mod test_kms;
mod test_lambda;
mod test_list;
mod test_logs;
mod test_multipart;
mod test_object;
mod test_precondition;
mod test_secretsmanager;
mod test_ses;
mod test_sns;
mod test_sqs;
mod test_ssm;
mod test_versioning;
