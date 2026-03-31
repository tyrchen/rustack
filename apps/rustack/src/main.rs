//! Rustack Server - High-performance AWS-compatible server.
//!
//! This binary provides a LocalStack-compatible server that can host one or
//! more AWS services. A gateway layer routes requests to the appropriate
//! service based on request headers.
//!
//! # Usage
//!
//! ```text
//! GATEWAY_LISTEN=0.0.0.0:4566 rustack
//! ```
//!
//! # Environment Variables
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address |
//! | `SERVICES` | *(empty = all)* | Comma-separated list of services to enable |
//! | `<SERVICE>_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 verification for `<SERVICE>` |
//! | `S3_DOMAIN` | `s3.localhost.localstack.cloud` | Virtual hosting domain |
//! | `LOG_LEVEL` | `info` | Log level filter |
//! | `RUST_LOG` | *(unset)* | Fine-grained tracing filter (overrides `LOG_LEVEL`) |

#[cfg(feature = "events")]
mod events_bridge;
mod gateway;
#[cfg(feature = "s3")]
mod handler;
mod service;
#[cfg(feature = "sns")]
mod sns_bridge;

use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as HttpConnBuilder,
};
#[cfg(feature = "apigatewayv2")]
use rustack_apigatewayv2_core::config::ApiGatewayV2Config;
#[cfg(feature = "apigatewayv2")]
use rustack_apigatewayv2_core::handler::RustackApiGatewayV2Handler;
#[cfg(feature = "apigatewayv2")]
use rustack_apigatewayv2_core::provider::RustackApiGatewayV2;
#[cfg(feature = "apigatewayv2")]
use rustack_apigatewayv2_http::service::{ApiGatewayV2HttpConfig, ApiGatewayV2HttpService};
#[cfg(feature = "cloudwatch")]
use rustack_cloudwatch_core::config::CloudWatchConfig;
#[cfg(feature = "cloudwatch")]
use rustack_cloudwatch_core::handler::RustackCloudWatchHandler;
#[cfg(feature = "cloudwatch")]
use rustack_cloudwatch_core::provider::RustackCloudWatch;
#[cfg(feature = "cloudwatch")]
use rustack_cloudwatch_http::service::{CloudWatchHttpConfig, CloudWatchHttpService};
#[cfg(feature = "dynamodb")]
use rustack_dynamodb_core::config::DynamoDBConfig;
#[cfg(feature = "dynamodb")]
use rustack_dynamodb_core::handler::RustackDynamoDBHandler;
#[cfg(feature = "dynamodb")]
use rustack_dynamodb_core::provider::RustackDynamoDB;
#[cfg(feature = "dynamodb")]
use rustack_dynamodb_http::service::{DynamoDBHttpConfig, DynamoDBHttpService};
#[cfg(feature = "dynamodbstreams")]
use rustack_dynamodbstreams_core::config::DynamoDBStreamsConfig;
#[cfg(feature = "dynamodbstreams")]
use rustack_dynamodbstreams_core::emitter::{
    DynamoDBStreamEmitter, DynamoDBStreamLifecycleManager,
};
#[cfg(feature = "dynamodbstreams")]
use rustack_dynamodbstreams_core::handler::RustackDynamoDBStreamsHandler;
#[cfg(feature = "dynamodbstreams")]
use rustack_dynamodbstreams_core::provider::RustackDynamoDBStreams;
#[cfg(feature = "dynamodbstreams")]
use rustack_dynamodbstreams_core::storage::StreamStore;
#[cfg(feature = "dynamodbstreams")]
use rustack_dynamodbstreams_http::service::{
    DynamoDBStreamsHttpConfig, DynamoDBStreamsHttpService,
};
#[cfg(feature = "events")]
use rustack_events_core::config::EventsConfig;
#[cfg(feature = "events")]
use rustack_events_core::handler::RustackEventsHandler;
#[cfg(feature = "events")]
use rustack_events_core::provider::RustackEvents;
#[cfg(feature = "events")]
use rustack_events_http::service::{EventsHttpConfig, EventsHttpService};
#[cfg(feature = "iam")]
use rustack_iam_core::config::IamConfig;
#[cfg(feature = "iam")]
use rustack_iam_core::handler::RustackIamHandler;
#[cfg(feature = "iam")]
use rustack_iam_core::provider::RustackIam;
#[cfg(feature = "iam")]
use rustack_iam_core::store::IamStore;
#[cfg(feature = "iam")]
use rustack_iam_http::service::{IamHttpConfig, IamHttpService};
#[cfg(feature = "kinesis")]
use rustack_kinesis_core::config::KinesisConfig;
#[cfg(feature = "kinesis")]
use rustack_kinesis_core::handler::RustackKinesisHandler;
#[cfg(feature = "kinesis")]
use rustack_kinesis_core::provider::RustackKinesis;
#[cfg(feature = "kinesis")]
use rustack_kinesis_http::service::{KinesisHttpConfig, KinesisHttpService};
#[cfg(feature = "kms")]
use rustack_kms_core::config::KmsConfig;
#[cfg(feature = "kms")]
use rustack_kms_core::handler::RustackKmsHandler;
#[cfg(feature = "kms")]
use rustack_kms_core::provider::RustackKms;
#[cfg(feature = "kms")]
use rustack_kms_http::service::{KmsHttpConfig, KmsHttpService};
#[cfg(feature = "lambda")]
use rustack_lambda_core::config::LambdaConfig;
#[cfg(feature = "lambda")]
use rustack_lambda_core::handler::RustackLambdaHandler;
#[cfg(feature = "lambda")]
use rustack_lambda_core::provider::RustackLambda;
#[cfg(feature = "lambda")]
use rustack_lambda_http::service::{LambdaHttpConfig, LambdaHttpService};
#[cfg(feature = "logs")]
use rustack_logs_core::config::LogsConfig;
#[cfg(feature = "logs")]
use rustack_logs_core::handler::RustackLogsHandler;
#[cfg(feature = "logs")]
use rustack_logs_core::provider::RustackLogs;
#[cfg(feature = "logs")]
use rustack_logs_http::service::{LogsHttpConfig, LogsHttpService};
#[cfg(feature = "s3")]
use rustack_s3_core::{RustackS3, S3Config};
#[cfg(feature = "s3")]
use rustack_s3_http::service::{S3HttpConfig, S3HttpService};
#[cfg(feature = "secretsmanager")]
use rustack_secretsmanager_core::config::SecretsManagerConfig;
#[cfg(feature = "secretsmanager")]
use rustack_secretsmanager_core::handler::RustackSecretsManagerHandler;
#[cfg(feature = "secretsmanager")]
use rustack_secretsmanager_core::provider::RustackSecretsManager;
#[cfg(feature = "secretsmanager")]
use rustack_secretsmanager_http::service::{SecretsManagerHttpConfig, SecretsManagerHttpService};
#[cfg(feature = "ses")]
use rustack_ses_core::config::SesConfig;
#[cfg(feature = "ses")]
use rustack_ses_core::handler::RustackSesHandler;
#[cfg(feature = "ses")]
use rustack_ses_core::provider::RustackSes;
#[cfg(feature = "ses")]
use rustack_ses_http::service::{SesHttpConfig, SesHttpService};
#[cfg(feature = "ses")]
use rustack_ses_http::v2::SesV2HttpService;
#[cfg(feature = "sns")]
use rustack_sns_core::config::SnsConfig;
#[cfg(feature = "sns")]
use rustack_sns_core::handler::RustackSnsHandler;
#[cfg(feature = "sns")]
use rustack_sns_core::provider::RustackSns;
#[cfg(feature = "sns")]
use rustack_sns_http::service::{SnsHttpConfig, SnsHttpService};
#[cfg(feature = "sqs")]
use rustack_sqs_core::config::SqsConfig;
#[cfg(feature = "sqs")]
use rustack_sqs_core::handler::RustackSqsHandler;
#[cfg(feature = "sqs")]
use rustack_sqs_core::provider::RustackSqs;
#[cfg(feature = "sqs")]
use rustack_sqs_http::service::{SqsHttpConfig, SqsHttpService};
#[cfg(feature = "ssm")]
use rustack_ssm_core::config::SsmConfig;
#[cfg(feature = "ssm")]
use rustack_ssm_core::handler::RustackSsmHandler;
#[cfg(feature = "ssm")]
use rustack_ssm_core::provider::RustackSsm;
#[cfg(feature = "ssm")]
use rustack_ssm_http::service::{SsmHttpConfig, SsmHttpService};
#[cfg(feature = "sts")]
use rustack_sts_core::config::StsConfig;
#[cfg(feature = "sts")]
use rustack_sts_core::handler::RustackStsHandler;
#[cfg(feature = "sts")]
use rustack_sts_core::provider::RustackSts;
#[cfg(feature = "sts")]
use rustack_sts_http::service::{StsHttpConfig, StsHttpService};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[cfg(feature = "events")]
use crate::events_bridge::LocalTargetDelivery;
#[cfg(feature = "sns")]
use crate::sns_bridge::RustackSqsPublisher;
use crate::{gateway::GatewayService, service::ServiceRouter};

