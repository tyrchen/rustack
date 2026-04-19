//! Top-level data-plane entry point.
//!
//! Exposes a single `handle_request` function: the gateway router feeds raw
//! HTTP requests in, and the data plane resolves the distribution, selects a
//! cache behavior, dispatches to origin, and returns the response.

use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, Response, StatusCode, Uri};
use rustack_cloudfront_core::RustackCloudFront;
use rustack_cloudfront_model::{CacheBehavior, DistributionConfig};
use rustack_s3_core::RustackS3;
use tracing::debug;

use crate::{
    behavior::select_behavior,
    config::DataPlaneConfig,
    dispatch::{OriginKind, classify_origin, dispatch_s3_origin, extract_s3_bucket},
    divergence::{
        DivergenceTracker, handle_function, handle_lambda_edge, handle_signed_url,
        handle_viewer_protocol,
    },
    error::DataPlaneError,
    host::match_host,
    transform::{
        add_cloudfront_response_headers, apply_response_headers_policy,
        rewrite_path_with_default_root,
    },
};

/// Built data plane, ready to serve requests.
#[derive(Debug, Clone)]
pub struct DataPlane {
    cloudfront: Arc<RustackCloudFront>,
    s3: Option<Arc<RustackS3>>,
    config: Arc<DataPlaneConfig>,
    divergence: Arc<DivergenceTracker>,
    #[cfg(feature = "http-origin")]
    http_client: reqwest::Client,
}

/// Builder for `DataPlane`.
#[derive(Debug, Default)]
pub struct DataPlaneBuilder {
    cloudfront: Option<Arc<RustackCloudFront>>,
    s3: Option<Arc<RustackS3>>,
    config: DataPlaneConfig,
}

impl DataPlaneBuilder {
    /// Required: CloudFront management-plane handle.
    #[must_use]
    pub fn cloudfront(mut self, cf: Arc<RustackCloudFront>) -> Self {
        self.cloudfront = Some(cf);
        self
    }

    /// Optional: S3 provider for in-process S3 origin dispatch.
    #[must_use]
    pub fn s3(mut self, s3: Arc<RustackS3>) -> Self {
        self.s3 = Some(s3);
        self
    }

    /// Supply an explicit config.
    #[must_use]
    pub fn config(mut self, c: DataPlaneConfig) -> Self {
        self.config = c;
        self
    }

    /// Construct the `DataPlane`.
    pub fn build(self) -> Result<DataPlane, &'static str> {
        let cf = self.cloudfront.ok_or("CloudFront provider is required")?;
        let divergence = DivergenceTracker::new(self.config.divergence_log_interval);
        #[cfg(feature = "http-origin")]
        let http_client = reqwest::Client::builder()
            .timeout(self.config.http_origin_timeout)
            .build()
            .map_err(|_| "failed to build reqwest client")?;
        Ok(DataPlane {
            cloudfront: cf,
            s3: self.s3,
            config: Arc::new(self.config),
            divergence,
            #[cfg(feature = "http-origin")]
            http_client,
        })
    }
}

impl DataPlane {
    /// Begin construction.
    #[must_use]
    pub fn builder() -> DataPlaneBuilder {
        DataPlaneBuilder::default()
    }

    /// Runtime config.
    #[must_use]
    pub fn config(&self) -> &DataPlaneConfig {
        &self.config
    }

