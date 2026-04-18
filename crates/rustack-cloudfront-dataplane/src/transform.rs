//! Request/response transformations applied during dispatch.

use std::borrow::Cow;

use http::{HeaderMap, HeaderName, HeaderValue};
use rustack_cloudfront_model::{CustomHeader, DistributionConfig, ResponseHeadersPolicyConfig};

/// Apply `DefaultRootObject` rewrite: `/` → `/{default}`.
#[must_use]
pub fn apply_default_root_object<'a>(path: &'a str, default: &str) -> Cow<'a, str> {
    if path == "/" && !default.is_empty() {
        Cow::Owned(format!("/{}", default.trim_start_matches('/')))
    } else {
        Cow::Borrowed(path)
    }
}

/// Concatenate `OriginPath` + request path, producing exactly one separator.
#[must_use]
pub fn concat_origin_path(origin_path: &str, request_path: &str) -> String {
    let left = origin_path.trim_end_matches('/');
    let right = if request_path.starts_with('/') {
        request_path.to_owned()
    } else {
        format!("/{request_path}")
    };
    format!("{left}{right}")
}

/// Hop-by-hop and CloudFront-reserved header names to strip on the way upstream.
pub const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "expect",
];

/// Copy inbound headers to upstream, dropping hop-by-hop and reserved entries.
#[must_use]
pub fn filter_inbound_headers(inbound: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::with_capacity(inbound.len());
    for (name, value) in inbound {
        let lower = name.as_str().to_ascii_lowercase();
        if HOP_BY_HOP.contains(&lower.as_str()) {
            continue;
        }
        if lower.starts_with("x-amz-cf-") {
            continue;
        }
        if lower == "host" {
            continue; // Host is set per-upstream request.
        }
        out.insert(name.clone(), value.clone());
    }
    out
}

/// Apply Origin.CustomHeaders to the upstream header map (overwriting).
pub fn apply_custom_headers(headers: &mut HeaderMap, custom: &[CustomHeader]) {
    for h in custom {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(h.header_name.as_bytes()),
            HeaderValue::from_str(&h.header_value),
        ) {
            headers.insert(name, value);
        }
    }
}

/// Add informational CloudFront headers to the downstream response.
pub fn add_cloudfront_response_headers(headers: &mut HeaderMap) {
    let id = uuid::Uuid::new_v4().simple().to_string();
    if let Ok(v) = HeaderValue::from_str(&id) {
        headers.insert(HeaderName::from_static("x-amz-cf-id"), v);
    }
    headers.insert(
        HeaderName::from_static("x-cache"),
        HeaderValue::from_static("Miss from rustack-cloudfront"),
    );
    headers.insert(
        HeaderName::from_static("via"),
        HeaderValue::from_static("1.1 rustack.cloudfront.net (CloudFront)"),
    );
}

/// Apply a response-headers policy to an outgoing response.
pub fn apply_response_headers_policy(
    headers: &mut HeaderMap,
    policy: &ResponseHeadersPolicyConfig,
    request_origin: Option<&HeaderValue>,
) {
    if let Some(cors) = &policy.cors_config {
        if request_origin.is_some() {
            if cors.access_control_allow_credentials {
                headers.insert(
                    HeaderName::from_static("access-control-allow-credentials"),
                    HeaderValue::from_static("true"),
                );
            }
            if !cors.access_control_allow_origins.is_empty() {
                let v = cors.access_control_allow_origins.join(",");
                if let Ok(hv) = HeaderValue::from_str(&v) {
                    headers.insert(HeaderName::from_static("access-control-allow-origin"), hv);
                }
            }
            if !cors.access_control_allow_headers.is_empty() {
                let v = cors.access_control_allow_headers.join(",");
                if let Ok(hv) = HeaderValue::from_str(&v) {
                    headers.insert(HeaderName::from_static("access-control-allow-headers"), hv);
                }
            }
            if !cors.access_control_allow_methods.is_empty() {
                let v = cors.access_control_allow_methods.join(",");
                if let Ok(hv) = HeaderValue::from_str(&v) {
                    headers.insert(HeaderName::from_static("access-control-allow-methods"), hv);
                }
            }
            if !cors.access_control_expose_headers.is_empty() {
                let v = cors.access_control_expose_headers.join(",");
                if let Ok(hv) = HeaderValue::from_str(&v) {
                    headers.insert(HeaderName::from_static("access-control-expose-headers"), hv);
                }
            }
            if cors.access_control_max_age_sec > 0 {
                if let Ok(hv) = HeaderValue::from_str(&cors.access_control_max_age_sec.to_string())
                {
                    headers.insert(HeaderName::from_static("access-control-max-age"), hv);
                }
            }
        }
    }
    if let Some(sec) = &policy.security_headers_config {
        if let Some(fo) = &sec.frame_options {
            if let Ok(hv) = HeaderValue::from_str(&fo.frame_option) {
                headers.insert(HeaderName::from_static("x-frame-options"), hv);
            }
        }
        if let Some(rp) = &sec.referrer_policy {
            if let Ok(hv) = HeaderValue::from_str(&rp.referrer_policy) {
                headers.insert(HeaderName::from_static("referrer-policy"), hv);
            }
        }
        if let Some(csp) = &sec.content_security_policy {
            if let Ok(hv) = HeaderValue::from_str(&csp.content_security_policy) {
                headers.insert(HeaderName::from_static("content-security-policy"), hv);
            }
        }
        if sec.content_type_options.is_some() {
            headers.insert(
                HeaderName::from_static("x-content-type-options"),
                HeaderValue::from_static("nosniff"),
            );
        }
        if let Some(sts) = &sec.strict_transport_security {
            let mut v = format!("max-age={}", sts.access_control_max_age_sec);
            if sts.include_subdomains {
                v.push_str("; includeSubDomains");
            }
            if sts.preload {
                v.push_str("; preload");
            }
            if let Ok(hv) = HeaderValue::from_str(&v) {
                headers.insert(HeaderName::from_static("strict-transport-security"), hv);
            }
        }
    }
    for hdr in &policy.custom_headers_config {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(hdr.header.as_bytes()),
            HeaderValue::from_str(&hdr.value),
        ) {
            if hdr.override_upstream || !headers.contains_key(&name) {
                headers.insert(name, value);
            }
        }
    }
    for h in &policy.remove_headers_config {
        if let Ok(name) = HeaderName::from_bytes(h.as_bytes()) {
            headers.remove(name);
        }
    }
}

/// If the distribution's default root object should be used, rewrite the path.
pub fn rewrite_path_with_default_root<'a>(
    config: &'a DistributionConfig,
    path: &'a str,
) -> Cow<'a, str> {
    apply_default_root_object(path, &config.default_root_object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_root_rewrite() {
        let out = apply_default_root_object("/", "index.html");
        assert_eq!(out, "/index.html");
        assert_eq!(apply_default_root_object("/foo", "index.html"), "/foo");
        assert_eq!(apply_default_root_object("/", ""), "/");
    }

    #[test]
    fn test_origin_path_concat() {
        assert_eq!(concat_origin_path("/v1", "/api"), "/v1/api");
        assert_eq!(concat_origin_path("/v1/", "/api"), "/v1/api");
        assert_eq!(concat_origin_path("", "/api"), "/api");
        assert_eq!(concat_origin_path("/v1", "api"), "/v1/api");
    }
}