/// Server version reported in health check responses.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the tracing subscriber.
///
/// Uses `RUST_LOG` if set, otherwise falls back to the `LOG_LEVEL` config value.
fn init_tracing(log_level: &str) -> Result<()> {
    let filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else {
        EnvFilter::try_new(log_level)
            .with_context(|| format!("invalid log level filter: {log_level}"))?
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();

    Ok(())
}

/// Build the [`S3HttpConfig`] from the application [`S3Config`].
#[cfg(feature = "s3")]
fn build_s3_http_config(config: &S3Config) -> S3HttpConfig {
    let credential_provider = build_credential_provider();

    S3HttpConfig {
        domain: config.s3_domain.clone(),
        virtual_hosting: config.s3_virtual_hosting,
        skip_signature_validation: config.s3_skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider: credential_provider.clone(),
    }
}

/// Build the [`DynamoDBHttpConfig`] from the [`DynamoDBConfig`].
#[cfg(feature = "dynamodb")]
fn build_dynamodb_http_config(config: &DynamoDBConfig) -> DynamoDBHttpConfig {
    let credential_provider = build_credential_provider();

    DynamoDBHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`SqsHttpConfig`] from the [`SqsConfig`].
#[cfg(feature = "sqs")]
fn build_sqs_http_config(config: &SqsConfig) -> SqsHttpConfig {
    let credential_provider = build_credential_provider();

    SqsHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`SsmHttpConfig`] from the [`SsmConfig`].
#[cfg(feature = "ssm")]
fn build_ssm_http_config(config: &SsmConfig) -> SsmHttpConfig {
    let credential_provider = build_credential_provider();

    SsmHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`SnsHttpConfig`] from the [`SnsConfig`].
#[cfg(feature = "sns")]
fn build_sns_http_config(config: &SnsConfig) -> SnsHttpConfig {
    let credential_provider = build_credential_provider();

    SnsHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`LambdaHttpConfig`] from the [`LambdaConfig`].
#[cfg(feature = "lambda")]
fn build_lambda_http_config(config: &LambdaConfig) -> LambdaHttpConfig {
    let credential_provider = build_credential_provider();

    LambdaHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`EventsHttpConfig`] from the [`EventsConfig`].
#[cfg(feature = "events")]
fn build_events_http_config(config: &EventsConfig) -> EventsHttpConfig {
    let credential_provider = build_credential_provider();

    EventsHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`LogsHttpConfig`] from the [`LogsConfig`].
#[cfg(feature = "logs")]
fn build_logs_http_config(config: &LogsConfig) -> LogsHttpConfig {
    let credential_provider = build_credential_provider();

    LogsHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`KmsHttpConfig`] from the [`KmsConfig`].
#[cfg(feature = "kms")]
fn build_kms_http_config(config: &KmsConfig) -> KmsHttpConfig {
    let credential_provider = build_credential_provider();

    KmsHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`KinesisHttpConfig`] from the [`KinesisConfig`].
#[cfg(feature = "kinesis")]
fn build_kinesis_http_config(config: &KinesisConfig) -> KinesisHttpConfig {
    let credential_provider = build_credential_provider();

    KinesisHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`SecretsManagerHttpConfig`] from the [`SecretsManagerConfig`].
#[cfg(feature = "secretsmanager")]
fn build_secretsmanager_http_config(config: &SecretsManagerConfig) -> SecretsManagerHttpConfig {
    let credential_provider = build_credential_provider();

    SecretsManagerHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`SesHttpConfig`] from the [`SesConfig`].
#[cfg(feature = "ses")]
fn build_ses_http_config(config: &SesConfig) -> SesHttpConfig {
    let credential_provider = build_credential_provider();

    SesHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`ApiGatewayV2HttpConfig`] from the [`ApiGatewayV2Config`].
#[cfg(feature = "apigatewayv2")]
fn build_apigatewayv2_http_config(config: &ApiGatewayV2Config) -> ApiGatewayV2HttpConfig {
    let credential_provider = build_credential_provider();

    ApiGatewayV2HttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`DynamoDBStreamsHttpConfig`] from the [`DynamoDBStreamsConfig`].
#[cfg(feature = "dynamodbstreams")]
fn build_dynamodbstreams_http_config(config: &DynamoDBStreamsConfig) -> DynamoDBStreamsHttpConfig {
    let credential_provider = build_credential_provider();

    DynamoDBStreamsHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`CloudWatchHttpConfig`] from the [`CloudWatchConfig`].
#[cfg(feature = "cloudwatch")]
fn build_cloudwatch_http_config(config: &CloudWatchConfig) -> CloudWatchHttpConfig {
    let credential_provider = build_credential_provider();

    CloudWatchHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build the [`IamHttpConfig`] from the [`IamConfig`].
///
/// IAM is a global service so we default the region to `us-east-1`.
#[cfg(feature = "iam")]
fn build_iam_http_config(config: &IamConfig) -> IamHttpConfig {
    let credential_provider = build_credential_provider();

    IamHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: "us-east-1".to_owned(),
        credential_provider,
    }
}

/// Build the [`StsHttpConfig`] from the [`StsConfig`].
#[cfg(feature = "sts")]
fn build_sts_http_config(config: &StsConfig) -> StsHttpConfig {
    let credential_provider = build_credential_provider();

    StsHttpConfig {
        skip_signature_validation: config.skip_signature_validation,
        region: config.default_region.clone(),
        credential_provider,
    }
}

/// Build a credential provider from `ACCESS_KEY` / `SECRET_KEY` environment
/// variables (used by MinIO Mint and other test harnesses).
#[cfg(any(
    feature = "s3",
    feature = "dynamodb",
    feature = "sqs",
    feature = "ssm",
    feature = "sns",
    feature = "lambda",
    feature = "events",
    feature = "logs",
    feature = "kms",
    feature = "kinesis",
    feature = "secretsmanager",
    feature = "ses",
    feature = "apigatewayv2",
    feature = "cloudwatch",
    feature = "dynamodbstreams",
    feature = "iam",
    feature = "sts"
))]
fn build_credential_provider() -> Option<Arc<dyn rustack_auth::CredentialProvider>> {
    use rustack_auth::StaticCredentialProvider;

    let access_key = std::env::var("ACCESS_KEY")
        .or_else(|_| std::env::var("AWS_ACCESS_KEY_ID"))
        .ok()?;
    let secret_key = std::env::var("SECRET_KEY")
        .or_else(|_| std::env::var("AWS_SECRET_ACCESS_KEY"))
        .ok()?;

    info!(
        access_key = %access_key,
        "configured credential provider from environment"
    );

    Some(Arc::new(StaticCredentialProvider::new(vec![(
        access_key, secret_key,
    )])))
}