    /// Check whether a host maps to a known distribution.
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        match_host(&self.cloudfront, &self.config.domain_suffix, host).is_some()
    }

    /// Resolve a path-based URL (`/_aws/cloudfront/{id}/...`) to a distribution ID + path.
    #[must_use]
    pub fn parse_path_based(path: &str) -> Option<(String, String)> {
        let rest = path.strip_prefix("/_aws/cloudfront/")?;
        let (id, rest) = rest.split_once('/').map_or((rest, ""), |(a, b)| (a, b));
        let id = id.to_ascii_uppercase();
        if id.is_empty() {
            return None;
        }
        let object_path = if rest.is_empty() {
            "/".to_owned()
        } else {
            format!("/{rest}")
        };
        Some((id, object_path))
    }

    /// Resolve a `(distribution_id, object_path)` from an incoming request.
    ///
    /// Tries path-based URL first, then host-based routing.
    pub fn resolve_request(&self, uri: &Uri, headers: &HeaderMap) -> Option<(String, String)> {
        if let Some(result) = Self::parse_path_based(uri.path()) {
            return Some(result);
        }
        let host = headers
            .get(http::header::HOST)
            .and_then(|v| v.to_str().ok())?;
        let id = match_host(&self.cloudfront, &self.config.domain_suffix, host)?;
        Some((id, uri.path().to_owned()))
    }

    /// Serve a request end-to-end. Returns a fully populated response.
    pub async fn handle_request(
        &self,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response<Bytes> {
        let (id, object_path) = match self.resolve_request(&uri, &headers) {
            Some(x) => x,
            None => return error_response(&DataPlaneError::NoSuchDistribution(uri.path().into())),
        };
        debug!(distribution_id = %id, object_path = %object_path, "cf dataplane resolved");
        let dist = match self.cloudfront.get_distribution(&id) {
            Ok(d) => d,
            Err(_) => return error_response(&DataPlaneError::NoSuchDistribution(id)),
        };
        if !dist.config.enabled {
            return error_response(&DataPlaneError::DistributionDisabled(id));
        }

        // Apply DefaultRootObject rewrite first.
        let effective_path =
            rewrite_path_with_default_root(&dist.config, &object_path).into_owned();

        // Select the cache behavior.
        let behavior = select_behavior(&dist.config, &effective_path).clone();

        // Enforce AllowedMethods.
        if !method_is_allowed(&behavior, &method) {
            return error_response(&DataPlaneError::MethodNotAllowed(format!(
                "{method} is not in AllowedMethods for behavior {}",
                behavior.path_pattern
            )));
        }

        // Divergence checks (emit warn! or hard-fail).
        if let Err(e) = self.check_divergence(&id, &behavior) {
            return error_response(&e);
        }

        // Resolve origin.
        let origin_id = behavior.target_origin_id.clone();
        let origin_opt = dist
            .config
            .origins
            .iter()
            .find(|o| o.id == origin_id)
            .cloned();
        let origin = match origin_opt {
            Some(o) => o,
            None => {
                return error_response(&DataPlaneError::BehaviorResolution(format!(
                    "TargetOriginId {origin_id} does not match any Origin"
                )));
            }
        };

        // Dispatch.
        let kind = classify_origin(&origin);
        let mut response = match kind {
            OriginKind::S3 => {
                let s3 = match self.s3.as_ref() {
                    Some(s) => s,
                    None => {
                        return error_response(&DataPlaneError::BehaviorResolution(
                            "S3 origin requested but no S3 provider configured".into(),
                        ));
                    }
                };
                let bucket = match extract_s3_bucket(&origin.domain_name) {
                    Some(b) => b.to_owned(),
                    None => {
                        return error_response(&DataPlaneError::BehaviorResolution(format!(
                            "cannot parse S3 bucket from {}",
                            origin.domain_name
                        )));
                    }
                };
                match dispatch_s3_origin(
                    s3,
                    &bucket,
                    &origin.origin_path,
                    &effective_path,
                    &method,
                    &headers,
                    &origin.custom_headers,
                    self.config.forward_user_metadata,
                )
                .await
                {
                    Ok(r) => r,
                    Err(e) => return self.handle_origin_error(&dist.config, e).await,
                }
            }
            #[cfg(feature = "http-origin")]
            OriginKind::Http | OriginKind::ApiGatewayV2 | OriginKind::LambdaUrl => {
                match crate::dispatch::dispatch_http_origin(
                    &self.http_client,
                    &origin,
                    &effective_path,
                    &method,
                    &headers,
                    body,
                    self.config.max_upstream_body_bytes,
                )
                .await
                {
                    Ok(r) => r,
                    Err(e) => return self.handle_origin_error(&dist.config, e).await,
                }
            }
            #[cfg(not(feature = "http-origin"))]
            OriginKind::Http | OriginKind::ApiGatewayV2 | OriginKind::LambdaUrl => {
                return error_response(&DataPlaneError::BehaviorResolution(
                    "custom HTTP origin requires the `http-origin` feature".into(),
                ));
            }
            OriginKind::Unknown => {
                return error_response(&DataPlaneError::BehaviorResolution(format!(
                    "cannot classify origin {}",
                    origin.domain_name
                )));
            }
        };

        // Apply response headers policy.
        if !behavior.response_headers_policy_id.is_empty() {
            if let Ok(policy) = self
                .cloudfront
                .get_response_headers_policy(&behavior.response_headers_policy_id)
            {
                let origin_header = headers.get(http::header::ORIGIN).cloned();
                apply_response_headers_policy(
                    response.headers_mut(),
                    &policy.config,
                    origin_header.as_ref(),
                );
            }
        }

        add_cloudfront_response_headers(response.headers_mut());
        let _ = body; // body consumed above in HTTP dispatch path; placate compiler on other paths.
        response
    }

    async fn handle_origin_error(
        &self,
        config: &DistributionConfig,
        err: DataPlaneError,
    ) -> Response<Bytes> {
        let status = err.http_status();
        // Check CustomErrorResponses.
        for cer in &config.custom_error_responses {
            if cer.error_code == i32::from(status) && !cer.response_page_path.is_empty() {
                // For simplicity we serve the configured response code with an
                // empty body (real AWS fetches the path from origin again).
                let effective_status = cer
                    .response_code
                    .parse::<u16>()
                    .ok()
                    .and_then(|c| StatusCode::from_u16(c).ok())
                    .unwrap_or(StatusCode::from_u16(status).unwrap_or(StatusCode::NOT_FOUND));
                let mut builder = Response::builder().status(effective_status);
                if cer.error_caching_min_ttl > 0 {
                    builder = builder.header(
                        http::header::CACHE_CONTROL,
                        format!("max-age={}", cer.error_caching_min_ttl),
                    );
                }
                return builder
                    .body(Bytes::from_static(b""))
                    .unwrap_or_else(|_| error_response(&err));
            }
        }
        error_response(&err)
    }

    fn check_divergence(
        &self,
        distribution_id: &str,
        behavior: &CacheBehavior,
    ) -> Result<(), DataPlaneError> {
        let fail = self.config.fail_on_function;
        if !behavior.lambda_function_associations.is_empty() {
            handle_lambda_edge(&self.divergence, fail, distribution_id)?;
        }
        if !behavior.function_associations.is_empty() {
            handle_function(&self.divergence, fail, distribution_id)?;
        }
        if (behavior.trusted_key_groups_enabled && !behavior.trusted_key_groups.is_empty())
            || (behavior.trusted_signers_enabled && !behavior.trusted_signers.is_empty())
        {
            handle_signed_url(&self.divergence, fail, distribution_id)?;
        }
        if !behavior.viewer_protocol_policy.is_empty() {
            handle_viewer_protocol(
                &self.divergence,
                fail,
                distribution_id,
                &behavior.viewer_protocol_policy,
            )?;
        }
        Ok(())
    }
}

