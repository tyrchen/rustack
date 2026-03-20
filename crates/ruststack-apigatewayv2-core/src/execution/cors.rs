//! CORS handling for API Gateway v2.
//!
//! Constructs CORS preflight responses and adds CORS headers to responses
//! based on the API's CORS configuration.

use bytes::Bytes;
use ruststack_apigatewayv2_model::types::Cors;

/// Build a CORS preflight (OPTIONS) response.
#[must_use]
pub fn build_cors_preflight_response(cors: &Cors) -> http::Response<Bytes> {
    let mut builder = http::Response::builder().status(204);

    if !cors.allow_origins.is_empty() {
        builder = builder.header("access-control-allow-origin", cors.allow_origins.join(", "));
    }

    if !cors.allow_methods.is_empty() {
        builder = builder.header(
            "access-control-allow-methods",
            cors.allow_methods.join(", "),
        );
    }

    if !cors.allow_headers.is_empty() {
        builder = builder.header(
            "access-control-allow-headers",
            cors.allow_headers.join(", "),
        );
    }

    if !cors.expose_headers.is_empty() {
        builder = builder.header(
            "access-control-expose-headers",
            cors.expose_headers.join(", "),
        );
    }

    if let Some(max_age) = cors.max_age {
        builder = builder.header("access-control-max-age", max_age.to_string());
    }

    if cors.allow_credentials == Some(true) {
        builder = builder.header("access-control-allow-credentials", "true");
    }

    builder
        .body(Bytes::new())
        .unwrap_or_else(|_| http::Response::new(Bytes::new()))
}

/// Add CORS headers to an existing response based on the CORS configuration.
pub fn add_cors_headers(response: &mut http::Response<Bytes>, cors: &Cors) {
    let headers = response.headers_mut();

    if !cors.allow_origins.is_empty() {
        if let Ok(v) = http::HeaderValue::from_str(&cors.allow_origins.join(", ")) {
            headers.insert("access-control-allow-origin", v);
        }
    }

    if !cors.expose_headers.is_empty() {
        if let Ok(v) = http::HeaderValue::from_str(&cors.expose_headers.join(", ")) {
            headers.insert("access-control-expose-headers", v);
        }
    }

    if cors.allow_credentials == Some(true) {
        headers.insert(
            "access-control-allow-credentials",
            http::HeaderValue::from_static("true"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_build_cors_preflight_with_all_headers() {
        let cors = Cors {
            allow_origins: vec!["*".to_owned()],
            allow_methods: vec!["GET".to_owned(), "POST".to_owned()],
            allow_headers: vec!["Content-Type".to_owned()],
            expose_headers: vec!["X-Custom".to_owned()],
            max_age: Some(3600),
            allow_credentials: Some(true),
        };
        let resp = build_cors_preflight_response(&cors);
        assert_eq!(resp.status(), 204);
        assert_eq!(
            resp.headers()
                .get("access-control-allow-origin")
                .expect("has origin")
                .to_str()
                .expect("str"),
            "*"
        );
        assert_eq!(
            resp.headers()
                .get("access-control-allow-methods")
                .expect("has methods")
                .to_str()
                .expect("str"),
            "GET, POST"
        );
    }
}