/// Run the accept loop, serving connections until a shutdown signal is received.
async fn serve(listener: TcpListener, service: GatewayService) -> Result<()> {
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let http = HttpConnBuilder::new(TokioExecutor::new());

    let shutdown = async {
        tokio::signal::ctrl_c().await.ok();
        info!("received shutdown signal, draining connections");
    };

    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, peer_addr) = match result {
                    Ok(conn) => conn,
                    Err(e) => {
                        warn!(error = %e, "failed to accept connection");
                        continue;
                    }
                };

                let svc = service.clone();
                let conn = http.serve_connection(TokioIo::new(stream), svc);
                let conn = graceful.watch(conn.into_owned());

                tokio::spawn(async move {
                    if let Err(e) = conn.await {
                        warn!(peer_addr = %peer_addr, error = %e, "connection error");
                    }
                });
            }

            () = &mut shutdown => {
                info!("shutting down gracefully");
                break;
            }
        }
    }

    // Wait for in-flight requests to complete.
    graceful.shutdown().await;
    info!("all connections drained, exiting");

    Ok(())
}

/// Check whether a service name was compiled into this binary.
fn is_compiled_in(name: &str) -> bool {
    (name == "s3" && cfg!(feature = "s3"))
        || (name == "dynamodb" && cfg!(feature = "dynamodb"))
        || (name == "sqs" && cfg!(feature = "sqs"))
        || (name == "ssm" && cfg!(feature = "ssm"))
        || (name == "sns" && cfg!(feature = "sns"))
        || (name == "lambda" && cfg!(feature = "lambda"))
        || (name == "events" && cfg!(feature = "events"))
        || (name == "logs" && cfg!(feature = "logs"))
        || (name == "kms" && cfg!(feature = "kms"))
        || (name == "kinesis" && cfg!(feature = "kinesis"))
        || (name == "secretsmanager" && cfg!(feature = "secretsmanager"))
        || (name == "ses" && cfg!(feature = "ses"))
        || (name == "apigatewayv2" && cfg!(feature = "apigatewayv2"))
        || (name == "cloudwatch" && cfg!(feature = "cloudwatch"))
        || (name == "dynamodbstreams" && cfg!(feature = "dynamodbstreams"))
        || (name == "iam" && cfg!(feature = "iam"))
        || (name == "sts" && cfg!(feature = "sts"))
}

