//! CloudWatch HTTP service implementing the hyper `Service` trait.
//!
//! CloudWatch Metrics supports two wire protocols:
//! - **awsQuery**: form-urlencoded requests, XML responses (legacy SDKs).
//! - **rpcv2Cbor**: CBOR requests/responses (AWS SDK v1.108+).
//!
//! The protocol is detected automatically from request headers and URL path.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;

use ruststack_cloudwatch_model::error::CloudWatchError;
use ruststack_cloudwatch_model::operations::CloudWatchOperation;

use crate::body::CloudWatchResponseBody;
use crate::dispatch::{CloudWatchHandler, Protocol, dispatch_operation};
use crate::request::parse_form_params;
use crate::response::{CONTENT_TYPE, cbor_error_response, error_to_response};
use crate::router::resolve_operation;

/// Configuration for the CloudWatch HTTP service.
#[derive(Clone)]
pub struct CloudWatchHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn ruststack_auth::CredentialProvider>>,
}

impl std::fmt::Debug for CloudWatchHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudWatchHttpConfig")
            .field("skip_signature_validation", &self.skip_signature_validation)
            .field("region", &self.region)
            .field(
                "credential_provider",
                &self.credential_provider.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for CloudWatchHttpConfig {
    fn default() -> Self {
        Self {
            skip_signature_validation: true,
            region: "us-east-1".to_owned(),
            credential_provider: None,
        }
    }
}

/// Hyper `Service` implementation for CloudWatch Metrics.
///
/// Wraps a [`CloudWatchHandler`] implementation and routes incoming HTTP
/// requests to the appropriate CloudWatch operation handler.
#[derive(Debug)]
pub struct CloudWatchHttpService<H: CloudWatchHandler> {
    handler: Arc<H>,
    config: Arc<CloudWatchHttpConfig>,
}

impl<H: CloudWatchHandler> CloudWatchHttpService<H> {
    /// Create a new `CloudWatchHttpService`.
    pub fn new(handler: Arc<H>, config: CloudWatchHttpConfig) -> Self {
        Self {
            handler,
            config: Arc::new(config),
        }
    }
}

impl<H: CloudWatchHandler> Clone for CloudWatchHttpService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
            config: Arc::clone(&self.config),
        }
    }
}

impl<H: CloudWatchHandler> hyper::service::Service<http::Request<Incoming>>
    for CloudWatchHttpService<H>
{
    type Response = http::Response<CloudWatchResponseBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: http::Request<Incoming>) -> Self::Future {
        let handler = Arc::clone(&self.handler);
        let config = Arc::clone(&self.config);
        let request_id = uuid::Uuid::new_v4().to_string();

        Box::pin(async move {
            let response = process_request(req, handler.as_ref(), &config, &request_id).await;
            let response = add_common_headers(response);
            Ok(response)
        })
    }
}

/// Detect the wire protocol from request headers and path.
fn detect_protocol(parts: &http::request::Parts) -> Protocol {
    // Check for rpcv2Cbor indicators.
    if parts
        .headers
        .get("smithy-protocol")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "rpc-v2-cbor")
    {
        return Protocol::RpcV2Cbor;
    }
    if parts
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("application/cbor"))
    {
        return Protocol::RpcV2Cbor;
    }
    Protocol::AwsQuery
}

/// Extract the operation name from an rpcv2Cbor URL path.
///
/// Path format: `/service/GraniteServiceVersion20100801/operation/{OperationName}`
fn extract_cbor_operation(path: &str) -> Option<&str> {
    let segments: Vec<&str> = path.split('/').collect();
    // ["", "service", "GraniteServiceVersion20100801", "operation", "OpName"]
    if segments.len() >= 5 && segments[1] == "service" && segments[3] == "operation" {
        Some(segments[4])
    } else {
        None
    }
}

/// Process a single CloudWatch HTTP request through the full pipeline.
async fn process_request<H: CloudWatchHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &CloudWatchHttpConfig,
    request_id: &str,
) -> http::Response<CloudWatchResponseBody> {
    let (parts, incoming) = req.into_parts();

    // Detect protocol.
    let protocol = detect_protocol(&parts);

    // Verify POST method.
    if parts.method != http::Method::POST {
        let err = CloudWatchError::with_message(
            ruststack_cloudwatch_model::error::CloudWatchErrorCode::InvalidParameterValueException,
            format!("CloudWatch requires POST method, got {}", parts.method),
        );
        return make_error_response(&err, request_id, protocol);
    }

    // Collect body.
    let body = match collect_body(incoming).await {
        Ok(body) => body,
        Err(err) => return make_error_response(&err, request_id, protocol),
    };

    // Resolve operation based on protocol.
    let op = match protocol {
        Protocol::AwsQuery => {
            let params = parse_form_params(&body);
            match resolve_operation(&params) {
                Ok(op) => op,
                Err(err) => return make_error_response(&err, request_id, protocol),
            }
        }
        Protocol::RpcV2Cbor => {
            let path = parts.uri.path();
            match extract_cbor_operation(path) {
                Some(name) => match CloudWatchOperation::from_name(name) {
                    Some(op) => op,
                    None => {
                        return make_error_response(
                            &CloudWatchError::unknown_operation(name),
                            request_id,
                            protocol,
                        );
                    }
                },
                None => {
                    return make_error_response(
                        &CloudWatchError::missing_action(),
                        request_id,
                        protocol,
                    );
                }
            }
        }
    };

    // Authenticate (if enabled).
    if !config.skip_signature_validation {
        if let Some(ref cred_provider) = config.credential_provider {
            let body_hash = ruststack_auth::hash_payload(&body);
            if let Err(auth_err) =
                ruststack_auth::verify_sigv4(&parts, &body_hash, cred_provider.as_ref())
            {
                let err = CloudWatchError::with_message(
                    ruststack_cloudwatch_model::error::CloudWatchErrorCode::InternalServiceFault,
                    auth_err.to_string(),
                );
                return make_error_response(&err, request_id, protocol);
            }
        }
    }

    // Dispatch to handler.
    match dispatch_operation(handler, op, body, protocol).await {
        Ok(response) => response,
        Err(err) => make_error_response(&err, request_id, protocol),
    }
}

/// Return the appropriate error response for the given protocol.
fn make_error_response(
    err: &CloudWatchError,
    request_id: &str,
    protocol: Protocol,
) -> http::Response<CloudWatchResponseBody> {
    match protocol {
        Protocol::AwsQuery => error_to_response(err, request_id),
        Protocol::RpcV2Cbor => cbor_error_response(err, request_id),
    }
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, CloudWatchError> {
    incoming
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .map_err(|e| CloudWatchError::internal_error(format!("Failed to read request body: {e}")))
}

/// Add common response headers to every CloudWatch response.
///
/// Protocol-specific headers (content-type, smithy-protocol, x-amzn-requestid)
/// are already set by the individual response builders; this only adds
/// server-level headers that apply to every response.
fn add_common_headers(
    mut response: http::Response<CloudWatchResponseBody>,
) -> http::Response<CloudWatchResponseBody> {
    let headers = response.headers_mut();

    // Fallback content-type for awsQuery when not already set.
    headers
        .entry("content-type")
        .or_insert(http::HeaderValue::from_static(CONTENT_TYPE));

    headers.insert("server", http::HeaderValue::from_static("RustStack"));

    headers.insert(
        "access-control-allow-origin",
        http::HeaderValue::from_static("*"),
    );

    response
}