fn method_is_allowed(behavior: &CacheBehavior, method: &Method) -> bool {
    if behavior.allowed_methods.is_empty() {
        // Default: GET, HEAD.
        return matches!(*method, Method::GET | Method::HEAD);
    }
    behavior
        .allowed_methods
        .iter()
        .any(|m| m.eq_ignore_ascii_case(method.as_str()))
}

/// Build a tiny HTML error response matching CloudFront's envelope.
pub fn error_response(err: &DataPlaneError) -> Response<Bytes> {
    let status =
        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = format!(
        "<?xml version=\"1.0\"?>\n<Error><Code>{}</Code><Message>{}</Message></Error>",
        err.error_type(),
        xml_escape(&err.to_string())
    );
    let mut resp = Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/xml")
        .body(Bytes::from(body))
        .unwrap_or_else(|_| Response::new(Bytes::new()));
    if let Ok(hv) = HeaderValue::from_str(err.error_type()) {
        resp.headers_mut()
            .insert(HeaderName::from_static("x-amzn-errortype"), hv);
    }
    add_cloudfront_response_headers(resp.headers_mut());
    resp
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_path_based() {
        let (id, path) = DataPlane::parse_path_based("/_aws/cloudfront/e1abcd/index.html").unwrap();
        assert_eq!(id, "E1ABCD");
        assert_eq!(path, "/index.html");

        let (id, path) = DataPlane::parse_path_based("/_aws/cloudfront/e1abcd").unwrap();
        assert_eq!(id, "E1ABCD");
        assert_eq!(path, "/");

        assert!(DataPlane::parse_path_based("/wrong/prefix").is_none());
    }
}
