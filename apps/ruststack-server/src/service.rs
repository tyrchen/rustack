//! Service router abstraction for the gateway.
//!
//! Each AWS service (S3, DynamoDB, etc.) implements [`ServiceRouter`] to
//! declare how it matches incoming requests and handles them. The gateway
//! holds a list of routers and dispatches to the first match.
//!
//! [`GatewayBody`] is a type-erased HTTP response body shared by all services.

use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::pin::Pin;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;

/// Type-erased response body used by the gateway.
///
/// All service routers convert their native response body into this type
/// so the gateway can handle responses uniformly.
pub type GatewayBody = BoxBody<Bytes, io::Error>;

/// Create a [`GatewayBody`] from a string.
pub fn gateway_body_from_string(s: impl Into<String>) -> GatewayBody {
    Full::new(Bytes::from(s.into()))
        .map_err(|never: Infallible| match never {})
        .boxed()
}

/// A routable AWS service registered with the gateway.
///
/// Implementors declare which requests they handle (via [`matches`](ServiceRouter::matches))
/// and how to process them (via [`call`](ServiceRouter::call)).
pub trait ServiceRouter: Send + Sync {
    /// Service name for health reporting (e.g., `"s3"`, `"dynamodb"`).
    fn name(&self) -> &'static str;

    /// Returns `true` if this router should handle the given request.
    fn matches(&self, req: &http::Request<Incoming>) -> bool;

    /// Handle the request, producing a response with a type-erased body.
    fn call(
        &self,
        req: http::Request<Incoming>,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>;
}

// ---------------------------------------------------------------------------
// S3
// ---------------------------------------------------------------------------

#[cfg(feature = "s3")]
mod s3_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_s3_http::dispatch::S3Handler;
    use ruststack_s3_http::service::S3HttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the S3 service.
    ///
    /// S3 acts as the catch-all: it matches every request that no other
    /// service has claimed. Register it **last** in the gateway's service list.
    pub struct S3ServiceRouter<H: S3Handler> {
        inner: S3HttpService<H>,
    }

    impl<H: S3Handler> S3ServiceRouter<H> {
        /// Wrap an [`S3HttpService`] in a router.
        pub fn new(inner: S3HttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: S3Handler> ServiceRouter for S3ServiceRouter<H> {
        fn name(&self) -> &'static str {
            "s3"
        }

        /// S3 is the default service — it matches every request.
        fn matches(&self, _req: &http::Request<Incoming>) -> bool {
            true
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "s3")]
pub use s3_router::S3ServiceRouter;

// ---------------------------------------------------------------------------
// DynamoDB
// ---------------------------------------------------------------------------

#[cfg(feature = "dynamodb")]
mod dynamodb_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_dynamodb_http::dispatch::DynamoDBHandler;
    use ruststack_dynamodb_http::service::DynamoDBHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the DynamoDB service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `DynamoDB_`.
    pub struct DynamoDBServiceRouter<H: DynamoDBHandler> {
        inner: DynamoDBHttpService<H>,
    }

    impl<H: DynamoDBHandler> DynamoDBServiceRouter<H> {
        /// Wrap a [`DynamoDBHttpService`] in a router.
        pub fn new(inner: DynamoDBHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: DynamoDBHandler> ServiceRouter for DynamoDBServiceRouter<H> {
        fn name(&self) -> &'static str {
            "dynamodb"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("DynamoDB_"))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "dynamodb")]
pub use dynamodb_router::DynamoDBServiceRouter;

// ---------------------------------------------------------------------------
// SQS
// ---------------------------------------------------------------------------

#[cfg(feature = "sqs")]
mod sqs_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_sqs_http::dispatch::SqsHandler;
    use ruststack_sqs_http::service::SqsHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the SQS service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `AmazonSQS`.
    pub struct SqsServiceRouter<H: SqsHandler> {
        inner: SqsHttpService<H>,
    }