/// Parse the `SERVICES` environment variable into a list of service names.
///
/// If `SERVICES` is unset or empty, returns all compiled-in services.
fn parse_enabled_services() -> Vec<String> {
    let raw = std::env::var("SERVICES").unwrap_or_default();
    parse_services_value(&raw)
}

/// Parse a comma-separated services string into a list of service names.
///
/// If the input is empty, returns all compiled-in services.
fn parse_services_value(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        // All compiled-in services.
        let mut all = Vec::new();
        if cfg!(feature = "s3") {
            all.push("s3".to_string());
        }
        if cfg!(feature = "dynamodb") {
            all.push("dynamodb".to_string());
        }
        if cfg!(feature = "sqs") {
            all.push("sqs".to_string());
        }
        if cfg!(feature = "ssm") {
            all.push("ssm".to_string());
        }
        if cfg!(feature = "sns") {
            all.push("sns".to_string());
        }
        if cfg!(feature = "lambda") {
            all.push("lambda".to_string());
        }
        if cfg!(feature = "events") {
            all.push("events".to_string());
        }
        if cfg!(feature = "logs") {
            all.push("logs".to_string());
        }
        if cfg!(feature = "kms") {
            all.push("kms".to_string());
        }
        if cfg!(feature = "kinesis") {
            all.push("kinesis".to_string());
        }
        if cfg!(feature = "secretsmanager") {
            all.push("secretsmanager".to_string());
        }
        if cfg!(feature = "ses") {
            all.push("ses".to_string());
        }
        if cfg!(feature = "apigatewayv2") {
            all.push("apigatewayv2".to_string());
        }
        if cfg!(feature = "cloudwatch") {
            all.push("cloudwatch".to_string());
        }
        if cfg!(feature = "dynamodbstreams") {
            all.push("dynamodbstreams".to_string());
        }
        if cfg!(feature = "iam") {
            all.push("iam".to_string());
        }
        if cfg!(feature = "sts") {
            all.push("sts".to_string());
        }
        all
    } else {
        trimmed
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Perform a health check by connecting to the gateway and requesting the health endpoint.
///
/// Exits with code 0 if the response is 200 OK and contains at least one
/// running service, 1 otherwise.
async fn run_health_check(addr: &str) -> Result<()> {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    let mut stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("cannot connect to {addr}"))?;

    let request =
        format!("GET /_localstack/health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    // Do not half-close the write side: the HTTP request is self-framing
    // (GET with no body) and the Connection: close header tells hyper not
    // to expect further requests.  Hyper will close after responding,
    // which gives read_to_string its EOF.

    let mut response = String::new();
    stream.read_to_string(&mut response).await?;

    // Accept any 200 response that reports at least one running service.
    if response.contains("200 OK") && response.contains("\"running\"") {
        Ok(())
    } else {
        anyhow::bail!("unhealthy response from {addr}")
    }
}

/// Read the gateway listen address from the environment.
///
/// Checks `GATEWAY_LISTEN` (the canonical var) and falls back to the
/// S3Config default when S3 is compiled in.
fn gateway_listen_addr() -> String {
    std::env::var("GATEWAY_LISTEN").unwrap_or_else(|_| "0.0.0.0:4566".to_string())
}

/// Read the log level from the environment.
fn log_level() -> String {
    std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string())
}

