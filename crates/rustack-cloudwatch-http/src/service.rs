//! CloudWatch HTTP service implementing the hyper `Service` trait.
//!
//! CloudWatch Metrics supports two wire protocols:
//! - **awsQuery**: form-urlencoded requests, XML responses (legacy SDKs).
//! - **rpcv2Cbor**: CBOR requests/responses (AWS SDK v1.108+).
//!
//! The protocol is detected automatically from request headers and URL path.

use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use hyper::body::Incoming;
use rustack_cloudwatch_model::{error::CloudWatchError, operations::CloudWatchOperation};

use crate::{
    body::CloudWatchResponseBody,
    dispatch::{CloudWatchHandler, Protocol, dispatch_operation},
    request::parse_form_params,
    response::{CONTENT_TYPE, cbor_error_response, error_to_response},
    router::resolve_operation,
};

/// Configuration for the CloudWatch HTTP service.
#[derive(Clone)]
pub struct CloudWatchHttpConfig {
    /// Whether to skip AWS signature validation.
    pub skip_signature_validation: bool,
    /// The AWS region this service is running in.
    pub region: String,
    /// Credential provider for signature validation.
    pub credential_provider: Option<Arc<dyn rustack_auth::CredentialProvider>>,
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
    let content_type = parts
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.contains("application/cbor") {
        return Protocol::RpcV2Cbor;
    }
    // awsJson_1.0: AWS CLI v2.27+ sends X-Amz-Target with JSON body.
    if content_type.contains("application/x-amz-json") && parts.headers.contains_key("x-amz-target")
    {
        return Protocol::AwsJson;
    }
    Protocol::AwsQuery
}

/// Extract the operation name from an `X-Amz-Target` header.
///
/// Format: `GraniteServiceVersion20100801.OperationName`
fn extract_json_operation(headers: &http::HeaderMap) -> Option<&str> {
    let target = headers.get("x-amz-target")?.to_str().ok()?;
    target.split('.').nth(1)
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
            rustack_cloudwatch_model::error::CloudWatchErrorCode::InvalidParameterValueException,
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
        Protocol::AwsJson => match extract_json_operation(&parts.headers) {
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
        },
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
    if let Err(auth_err) = rustack_auth::AuthMode::resolve(
        config.skip_signature_validation,
        config.credential_provider.as_deref(),
    )
    .and_then(|mode| mode.verify(&parts, &rustack_auth::hash_payload(&body)))
    {
        let mut err = CloudWatchError::with_message(
            rustack_cloudwatch_model::error::CloudWatchErrorCode::InternalServiceFault,
            auth_err.to_string(),
        );
        err.status_code = http::StatusCode::FORBIDDEN;
        return make_error_response(&err, request_id, protocol);
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
        Protocol::AwsJson => json_error_response(err, request_id),
        Protocol::RpcV2Cbor => cbor_error_response(err, request_id),
    }
}

/// Build a JSON error response for awsJson_1.0 protocol.
fn json_error_response(
    err: &CloudWatchError,
    request_id: &str,
) -> http::Response<CloudWatchResponseBody> {
    let body = serde_json::json!({
        "__type": err.code.as_str(),
        "message": err.message,
    });
    http::Response::builder()
        .status(err.status_code)
        .header("content-type", "application/x-amz-json-1.0")
        .header("x-amzn-requestid", request_id)
        .body(CloudWatchResponseBody::from_bytes(
            serde_json::to_vec(&body).unwrap_or_default(),
        ))
        .expect("valid JSON error response")
}

/// Collect the incoming body into a single `Bytes` buffer.
async fn collect_body(incoming: Incoming) -> Result<Bytes, CloudWatchError> {
    rustack_core::http::collect_body(incoming, rustack_core::http::BodyBudget::control())
        .await
        .map_err(|e| {
            let mut err = CloudWatchError::internal_error(e.to_string());
            err.status_code = e.status_code();
            err
        })
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

    headers.insert("server", http::HeaderValue::from_static("Rustack"));

    headers.insert(
        "access-control-allow-origin",
        http::HeaderValue::from_static("*"),
    );

    response
}
