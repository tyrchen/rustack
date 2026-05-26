//! HTTP response helpers.

use bytes::Bytes;
use http::{HeaderValue, Response, StatusCode};
use rustack_cloudfront_model::CloudFrontError;

use crate::{service::HttpBody, xml::ser::error_xml};

/// Turn a `CloudFrontError` into an HTTP response.
pub fn error_response(err: &CloudFrontError, request_id: &str) -> Response<HttpBody> {
    let status =
        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let xml = error_xml(err.code(), &err.message(), request_id);
    let mut resp = Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "text/xml")
        .body(HttpBody::from(xml))
        .unwrap_or_else(|_| Response::new(HttpBody::from(String::new())));
    if let Ok(hv) = HeaderValue::from_str(err.code()) {
        resp.headers_mut().insert("x-amzn-errortype", hv);
    }
    if let Ok(hv) = HeaderValue::from_str(request_id) {
        resp.headers_mut().insert("x-amzn-requestid", hv);
    }
    resp
}

/// Build an XML response with `ETag`, `Content-Type`, and optional `Location` headers.
pub fn xml_response(status: StatusCode, body: String, etag: Option<&str>) -> Response<HttpBody> {
    let mut builder = Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/xml");
    if let Some(tag) = etag {
        builder = builder.header(http::header::ETAG, tag);
    }
    builder
        .body(HttpBody::from(body))
        .unwrap_or_else(|_| Response::new(HttpBody::from(String::new())))
}

/// Build a byte response with `ETag` and `Content-Type` headers.
pub fn bytes_response(
    status: StatusCode,
    body: Bytes,
    content_type: &'static str,
    etag: Option<&str>,
) -> Response<HttpBody> {
    let mut builder = Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, content_type);
    if let Some(tag) = etag {
        builder = builder.header(http::header::ETAG, tag);
    }
    builder
        .body(HttpBody::from(body))
        .unwrap_or_else(|_| Response::new(HttpBody::default()))
}

/// Empty 204 response.
pub fn empty_204() -> Response<HttpBody> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(HttpBody::default())
        .unwrap_or_else(|_| Response::new(HttpBody::default()))
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http_body_util::BodyExt;

    use super::*;

    #[tokio::test]
    async fn test_should_build_bytes_response_with_etag() {
        let response = bytes_response(
            StatusCode::OK,
            Bytes::from_static(b"function code"),
            "application/octet-stream",
            Some("etag-1"),
        );

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(http::header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/octet-stream")),
        );
        assert_eq!(
            response.headers().get(http::header::ETAG),
            Some(&HeaderValue::from_static("etag-1")),
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body, Bytes::from_static(b"function code"));
    }
}