/// Build all enabled service routers based on environment configuration.
#[allow(clippy::too_many_lines)]
fn build_services(is_enabled: impl Fn(&str) -> bool) -> Vec<Box<dyn ServiceRouter>> {
    let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

    // ----- DynamoDB + DynamoDB Streams (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "dynamodb")]
    if is_enabled("dynamodb") {
        let dynamodb_config = DynamoDBConfig::from_env();
        info!(
            dynamodb_skip_signature_validation = dynamodb_config.skip_signature_validation,
            "initializing DynamoDB service",
        );
        let mut dynamodb_provider = RustackDynamoDB::new(dynamodb_config.clone());

        // Wire in DynamoDB Streams emitter and lifecycle manager if enabled.
        #[cfg(feature = "dynamodbstreams")]
        let stream_store = if is_enabled("dynamodbstreams") {
            let streams_config = DynamoDBStreamsConfig::from_env();
            let store = Arc::new(StreamStore::new());
            let emitter = Arc::new(DynamoDBStreamEmitter::new(
                Arc::clone(&store),
                dynamodb_config.default_region.clone(),
            ));
            let lifecycle = Arc::new(DynamoDBStreamLifecycleManager::new(
                Arc::clone(&store),
                streams_config.default_region.clone(),
                streams_config.default_account_id.clone(),
            ));
            dynamodb_provider.set_emitter(emitter);
            dynamodb_provider.set_lifecycle(lifecycle);
            Some((store, streams_config))
        } else {
            None
        };

        let dynamodb_handler = RustackDynamoDBHandler::new(Arc::new(dynamodb_provider));
        let dynamodb_http_config = build_dynamodb_http_config(&dynamodb_config);
        let dynamodb_service =
            DynamoDBHttpService::new(Arc::new(dynamodb_handler), dynamodb_http_config);
        services.push(Box::new(service::DynamoDBServiceRouter::new(
            dynamodb_service,
        )));

        // Register DynamoDB Streams router.
        #[cfg(feature = "dynamodbstreams")]
        if let Some((store, streams_config)) = stream_store {
            info!("initializing DynamoDB Streams service");
            let streams_provider = RustackDynamoDBStreams::new(store, streams_config.clone());
            let streams_handler = RustackDynamoDBStreamsHandler::new(Arc::new(streams_provider));
            let streams_http_config = build_dynamodbstreams_http_config(&streams_config);
            let streams_service =
                DynamoDBStreamsHttpService::new(Arc::new(streams_handler), streams_http_config);
            services.push(Box::new(service::DynamoDBStreamsServiceRouter::new(
                streams_service,
            )));
        }
    }

    // ----- SQS (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "sqs")]
    let sqs_provider_arc: Option<Arc<RustackSqs>> = if is_enabled("sqs") {
        let sqs_config = SqsConfig::from_env();
        info!(
            sqs_skip_signature_validation = sqs_config.skip_signature_validation,
            "initializing SQS service",
        );
        let sqs_provider = Arc::new(RustackSqs::new(sqs_config.clone()));
        let sqs_handler = RustackSqsHandler::new(Arc::clone(&sqs_provider));
        let sqs_http_config = build_sqs_http_config(&sqs_config);
        let sqs_service = SqsHttpService::new(Arc::new(sqs_handler), sqs_http_config);
        services.push(Box::new(service::SqsServiceRouter::new(sqs_service)));
        Some(sqs_provider)
    } else {
        None
    };

    // ----- SSM (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "ssm")]
    if is_enabled("ssm") {
        let ssm_config = SsmConfig::from_env();
        info!(
            ssm_skip_signature_validation = ssm_config.skip_signature_validation,
            "initializing SSM service",
        );
        let ssm_provider = RustackSsm::new(ssm_config.clone());
        let ssm_handler = RustackSsmHandler::new(Arc::new(ssm_provider));
        let ssm_http_config = build_ssm_http_config(&ssm_config);
        let ssm_service = SsmHttpService::new(Arc::new(ssm_handler), ssm_http_config);
        services.push(Box::new(service::SsmServiceRouter::new(ssm_service)));
    }

    // ----- IAM (register BEFORE CloudWatch/SES/SNS: all use form-urlencoded POST) -----
    // IAM matches on SigV4 service=iam.
    #[cfg(feature = "iam")]
    if is_enabled("iam") {
        let iam_config = IamConfig::from_env();
        info!(
            iam_skip_signature_validation = iam_config.skip_signature_validation,
            "initializing IAM service",
        );
        let iam_store = Arc::new(IamStore::new());
        let iam_provider = RustackIam::new(iam_store, Arc::new(iam_config.clone()));
        let iam_handler = RustackIamHandler::new(Arc::new(iam_provider));
        let iam_http_config = build_iam_http_config(&iam_config);
        let iam_service = IamHttpService::new(Arc::new(iam_handler), iam_http_config);
        services.push(Box::new(service::IamServiceRouter::new(iam_service)));
    }

    // ----- CloudWatch Metrics (register BEFORE SES/SNS: all use form-urlencoded POST) -----
    // CloudWatch matches on SigV4 service=monitoring.
    #[cfg(feature = "cloudwatch")]
    if is_enabled("cloudwatch") {
        let cw_config = CloudWatchConfig::from_env();
        info!(
            cloudwatch_skip_signature_validation = cw_config.skip_signature_validation,
            "initializing CloudWatch Metrics service",
        );
        let cw_provider = RustackCloudWatch::new(Arc::new(cw_config.clone()));
        let cw_handler = RustackCloudWatchHandler::new(Arc::new(cw_provider));
        let cw_http_config = build_cloudwatch_http_config(&cw_config);
        let cw_service = CloudWatchHttpService::new(Arc::new(cw_handler), cw_http_config);
        services.push(Box::new(service::CloudWatchServiceRouter::new(cw_service)));
    }

    // ----- SES (register BEFORE SNS: both use form-urlencoded POST) -----
    #[cfg(feature = "ses")]
    if is_enabled("ses") {
        let ses_config = SesConfig::from_env();
        info!(
            ses_skip_signature_validation = ses_config.skip_signature_validation,
            ses_require_verified_identity = ses_config.require_verified_identity,
            "initializing SES service",
        );
        let ses_provider = RustackSes::new(ses_config.clone());
        let ses_handler = Arc::new(RustackSesHandler::new(Arc::new(ses_provider)));
        let ses_http_config = build_ses_http_config(&ses_config);
        let ses_v1_service = SesHttpService::new(Arc::clone(&ses_handler), ses_http_config);
        let ses_v2_service = SesV2HttpService::new(Arc::clone(&ses_handler));
        services.push(Box::new(service::SesServiceRouter::new(
            ses_v1_service,
            ses_v2_service,
        )));
    }

    // ----- STS (register BEFORE SNS: both use form-urlencoded POST) -----
    // STS matches on SigV4 service=sts.
    #[cfg(feature = "sts")]
    if is_enabled("sts") {
        let sts_config = StsConfig::from_env();
        info!(
            sts_skip_signature_validation = sts_config.skip_signature_validation,
            "initializing STS service",
        );
        let sts_provider = RustackSts::new(sts_config.clone());
        let sts_handler = RustackStsHandler::new(Arc::new(sts_provider));
        let sts_http_config = build_sts_http_config(&sts_config);
        let sts_service = StsHttpService::new(Arc::new(sts_handler), sts_http_config);
        services.push(Box::new(service::StsServiceRouter::new(sts_service)));
    }

    // ----- SNS (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "sns")]
    if is_enabled("sns") {
        let sns_config = SnsConfig::from_env();
        info!(
            sns_skip_signature_validation = sns_config.skip_signature_validation,
            "initializing SNS service",
        );
        let sqs_publisher: Arc<dyn rustack_sns_core::publisher::SqsPublisher> =
            if let Some(ref sqs) = sqs_provider_arc {
                Arc::new(RustackSqsPublisher::new(
                    Arc::clone(sqs),
                    sns_config.clone(),
                ))
            } else {
                Arc::new(rustack_sns_core::publisher::NoopSqsPublisher)
            };
        let sns_provider = RustackSns::new(sns_config.clone(), sqs_publisher);
        let sns_handler = RustackSnsHandler::new(Arc::new(sns_provider));
        let sns_http_config = build_sns_http_config(&sns_config);
        let sns_service = SnsHttpService::new(Arc::new(sns_handler), sns_http_config);
        services.push(Box::new(service::SnsServiceRouter::new(sns_service)));
    }

    // ----- EventBridge (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "events")]
    if is_enabled("events") {
        let events_config = EventsConfig::from_env();
        info!(
            events_skip_signature_validation = events_config.skip_signature_validation,
            "initializing EventBridge service",
        );
        let delivery: Arc<dyn rustack_events_core::delivery::TargetDelivery> =
            if let Some(ref sqs) = sqs_provider_arc {
                Arc::new(LocalTargetDelivery::new(
                    Arc::clone(sqs),
                    events_config.account_id.clone(),
                    events_config.host.clone(),
                    events_config.port,
                ))
            } else {
                Arc::new(rustack_events_core::delivery::NoopTargetDelivery)
            };
        let events_provider = RustackEvents::new(events_config.clone(), delivery);
        let events_handler = RustackEventsHandler::new(Arc::new(events_provider));
        let events_http_config = build_events_http_config(&events_config);
        let events_service = EventsHttpService::new(Arc::new(events_handler), events_http_config);
        services.push(Box::new(service::EventsServiceRouter::new(events_service)));
    }

    // ----- CloudWatch Logs (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "logs")]
    if is_enabled("logs") {
        let logs_config = LogsConfig::from_env();
        info!(
            logs_skip_signature_validation = logs_config.skip_signature_validation,
            "initializing CloudWatch Logs service",
        );
        let logs_provider = RustackLogs::new(logs_config.clone());
        let logs_handler = RustackLogsHandler::new(Arc::new(logs_provider));
        let logs_http_config = build_logs_http_config(&logs_config);
        let logs_service = LogsHttpService::new(Arc::new(logs_handler), logs_http_config);
        services.push(Box::new(service::LogsServiceRouter::new(logs_service)));
    }

    // ----- KMS (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "kms")]
    if is_enabled("kms") {
        let kms_config = KmsConfig::from_env();
        info!(
            kms_skip_signature_validation = kms_config.skip_signature_validation,
            "initializing KMS service",
        );
        let kms_provider = RustackKms::new(kms_config.clone());
        let kms_handler = RustackKmsHandler::new(Arc::new(kms_provider));
        let kms_http_config = build_kms_http_config(&kms_config);
        let kms_service = KmsHttpService::new(Arc::new(kms_handler), kms_http_config);
        services.push(Box::new(service::KmsServiceRouter::new(kms_service)));
    }

    // ----- Kinesis (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "kinesis")]
    if is_enabled("kinesis") {
        let kinesis_config = KinesisConfig::from_env();
        info!(
            kinesis_skip_signature_validation = kinesis_config.skip_signature_validation,
            "initializing Kinesis service",
        );
        let kinesis_provider = RustackKinesis::new(kinesis_config.clone());
        let kinesis_handler = RustackKinesisHandler::new(Arc::new(kinesis_provider));
        let kinesis_http_config = build_kinesis_http_config(&kinesis_config);
        let kinesis_service =
            KinesisHttpService::new(Arc::new(kinesis_handler), kinesis_http_config);
        services.push(Box::new(service::KinesisServiceRouter::new(
            kinesis_service,
        )));
    }

    // ----- Secrets Manager (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "secretsmanager")]
    if is_enabled("secretsmanager") {
        let sm_config = SecretsManagerConfig::from_env();
        info!(
            secretsmanager_skip_signature_validation = sm_config.skip_signature_validation,
            "initializing Secrets Manager service",
        );
        let sm_provider = RustackSecretsManager::new(sm_config.clone());
        let sm_handler = RustackSecretsManagerHandler::new(Arc::new(sm_provider));
        let sm_http_config = build_secretsmanager_http_config(&sm_config);
        let sm_service = SecretsManagerHttpService::new(Arc::new(sm_handler), sm_http_config);
        services.push(Box::new(service::SecretsManagerServiceRouter::new(
            sm_service,
        )));
    }

    // ----- API Gateway v2 (register before Lambda and S3: S3 is the catch-all) -----
    #[cfg(feature = "apigatewayv2")]
    if is_enabled("apigatewayv2") {
        let apigw_config = ApiGatewayV2Config::from_env();
        info!(
            apigatewayv2_skip_signature_validation = apigw_config.skip_signature_validation,
            "initializing API Gateway v2 service",
        );
        let apigw_provider = Arc::new(RustackApiGatewayV2::new(apigw_config.clone()));
        let apigw_handler = RustackApiGatewayV2Handler::new(Arc::clone(&apigw_provider));
        let apigw_http_config = build_apigatewayv2_http_config(&apigw_config);
        let apigw_service =
            ApiGatewayV2HttpService::new(Arc::new(apigw_handler), apigw_http_config);
        services.push(Box::new(service::ApiGatewayV2ManagementRouter::new(
            apigw_service,
        )));
        services.push(Box::new(service::ApiGatewayV2ExecutionRouter::new(
            Arc::clone(&apigw_provider),
        )));
    }

    // ----- Lambda (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "lambda")]
    if is_enabled("lambda") {
        let lambda_config = LambdaConfig::from_env();
        info!(
            lambda_skip_signature_validation = lambda_config.skip_signature_validation,
            lambda_docker_enabled = lambda_config.docker_enabled,
            "initializing Lambda service",
        );
        let lambda_provider = RustackLambda::new(lambda_config.clone());
        let lambda_handler = RustackLambdaHandler::new(Arc::new(lambda_provider));
        let lambda_http_config = build_lambda_http_config(&lambda_config);
        let lambda_service = LambdaHttpService::new(Arc::new(lambda_handler), lambda_http_config);
        services.push(Box::new(service::LambdaServiceRouter::new(lambda_service)));
    }

    // ----- S3 (catch-all, must be last) -----
    #[cfg(feature = "s3")]
    if is_enabled("s3") {
        let s3_config = S3Config::from_env();
        info!(
            s3_domain = %s3_config.s3_domain,
            s3_virtual_hosting = s3_config.s3_virtual_hosting,
            s3_skip_signature_validation = s3_config.s3_skip_signature_validation,
            "initializing S3 service",
        );
        let s3_provider = RustackS3::new(s3_config.clone());
        let s3_handler = handler::RustackHandler(s3_provider);
        let s3_http_config = build_s3_http_config(&s3_config);
        let s3_service = S3HttpService::new(s3_handler, s3_http_config);
        services.push(Box::new(service::S3ServiceRouter::new(s3_service)));
    }

    services
}

