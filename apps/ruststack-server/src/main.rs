//! RustStack Server - High-performance AWS-compatible server.
//!
//! This binary provides a LocalStack-compatible server that can host one or
//! more AWS services. A gateway layer routes requests to the appropriate
//! service based on request headers.
//!
//! # Usage
//!
//! ```text
//! GATEWAY_LISTEN=0.0.0.0:4566 ruststack-server
//! ```
//!
//! # Environment Variables
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address |
//! | `SERVICES` | *(empty = all)* | Comma-separated list of services to enable |
//! | `S3_SKIP_SIGNATURE_VALIDATION` | `true` | Skip S3 SigV4 verification |
//! | `DYNAMODB_SKIP_SIGNATURE_VALIDATION` | `true` | Skip DynamoDB SigV4 verification |
//! | `SQS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SQS SigV4 verification |
//! | `SSM_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SSM SigV4 verification |
//! | `SNS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SNS SigV4 verification |
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

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HttpConnBuilder;
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[cfg(feature = "dynamodb")]
use ruststack_dynamodb_core::config::DynamoDBConfig;
#[cfg(feature = "dynamodb")]
use ruststack_dynamodb_core::handler::RustStackDynamoDBHandler;
#[cfg(feature = "dynamodb")]
use ruststack_dynamodb_core::provider::RustStackDynamoDB;
#[cfg(feature = "dynamodb")]
use ruststack_dynamodb_http::service::{DynamoDBHttpConfig, DynamoDBHttpService};

#[cfg(feature = "sqs")]
use ruststack_sqs_core::config::SqsConfig;
#[cfg(feature = "sqs")]
use ruststack_sqs_core::handler::RustStackSqsHandler;
#[cfg(feature = "sqs")]
use ruststack_sqs_core::provider::RustStackSqs;
#[cfg(feature = "sqs")]
use ruststack_sqs_http::service::{SqsHttpConfig, SqsHttpService};

#[cfg(feature = "ssm")]
use ruststack_ssm_core::config::SsmConfig;
#[cfg(feature = "ssm")]
use ruststack_ssm_core::handler::RustStackSsmHandler;
#[cfg(feature = "ssm")]
use ruststack_ssm_core::provider::RustStackSsm;
#[cfg(feature = "ssm")]
use ruststack_ssm_http::service::{SsmHttpConfig, SsmHttpService};

#[cfg(feature = "sns")]
use crate::sns_bridge::RustStackSqsPublisher;
#[cfg(feature = "sns")]
use ruststack_sns_core::config::SnsConfig;
#[cfg(feature = "sns")]
use ruststack_sns_core::handler::RustStackSnsHandler;
#[cfg(feature = "sns")]
use ruststack_sns_core::provider::RustStackSns;
#[cfg(feature = "sns")]
use ruststack_sns_http::service::{SnsHttpConfig, SnsHttpService};

#[cfg(feature = "lambda")]
use ruststack_lambda_core::config::LambdaConfig;
#[cfg(feature = "lambda")]
use ruststack_lambda_core::handler::RustStackLambdaHandler;
#[cfg(feature = "lambda")]
use ruststack_lambda_core::provider::RustStackLambda;
#[cfg(feature = "lambda")]
use ruststack_lambda_http::service::{LambdaHttpConfig, LambdaHttpService};

#[cfg(feature = "events")]
use crate::events_bridge::LocalTargetDelivery;
#[cfg(feature = "events")]
use ruststack_events_core::config::EventsConfig;
#[cfg(feature = "events")]
use ruststack_events_core::handler::RustStackEventsHandler;
#[cfg(feature = "events")]
use ruststack_events_core::provider::RustStackEvents;
#[cfg(feature = "events")]
use ruststack_events_http::service::{EventsHttpConfig, EventsHttpService};

#[cfg(feature = "logs")]
use ruststack_logs_core::config::LogsConfig;
#[cfg(feature = "logs")]
use ruststack_logs_core::handler::RustStackLogsHandler;
#[cfg(feature = "logs")]
use ruststack_logs_core::provider::RustStackLogs;
#[cfg(feature = "logs")]
use ruststack_logs_http::service::{LogsHttpConfig, LogsHttpService};

#[cfg(feature = "s3")]
use ruststack_s3_core::{RustStackS3, S3Config};
#[cfg(feature = "s3")]
use ruststack_s3_http::service::{S3HttpConfig, S3HttpService};