    impl<H: SqsHandler> SqsServiceRouter<H> {
        /// Wrap an [`SqsHttpService`] in a router.
        pub fn new(inner: SqsHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: SqsHandler> ServiceRouter for SqsServiceRouter<H> {
        fn name(&self) -> &'static str {
            "sqs"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("AmazonSQS"))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "sqs")]
pub use sqs_router::SqsServiceRouter;

// ---------------------------------------------------------------------------
// SSM
// ---------------------------------------------------------------------------

#[cfg(feature = "ssm")]
mod ssm_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_ssm_http::dispatch::SsmHandler;
    use ruststack_ssm_http::service::SsmHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the SSM service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `AmazonSSM.`.
    pub struct SsmServiceRouter<H: SsmHandler> {
        inner: SsmHttpService<H>,
    }

    impl<H: SsmHandler> SsmServiceRouter<H> {
        /// Wrap an [`SsmHttpService`] in a router.
        pub fn new(inner: SsmHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: SsmHandler> ServiceRouter for SsmServiceRouter<H> {
        fn name(&self) -> &'static str {
            "ssm"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("AmazonSSM."))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "ssm")]
pub use ssm_router::SsmServiceRouter;

// ---------------------------------------------------------------------------
// SNS
// ---------------------------------------------------------------------------

#[cfg(feature = "sns")]
mod sns_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_sns_http::dispatch::SnsHandler;
    use ruststack_sns_http::service::SnsHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the SNS service.
    ///
    /// Matches `POST /` requests with `Content-Type: application/x-www-form-urlencoded`.
    /// SNS uses the `awsQuery` protocol where the operation is determined by
    /// the `Action=` parameter in the form body.
    pub struct SnsServiceRouter<H: SnsHandler> {
        inner: SnsHttpService<H>,
    }

    impl<H: SnsHandler> SnsServiceRouter<H> {
        /// Wrap an [`SnsHttpService`] in a router.
        pub fn new(inner: SnsHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: SnsHandler> ServiceRouter for SnsServiceRouter<H> {
        fn name(&self) -> &'static str {
            "sns"
        }

        /// SNS matches form-urlencoded POST requests to `/`.
        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            if *req.method() != http::Method::POST {
                return false;
            }
            req.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.contains("x-www-form-urlencoded"))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "sns")]
pub use sns_router::SnsServiceRouter;

// ---------------------------------------------------------------------------
// Lambda
// ---------------------------------------------------------------------------

#[cfg(feature = "lambda")]
mod lambda_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_lambda_http::dispatch::LambdaHandler;
    use ruststack_lambda_http::service::LambdaHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Lambda API uses multiple date-versioned path prefixes.
    /// The SDK may use different dates than the spec documents.
    fn is_lambda_path(path: &str) -> bool {
        // Function CRUD and invoke paths.
        if path.contains("/functions") {
            // Match any date prefix for /YYYY-MM-DD/functions
            if let Some(rest) = path.strip_prefix('/') {
                let parts: Vec<&str> = rest.splitn(2, '/').collect();
                if parts.len() == 2 && parts[0].len() == 10 && parts[1].starts_with("functions") {
                    return true;
                }
            }
        }
        // Tags paths (e.g., /2017-03-31/tags/{arn}).
        if path.contains("/tags/") {
            if let Some(rest) = path.strip_prefix('/') {
                let parts: Vec<&str> = rest.splitn(2, '/').collect();
                if parts.len() == 2 && parts[0].len() == 10 && parts[1].starts_with("tags/") {
                    return true;
                }
            }
        }
        // Account settings paths.
        if path.contains("/account-settings") {
            if let Some(rest) = path.strip_prefix('/') {
                let parts: Vec<&str> = rest.splitn(2, '/').collect();
                if parts.len() == 2 && parts[0].len() == 10 && parts[1] == "account-settings" {
                    return true;
                }
            }
        }
        // Function URL invocation paths.
        path.starts_with("/lambda-url/")
    }

    /// Routes requests to the Lambda service.
    ///
    /// Matches requests whose URL path starts with a date-versioned Lambda API
    /// prefix (e.g., `/2015-03-31/functions`, `/2017-03-31/tags`).
    pub struct LambdaServiceRouter<H: LambdaHandler> {
        inner: LambdaHttpService<H>,
    }