#[tokio::main]
async fn main() -> Result<()> {
    let listen_addr = gateway_listen_addr();

    // Handle --health-check flag for Docker HEALTHCHECK.
    if std::env::args().any(|a| a == "--health-check") {
        let addr = listen_addr.replace("0.0.0.0", "127.0.0.1");
        let healthy = run_health_check(&addr).await.is_ok();
        std::process::exit(i32::from(!healthy));
    }

    let log = log_level();
    init_tracing(&log)?;

    let enabled = parse_enabled_services();

    // Warn about services that are requested but not compiled in.
    for name in &enabled {
        if !is_compiled_in(name) {
            warn!(service = %name, "requested service is not compiled in, skipping");
        }
    }

    let services = build_services(|name| enabled.iter().any(|s| s == name) && is_compiled_in(name));

    if services.is_empty() {
        anyhow::bail!(
            "no services enabled. Check the SERVICES environment variable and compiled feature \
             flags."
        );
    }

    let gateway = GatewayService::new(services);
    let service_names = gateway.service_names();

    let addr: SocketAddr = listen_addr
        .parse()
        .with_context(|| format!("invalid bind address: {listen_addr}"))?;

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind to {addr}"))?;

    info!(
        %addr,
        services = ?service_names,
        version = VERSION,
        "starting Rustack Server",
    );

    serve(listener, gateway).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_services_value_default() {
        // Empty string = all compiled-in services.
        let services = parse_services_value("");
        if cfg!(feature = "s3") {
            assert!(services.contains(&"s3".to_string()));
        }
        if cfg!(feature = "dynamodb") {
            assert!(services.contains(&"dynamodb".to_string()));
        }
    }

    #[test]
    fn test_should_parse_services_value_explicit() {
        let services = parse_services_value("dynamodb");
        assert_eq!(services, vec!["dynamodb"]);
    }

    #[test]
    fn test_should_parse_services_value_multiple() {
        let services = parse_services_value("s3, dynamodb");
        assert_eq!(services, vec!["s3", "dynamodb"]);
    }

    #[test]
    fn test_should_parse_services_value_whitespace() {
        // Whitespace-only = all compiled-in services.
        let services = parse_services_value("  ");
        assert!(!services.is_empty());
    }

    #[test]
    fn test_should_parse_services_value_case_insensitive() {
        let services = parse_services_value("S3,DynamoDB");
        assert_eq!(services, vec!["s3", "dynamodb"]);
    }

    #[test]
    fn test_should_detect_compiled_services() {
        assert_eq!(is_compiled_in("s3"), cfg!(feature = "s3"));
        assert_eq!(is_compiled_in("dynamodb"), cfg!(feature = "dynamodb"));
        assert_eq!(is_compiled_in("sqs"), cfg!(feature = "sqs"));
        assert_eq!(is_compiled_in("ssm"), cfg!(feature = "ssm"));
        assert_eq!(is_compiled_in("sns"), cfg!(feature = "sns"));
        assert_eq!(is_compiled_in("lambda"), cfg!(feature = "lambda"));
        assert_eq!(is_compiled_in("events"), cfg!(feature = "events"));
        assert_eq!(is_compiled_in("logs"), cfg!(feature = "logs"));
        assert_eq!(is_compiled_in("kms"), cfg!(feature = "kms"));
        assert_eq!(is_compiled_in("kinesis"), cfg!(feature = "kinesis"));
        assert_eq!(
            is_compiled_in("secretsmanager"),
            cfg!(feature = "secretsmanager")
        );
        assert_eq!(is_compiled_in("ses"), cfg!(feature = "ses"));
        assert_eq!(
            is_compiled_in("apigatewayv2"),
            cfg!(feature = "apigatewayv2")
        );
    }

    #[cfg(feature = "s3")]
    #[test]
    fn test_should_build_s3_http_config_from_s3_config() {
        let config = S3Config::from_env();
        let http_config = build_s3_http_config(&config);

        assert_eq!(http_config.domain, config.s3_domain);
        assert_eq!(http_config.virtual_hosting, config.s3_virtual_hosting);
        assert_eq!(
            http_config.skip_signature_validation,
            config.s3_skip_signature_validation
        );
        assert_eq!(http_config.region, config.default_region);
    }

    #[cfg(feature = "dynamodb")]
    #[test]
    fn test_should_build_dynamodb_http_config_from_dynamodb_config() {
        let config = DynamoDBConfig::from_env();
        let http_config = build_dynamodb_http_config(&config);

        assert_eq!(
            http_config.skip_signature_validation,
            config.skip_signature_validation
        );
        assert_eq!(http_config.region, config.default_region);
    }
}