use crate::gateway::GatewayService;
use crate::service::ServiceRouter;

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
    feature = "logs"
))]
fn build_credential_provider() -> Option<Arc<dyn ruststack_auth::CredentialProvider>> {
    use ruststack_auth::StaticCredentialProvider;

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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

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

    // ----- DynamoDB (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "dynamodb")]
    if is_enabled("dynamodb") {
        let dynamodb_config = DynamoDBConfig::from_env();
        info!(
            dynamodb_skip_signature_validation = dynamodb_config.skip_signature_validation,
            "initializing DynamoDB service",
        );
        let dynamodb_provider = RustStackDynamoDB::new(dynamodb_config.clone());
        let dynamodb_handler = RustStackDynamoDBHandler::new(Arc::new(dynamodb_provider));
        let dynamodb_http_config = build_dynamodb_http_config(&dynamodb_config);
        let dynamodb_service =
            DynamoDBHttpService::new(Arc::new(dynamodb_handler), dynamodb_http_config);
        services.push(Box::new(service::DynamoDBServiceRouter::new(
            dynamodb_service,
        )));
    }

    // ----- SQS (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "sqs")]
    let sqs_provider_arc: Option<Arc<RustStackSqs>> = if is_enabled("sqs") {
        let sqs_config = SqsConfig::from_env();
        info!(
            sqs_skip_signature_validation = sqs_config.skip_signature_validation,
            "initializing SQS service",
        );
        let sqs_provider = Arc::new(RustStackSqs::new(sqs_config.clone()));
        let sqs_handler = RustStackSqsHandler::new(Arc::clone(&sqs_provider));
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
        let ssm_provider = RustStackSsm::new(ssm_config.clone());
        let ssm_handler = RustStackSsmHandler::new(Arc::new(ssm_provider));
        let ssm_http_config = build_ssm_http_config(&ssm_config);
        let ssm_service = SsmHttpService::new(Arc::new(ssm_handler), ssm_http_config);
        services.push(Box::new(service::SsmServiceRouter::new(ssm_service)));
    }

    // ----- SNS (register before S3: S3 is the catch-all) -----
    #[cfg(feature = "sns")]
    if is_enabled("sns") {
        let sns_config = SnsConfig::from_env();
        info!(
            sns_skip_signature_validation = sns_config.skip_signature_validation,
            "initializing SNS service",
        );
        let sqs_publisher: Arc<dyn ruststack_sns_core::publisher::SqsPublisher> =
            if let Some(ref sqs) = sqs_provider_arc {
                Arc::new(RustStackSqsPublisher::new(
                    Arc::clone(sqs),
                    sns_config.clone(),
                ))
            } else {
                Arc::new(ruststack_sns_core::publisher::NoopSqsPublisher)
            };
        let sns_provider = RustStackSns::new(sns_config.clone(), sqs_publisher);
        let sns_handler = RustStackSnsHandler::new(Arc::new(sns_provider));
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
        let delivery: Arc<dyn ruststack_events_core::delivery::TargetDelivery> =
            if let Some(ref sqs) = sqs_provider_arc {
                Arc::new(LocalTargetDelivery::new(
                    Arc::clone(sqs),
                    events_config.account_id.clone(),
                    events_config.host.clone(),
                    events_config.port,
                ))
            } else {
                Arc::new(ruststack_events_core::delivery::NoopTargetDelivery)
            };
        let events_provider = RustStackEvents::new(events_config.clone(), delivery);
        let events_handler = RustStackEventsHandler::new(Arc::new(events_provider));
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
        let logs_provider = RustStackLogs::new(logs_config.clone());
        let logs_handler = RustStackLogsHandler::new(Arc::new(logs_provider));
        let logs_http_config = build_logs_http_config(&logs_config);
        let logs_service = LogsHttpService::new(Arc::new(logs_handler), logs_http_config);
        services.push(Box::new(service::LogsServiceRouter::new(logs_service)));
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
        let lambda_provider = RustStackLambda::new(lambda_config.clone());
        let lambda_handler = RustStackLambdaHandler::new(Arc::new(lambda_provider));
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
        let s3_provider = RustStackS3::new(s3_config.clone());
        let s3_handler = handler::RustStackHandler(s3_provider);
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
            "no services enabled. Check the SERVICES environment variable \
             and compiled feature flags."
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
        "starting RustStack Server",
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
