//! Origin dispatch switchyard.
//!
//! The data plane picks the target origin for a request and funnels it
//! through one of the supported origin kinds:
//! - S3 (in-process function call into `rustack-s3-core`)
//! - Custom HTTP (via `reqwest`, behind the `http-origin` feature)

use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, Response, StatusCode};
use rustack_cloudfront_model::{CustomHeader, Origin};
use rustack_s3_core::RustackS3;
use rustack_s3_model::{
    error::S3ErrorCode,
    input::{GetObjectInput, HeadObjectInput},
};
use tracing::debug;

use crate::{
    error::DataPlaneError,
    transform::{apply_custom_headers, concat_origin_path, filter_inbound_headers},
};

/// Kind of origin the dispatcher is sending to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginKind {
    /// S3 bucket endpoint.
    S3,
    /// Custom HTTP endpoint.
    Http,
    /// API Gateway v2 execute-api endpoint.
    ApiGatewayV2,
    /// Lambda function URL.
    LambdaUrl,
    /// Unknown — treat as HTTP.
    Unknown,
}

/// Classify an origin by inspecting its domain name and config.
#[must_use]
pub fn classify_origin(origin: &Origin) -> OriginKind {
    if origin.s3_origin_config.is_some() || is_s3_domain(&origin.domain_name) {
        return OriginKind::S3;
    }
    if origin.custom_origin_config.is_some() {
        if is_apigw_domain(&origin.domain_name) {
            return OriginKind::ApiGatewayV2;
        }
        if is_lambda_url_domain(&origin.domain_name) {
            return OriginKind::LambdaUrl;
        }
        return OriginKind::Http;
    }
    if is_apigw_domain(&origin.domain_name) {
        OriginKind::ApiGatewayV2
    } else if is_lambda_url_domain(&origin.domain_name) {
        OriginKind::LambdaUrl
    } else {
        OriginKind::Unknown
    }
}

fn is_s3_domain(host: &str) -> bool {
    (host.contains(".s3.") && host.ends_with(".amazonaws.com"))
        || host.ends_with(".s3.amazonaws.com")
        || (host.contains(".s3-website-") && host.ends_with(".amazonaws.com"))
}

fn is_apigw_domain(host: &str) -> bool {
    host.contains(".execute-api.") && host.ends_with(".amazonaws.com")
}

fn is_lambda_url_domain(host: &str) -> bool {
    host.contains(".lambda-url.") && host.ends_with(".on.aws")
}

/// Extract the S3 bucket name from an `Origin.DomainName`.
#[must_use]
pub fn extract_s3_bucket(host: &str) -> Option<&str> {
    if let Some(idx) = host.find(".s3.") {
        return Some(&host[..idx]);
    }
    if let Some(idx) = host.find(".s3-website-") {
        return Some(&host[..idx]);
    }
    host.strip_suffix(".s3.amazonaws.com")
}

/// Dispatch to an S3 origin in-process.
pub async fn dispatch_s3_origin(
    s3: &Arc<RustackS3>,
    bucket: &str,
    origin_path: &str,
    request_path: &str,
    method: &Method,
    inbound_headers: &HeaderMap,
    origin_custom_headers: &[CustomHeader],
    forward_user_metadata: bool,
) -> Result<Response<Bytes>, DataPlaneError> {
    let joined = concat_origin_path(origin_path, request_path);
    let key = joined.trim_start_matches('/').to_owned();
    debug!(bucket = %bucket, key = %key, method = ?method, "cloudfront dispatch_s3");

    let mut upstream_headers = filter_inbound_headers(inbound_headers);
    apply_custom_headers(&mut upstream_headers, origin_custom_headers);

    match *method {
        Method::GET => {
            let input = GetObjectInput {
                bucket: bucket.to_owned(),
                key: key.clone(),
                range: upstream_headers
                    .get(http::header::RANGE)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned),
                ..GetObjectInput::default()
            };
            let out = s3.handle_get_object(input).await.map_err(to_dp_error)?;
            let body = out.body.map(|b| b.data).unwrap_or_default();
            let mut builder = Response::builder().status(StatusCode::OK);
            if let Some(ct) = out.content_type {
                builder = builder.header(http::header::CONTENT_TYPE, ct);
            }
            if let Some(len) = out.content_length {
                builder = builder.header(http::header::CONTENT_LENGTH, len.to_string());
            }
            if let Some(etag) = out.e_tag {
                builder = builder.header(http::header::ETAG, etag);
            }
            if let Some(cc) = out.cache_control {
                builder = builder.header(http::header::CACHE_CONTROL, cc);
            }
            if let Some(lm) = out.last_modified {
                if let Ok(hv) = HeaderValue::from_str(&lm.to_rfc2822()) {
                    builder = builder.header(http::header::LAST_MODIFIED, hv);
                }
            }
            if forward_user_metadata {
                for (k, v) in &out.metadata {
                    let name = format!("x-amz-meta-{k}");
                    if let (Ok(hn), Ok(hv)) = (
                        HeaderName::from_bytes(name.as_bytes()),
                        HeaderValue::from_str(v),
                    ) {
                        builder = builder.header(hn, hv);
                    }
                }
            }
            builder
                .body(body)
                .map_err(|e| DataPlaneError::Internal(format!("build response: {e}")))
        }
        Method::HEAD => {
            let input = HeadObjectInput {
                bucket: bucket.to_owned(),
                key: key.clone(),
                ..HeadObjectInput::default()
            };
            let out = s3.handle_head_object(input).await.map_err(to_dp_error)?;
            let mut builder = Response::builder().status(StatusCode::OK);
            if let Some(ct) = out.content_type {
                builder = builder.header(http::header::CONTENT_TYPE, ct);
            }
            if let Some(len) = out.content_length {
                builder = builder.header(http::header::CONTENT_LENGTH, len.to_string());
            }
            if let Some(etag) = out.e_tag {
                builder = builder.header(http::header::ETAG, etag);
            }
            if let Some(cc) = out.cache_control {
                builder = builder.header(http::header::CACHE_CONTROL, cc);
            }
            if let Some(lm) = out.last_modified {
                if let Ok(hv) = HeaderValue::from_str(&lm.to_rfc2822()) {
                    builder = builder.header(http::header::LAST_MODIFIED, hv);
                }
            }
            builder
                .body(Bytes::new())
                .map_err(|e| DataPlaneError::Internal(format!("build response: {e}")))
        }
        _ => Err(DataPlaneError::MethodNotAllowed(format!(
            "CloudFront data plane only supports GET/HEAD for S3 origins; got {method}"
        ))),
    }
}