    impl<H: LambdaHandler> LambdaServiceRouter<H> {
        /// Wrap a [`LambdaHttpService`] in a router.
        pub fn new(inner: LambdaHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: LambdaHandler> ServiceRouter for LambdaServiceRouter<H> {
        fn name(&self) -> &'static str {
            "lambda"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            let path = req.uri().path();
            is_lambda_path(path)
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "lambda")]
pub use lambda_router::LambdaServiceRouter;

// ---------------------------------------------------------------------------
// EventBridge
// ---------------------------------------------------------------------------

#[cfg(feature = "events")]
mod events_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_events_http::dispatch::EventsHandler;
    use ruststack_events_http::service::EventsHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the EventBridge service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `AWSEvents.`.
    pub struct EventsServiceRouter<H: EventsHandler> {
        inner: EventsHttpService<H>,
    }

    impl<H: EventsHandler> EventsServiceRouter<H> {
        /// Wrap an [`EventsHttpService`] in a router.
        pub fn new(inner: EventsHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: EventsHandler> ServiceRouter for EventsServiceRouter<H> {
        fn name(&self) -> &'static str {
            "events"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("AWSEvents."))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "events")]
pub use events_router::EventsServiceRouter;

// ---------------------------------------------------------------------------
// CloudWatch Logs
// ---------------------------------------------------------------------------

#[cfg(feature = "logs")]
mod logs_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_logs_http::dispatch::LogsHandler;
    use ruststack_logs_http::service::LogsHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the CloudWatch Logs service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `Logs_20140328.`.
    pub struct LogsServiceRouter<H: LogsHandler> {
        inner: LogsHttpService<H>,
    }

    impl<H: LogsHandler> LogsServiceRouter<H> {
        /// Wrap a [`LogsHttpService`] in a router.
        pub fn new(inner: LogsHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: LogsHandler> ServiceRouter for LogsServiceRouter<H> {
        fn name(&self) -> &'static str {
            "logs"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("Logs_20140328."))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "logs")]
pub use logs_router::LogsServiceRouter;

// ---------------------------------------------------------------------------
// KMS
// ---------------------------------------------------------------------------

#[cfg(feature = "kms")]
mod kms_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_kms_http::dispatch::KmsHandler;
    use ruststack_kms_http::service::KmsHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the KMS service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `TrentService.`.
    pub struct KmsServiceRouter<H: KmsHandler> {
        inner: KmsHttpService<H>,
    }

    impl<H: KmsHandler> KmsServiceRouter<H> {
        /// Wrap a [`KmsHttpService`] in a router.
        pub fn new(inner: KmsHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: KmsHandler> ServiceRouter for KmsServiceRouter<H> {
        fn name(&self) -> &'static str {
            "kms"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("TrentService."))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "kms")]
pub use kms_router::KmsServiceRouter;

// ---------------------------------------------------------------------------
// Kinesis
// ---------------------------------------------------------------------------

#[cfg(feature = "kinesis")]
mod kinesis_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_kinesis_http::dispatch::KinesisHandler;
    use ruststack_kinesis_http::service::KinesisHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the Kinesis service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `Kinesis_20131202.`.
    pub struct KinesisServiceRouter<H: KinesisHandler> {
        inner: KinesisHttpService<H>,
    }

    impl<H: KinesisHandler> KinesisServiceRouter<H> {
        /// Wrap a [`KinesisHttpService`] in a router.
        pub fn new(inner: KinesisHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: KinesisHandler> ServiceRouter for KinesisServiceRouter<H> {
        fn name(&self) -> &'static str {
            "kinesis"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("Kinesis_20131202."))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "kinesis")]
pub use kinesis_router::KinesisServiceRouter;

// ---------------------------------------------------------------------------
// Secrets Manager
// ---------------------------------------------------------------------------

#[cfg(feature = "secretsmanager")]
mod secretsmanager_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_secretsmanager_http::dispatch::SecretsManagerHandler;
    use ruststack_secretsmanager_http::service::SecretsManagerHttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Routes requests to the Secrets Manager service.
    ///
    /// Matches requests whose `X-Amz-Target` header starts with `secretsmanager.`.
    pub struct SecretsManagerServiceRouter<H: SecretsManagerHandler> {
        inner: SecretsManagerHttpService<H>,
    }

