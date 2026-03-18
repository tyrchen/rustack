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