fn to_dp_error(err: rustack_s3_model::error::S3Error) -> DataPlaneError {
    match err.code {
        S3ErrorCode::NoSuchKey => DataPlaneError::ObjectNotFound(err.message),
        S3ErrorCode::NoSuchBucket => DataPlaneError::OriginServerError {
            status: 502,
            message: format!("NoSuchBucket: {}", err.message),
        },
        S3ErrorCode::AccessDenied => DataPlaneError::OriginClientError {
            status: 403,
            message: err.message,
        },
        _ => {
            let status = err.status_code.as_u16();
            if status >= 500 {
                DataPlaneError::OriginServerError {
                    status,
                    message: err.message,
                }
            } else if status >= 400 {
                DataPlaneError::OriginClientError {
                    status,
                    message: err.message,
                }
            } else {
                DataPlaneError::Internal(err.message)
            }
        }
    }
}

/// Dispatch to a custom HTTP origin via `reqwest`.
#[cfg(feature = "http-origin")]
pub async fn dispatch_http_origin(
    client: &reqwest::Client,
    origin: &Origin,
    request_path: &str,
    method: &Method,
    inbound_headers: &HeaderMap,
    body: Bytes,
    max_body: usize,
) -> Result<Response<Bytes>, DataPlaneError> {
    let cfg = origin
        .custom_origin_config
        .as_ref()
        .ok_or_else(|| DataPlaneError::BehaviorResolution("missing CustomOriginConfig".into()))?;
    let scheme = match cfg.origin_protocol_policy.as_str() {
        "http-only" | "match-viewer" => "http",
        _ => "https",
    };
    let port = match scheme {
        "http" => cfg.http_port,
        _ => cfg.https_port,
    };
    let effective_port = if port == 0 {
        if scheme == "http" { 80 } else { 443 }
    } else {
        port
    };

    let joined = concat_origin_path(&origin.origin_path, request_path);
    let url = if (scheme == "http" && effective_port == 80)
        || (scheme == "https" && effective_port == 443)
    {
        format!("{scheme}://{}{joined}", origin.domain_name)
    } else {
        format!("{scheme}://{}:{effective_port}{joined}", origin.domain_name)
    };

    let mut upstream = filter_inbound_headers(inbound_headers);
    apply_custom_headers(&mut upstream, &origin.custom_headers);

    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|e| DataPlaneError::Internal(format!("invalid method: {e}")))?;

    let req = client
        .request(reqwest_method, &url)
        .headers(translate_headers_to_reqwest(&upstream))
        .body(body.to_vec())
        .build()
        .map_err(|e| DataPlaneError::Internal(format!("build reqwest: {e}")))?;

    let resp = client.execute(req).await.map_err(|e| {
        if e.is_timeout() {
            DataPlaneError::OriginServerError {
                status: 504,
                message: format!("origin timeout: {e}"),
            }
        } else {
            DataPlaneError::OriginServerError {
                status: 502,
                message: format!("origin unreachable: {e}"),
            }
        }
    })?;

    let status =
        StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let upstream_headers = translate_headers_from_reqwest(resp.headers());
    let body_bytes = resp
        .bytes()
        .await
        .map_err(|e| DataPlaneError::Internal(format!("read body: {e}")))?;
    if body_bytes.len() > max_body {
        return Err(DataPlaneError::PayloadTooLarge(format!(
            "upstream body {} bytes exceeds cap {}",
            body_bytes.len(),
            max_body
        )));
    }

    let mut builder = Response::builder().status(status);
    for (k, v) in upstream_headers.iter() {
        builder = builder.header(k, v);
    }
    builder
        .body(body_bytes)
        .map_err(|e| DataPlaneError::Internal(format!("build response: {e}")))
}

#[cfg(feature = "http-origin")]
fn translate_headers_to_reqwest(h: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut out = reqwest::header::HeaderMap::with_capacity(h.len());
    for (k, v) in h {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(v.as_bytes()),
        ) {
            out.insert(name, value);
        }
    }
    out
}

#[cfg(feature = "http-origin")]
fn translate_headers_from_reqwest(h: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::with_capacity(h.len());
    for (k, v) in h {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(k.as_str().as_bytes()),
            HeaderValue::from_bytes(v.as_bytes()),
        ) {
            // Skip hop-by-hop and content-length (reqwest sets it).
            let lower = name.as_str().to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "transfer-encoding"
                    | "connection"
                    | "keep-alive"
                    | "proxy-authenticate"
                    | "proxy-authorization"
                    | "te"
                    | "trailers"
                    | "upgrade"
            ) {
                continue;
            }
            out.insert(name, value);
        }
    }
    out
}