    impl<H: SecretsManagerHandler> SecretsManagerServiceRouter<H> {
        /// Wrap a [`SecretsManagerHttpService`] in a router.
        pub fn new(inner: SecretsManagerHttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: SecretsManagerHandler> ServiceRouter for SecretsManagerServiceRouter<H> {
        fn name(&self) -> &'static str {
            "secretsmanager"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|t| t.starts_with("secretsmanager."))
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }
}

#[cfg(feature = "secretsmanager")]
pub use secretsmanager_router::SecretsManagerServiceRouter;

// ---------------------------------------------------------------------------
// SES
// ---------------------------------------------------------------------------

#[cfg(feature = "ses")]
mod ses_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;

    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_ses_http::dispatch::SesHandler;
    use ruststack_ses_http::service::SesHttpService;
    use ruststack_ses_http::v2::SesV2HttpService;

    use super::{GatewayBody, ServiceRouter};

    /// Extract the SigV4 service name from the Authorization header.
    ///
    /// Parses `Credential=AKID/date/region/SERVICE/aws4_request` and returns SERVICE.
    fn extract_sigv4_service(headers: &http::HeaderMap) -> Option<&str> {
        let auth = headers.get("authorization")?.to_str().ok()?;
        let credential_start = auth.find("Credential=")? + "Credential=".len();
        let credential_end = auth[credential_start..]
            .find(',')
            .map_or(auth.len(), |i| credential_start + i);
        let credential = &auth[credential_start..credential_end];
        // Format: AKID/date/region/service/aws4_request
        let parts: Vec<&str> = credential.split('/').collect();
        if parts.len() >= 4 {
            Some(parts[3])
        } else {
            None
        }
    }

    /// Routes requests to the SES service.
    ///
    /// Matches SES v1 (`awsQuery` via SigV4 service=`email`) and SES v2
    /// (`restJson1` via `/v2/email/` path prefix). Also handles the
    /// `/_aws/ses` retrospection endpoint.
    pub struct SesServiceRouter<H: SesHandler> {
        inner_v1: SesHttpService<H>,
        inner_v2: SesV2HttpService<H>,
    }

    impl<H: SesHandler> SesServiceRouter<H> {
        /// Create a new SES service router.
        pub fn new(v1: SesHttpService<H>, v2: SesV2HttpService<H>) -> Self {
            Self {
                inner_v1: v1,
                inner_v2: v2,
            }
        }
    }

    impl<H: SesHandler> ServiceRouter for SesServiceRouter<H> {
        fn name(&self) -> &'static str {
            "ses"
        }

        /// SES matches in three ways:
        /// 1. SES v2: path starts with `/v2/email/`
        /// 2. SES retrospection: path starts with `/_aws/ses`
        /// 3. SES v1: form-urlencoded POST with SigV4 service=`email`
        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            let path = req.uri().path();

            // SES v2: path-based routing.
            if path.starts_with("/v2/email/") || path == "/v2/email" {
                return true;
            }

            // SES retrospection endpoint.
            if path == "/_aws/ses" {
                return true;
            }

            // SES v1: form-urlencoded POST with SigV4 service=email.
            if *req.method() != http::Method::POST {
                return false;
            }

            let is_form = req
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.contains("x-www-form-urlencoded"));

            if !is_form {
                return false;
            }

            // Check SigV4 Credential for service=ses (or email).
            // The AWS SDK and CLI sign SES v1 with service name "ses".
            // Some older SDKs may use "email" as the signing name.
            extract_sigv4_service(req.headers()).is_some_and(|svc| svc == "ses" || svc == "email")
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let path = req.uri().path().to_owned();

            if path.starts_with("/v2/email") || path == "/_aws/ses" {
                // Route to SES v2 / retrospection handler.
                let svc = self.inner_v2.clone();
                return Box::pin(async move {
                    let resp = svc.call(req).await;
                    Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
                });
            }

            // Route to SES v1 handler.
            let svc = self.inner_v1.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_should_extract_sigv4_service_ses() {
            let mut headers = http::HeaderMap::new();
            headers.insert(
                "authorization",
                http::HeaderValue::from_static(
                    "AWS4-HMAC-SHA256 Credential=test/20260319/us-east-1/ses/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=abc123",
                ),
            );
            assert_eq!(extract_sigv4_service(&headers), Some("ses"));
        }

        #[test]
        fn test_should_extract_sigv4_service_email() {
            let mut headers = http::HeaderMap::new();
            headers.insert(
                "authorization",
                http::HeaderValue::from_static(
                    "AWS4-HMAC-SHA256 Credential=AKID/20260319/us-east-1/email/aws4_request, SignedHeaders=host, Signature=abc123",
                ),
            );
            assert_eq!(extract_sigv4_service(&headers), Some("email"));
        }

        #[test]
        fn test_should_extract_sigv4_service_sns() {
            let mut headers = http::HeaderMap::new();
            headers.insert(
                "authorization",
                http::HeaderValue::from_static(
                    "AWS4-HMAC-SHA256 Credential=AKID/20260319/us-east-1/sns/aws4_request, SignedHeaders=host, Signature=abc123",
                ),
            );
            assert_eq!(extract_sigv4_service(&headers), Some("sns"));
        }

        #[test]
        fn test_should_return_none_for_missing_auth_header() {
            let headers = http::HeaderMap::new();
            assert_eq!(extract_sigv4_service(&headers), None);
        }

        #[test]
        fn test_should_return_none_for_malformed_credential() {
            let mut headers = http::HeaderMap::new();
            headers.insert(
                "authorization",
                http::HeaderValue::from_static("AWS4-HMAC-SHA256 Credential=AKID, Signature=abc"),
            );
            assert_eq!(extract_sigv4_service(&headers), None);
        }
    }
}

#[cfg(feature = "ses")]
pub use ses_router::SesServiceRouter;

// ---------------------------------------------------------------------------
// API Gateway v2
// ---------------------------------------------------------------------------

#[cfg(feature = "apigatewayv2")]
mod apigatewayv2_router {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use bytes::Bytes;
    use http_body_util::BodyExt;
    use hyper::body::Incoming;
    use hyper::service::Service;
    use ruststack_apigatewayv2_core::execution::{handle_execution, parse_execution_path};
    use ruststack_apigatewayv2_core::provider::RustStackApiGatewayV2;
    use ruststack_apigatewayv2_http::dispatch::ApiGatewayV2Handler;
    use ruststack_apigatewayv2_http::service::ApiGatewayV2HttpService;

    use super::{GatewayBody, ServiceRouter, gateway_body_from_string};

    /// Routes management API requests to the API Gateway v2 service.
    ///
    /// Matches requests whose URL path starts with `/v2/apis`, `/v2/domainnames`,
    /// `/v2/vpclinks`, or `/v2/tags`.
    pub struct ApiGatewayV2ManagementRouter<H: ApiGatewayV2Handler> {
        inner: ApiGatewayV2HttpService<H>,
    }

    impl<H: ApiGatewayV2Handler> ApiGatewayV2ManagementRouter<H> {
        /// Wrap an [`ApiGatewayV2HttpService`] in a management router.
        pub fn new(inner: ApiGatewayV2HttpService<H>) -> Self {
            Self { inner }
        }
    }

    impl<H: ApiGatewayV2Handler> ServiceRouter for ApiGatewayV2ManagementRouter<H> {
        fn name(&self) -> &'static str {
            "apigatewayv2"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            let path = req.uri().path();
            path.starts_with("/v2/apis")
                || path.starts_with("/v2/domainnames")
                || path.starts_with("/v2/vpclinks")
                || path.starts_with("/v2/tags")
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.inner.clone();
            Box::pin(async move {
                let resp = svc.call(req).await;
                Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
            })
        }
    }

    /// Routes execution requests to the API Gateway v2 execution engine.
    ///
    /// Matches requests whose URL path starts with `/_aws/execute-api/`.
    pub struct ApiGatewayV2ExecutionRouter {
        provider: Arc<RustStackApiGatewayV2>,
    }

    impl ApiGatewayV2ExecutionRouter {
        /// Create a new execution router.
        pub fn new(provider: Arc<RustStackApiGatewayV2>) -> Self {
            Self { provider }
        }
    }

    impl ServiceRouter for ApiGatewayV2ExecutionRouter {
        fn name(&self) -> &'static str {
            "apigatewayv2-execution"
        }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            req.uri().path().starts_with("/_aws/execute-api/")
        }

        fn call(
            &self,
            req: http::Request<Incoming>,
        ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let provider = Arc::clone(&self.provider);
            Box::pin(async move {
                let method = req.method().clone();
                let path = req.uri().path().to_owned();
                let headers = req.headers().clone();

                // Strip the /_aws/execute-api prefix.
                let exec_path = path.strip_prefix("/_aws/execute-api").unwrap_or(&path);

                let target = match parse_execution_path(exec_path) {
                    Ok(t) => t,
                    Err(e) => {
                        let body = serde_json::json!({"message": e.to_string()});
                        let resp = http::Response::builder()
                            .status(http::StatusCode::BAD_REQUEST)
                            .header("content-type", "application/json")
                            .body(gateway_body_from_string(body.to_string()))
                            .unwrap_or_else(|_| {
                                http::Response::new(gateway_body_from_string(
                                    "Bad Request".to_owned(),
                                ))
                            });
                        return Ok(resp);
                    }
                };

                // Collect request body.
                let body_bytes: Bytes = match http_body_util::BodyExt::collect(req.into_body())
                    .await
                    .map(http_body_util::Collected::to_bytes)
                {
                    Ok(b) => b,
                    Err(e) => {
                        let body =
                            serde_json::json!({"message": format!("Failed to read body: {e}")});
                        let resp = http::Response::builder()
                            .status(http::StatusCode::BAD_REQUEST)
                            .header("content-type", "application/json")
                            .body(gateway_body_from_string(body.to_string()))
                            .unwrap_or_else(|_| {
                                http::Response::new(gateway_body_from_string(
                                    "Bad Request".to_owned(),
                                ))
                            });
                        return Ok(resp);
                    }
                };

                match handle_execution(
                    &provider,
                    &target.api_id,
                    &target.stage_name,
                    &method,
                    &target.path,
                    &headers,
                    &body_bytes,
                )
                .await
                {
                    Ok(resp) => {
                        let (parts, body) = resp.into_parts();
                        Ok(http::Response::from_parts(
                            parts,
                            gateway_body_from_string(String::from_utf8_lossy(&body).into_owned()),
                        ))
                    }
                    Err(e) => {
                        let status = match &e {
                            ruststack_apigatewayv2_core::error::ApiGatewayV2ServiceError::NotFound(_) => {
                                http::StatusCode::NOT_FOUND
                            }
                            ruststack_apigatewayv2_core::error::ApiGatewayV2ServiceError::BadRequest(_) => {
                                http::StatusCode::BAD_REQUEST
                            }
                            _ => http::StatusCode::INTERNAL_SERVER_ERROR,
                        };
                        let body = serde_json::json!({"message": e.to_string()});
                        let resp = http::Response::builder()
                            .status(status)
                            .header("content-type", "application/json")
                            .body(gateway_body_from_string(body.to_string()))
                            .unwrap_or_else(|_| {
                                http::Response::new(gateway_body_from_string(
                                    "Internal error".to_owned(),
                                ))
                            });
                        Ok(resp)
                    }
                }
            })
        }
    }
}

#[cfg(feature = "apigatewayv2")]
pub use apigatewayv2_router::{ApiGatewayV2ExecutionRouter, ApiGatewayV2ManagementRouter};
