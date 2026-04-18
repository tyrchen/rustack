//! Integration tests for the CloudFront **data plane**.
//!
//! These tests require a running Rustack server at `localhost:4566` with
//! CloudFront and S3 enabled. They create real S3 buckets, upload objects,
//! configure distributions, and then fetch content through the CloudFront
//! pass-through data plane.
//!
//! All tests are `#[ignore]` by default so they don't run in `cargo test`.
//! Run with:
//!
//! ```text
//! SERVICES=cloudfront,s3 rustack &
//! cargo test -p rustack-integration cloudfront_dataplane -- --ignored --test-threads=1
//! ```

use reqwest::StatusCode;

use crate::{
    cloudfront_helpers::{
        base_url, cf_delete, cf_get, cf_http_client, cf_post, cf_put, create_distribution,
        distribution_xml, distribution_xml_with_cache_behaviors,
        distribution_xml_with_custom_errors, distribution_xml_with_lambda_edge,
        distribution_xml_with_origin_path, extract_tag, new_caller_ref, response_etag,
        response_header,
    },
    create_test_bucket, s3_client,
};

/// Upload `content` to `bucket/key` with the given `content-type`.
async fn put_s3_object(bucket: &str, key: &str, content: &[u8], content_type: &str) {
    let client = s3_client();
    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .content_type(content_type)
        .body(content.to_vec().into())
        .send()
        .await
        .expect("put_object");
}

/// Build the `{bucket}.s3.us-east-1.amazonaws.com` virtual-hosted S3 origin
/// domain expected by CloudFront.
fn origin_domain(bucket: &str) -> String {
    format!("{bucket}.s3.us-east-1.amazonaws.com")
}

// ---------------------------------------------------------------------------
// Path-based routing
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_serve_s3_object_via_path_based_url() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp").await;
    put_s3_object(
        &bucket,
        "index.html",
        b"<h1>hello from cf</h1>",
        "text/html",
    )
    .await;

    let client = cf_http_client();
    let body = distribution_xml(&new_caller_ref("cfdp-serve"), &origin_domain(&bucket), true);
    let (id, etag) = create_distribution(&client, &body).await;

    let resp = client
        .get(format!("{}/_aws/cloudfront/{id}/index.html", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        response_header(&resp, "content-type").as_deref(),
        Some("text/html")
    );
    // CloudFront informational headers must be present on every response.
    assert!(response_header(&resp, "x-amz-cf-id").is_some());
    assert_eq!(
        response_header(&resp, "x-cache").as_deref(),
        Some("Miss from rustack-cloudfront")
    );
    assert!(
        response_header(&resp, "via")
            .unwrap_or_default()
            .contains("CloudFront")
    );
    assert_eq!(resp.text().await.unwrap(), "<h1>hello from cf</h1>");

    // Cleanup — distribution is enabled, must disable first.
    disable_and_clean(&client, &id, &etag).await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_apply_default_root_object_rewrite() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-root").await;
    put_s3_object(&bucket, "index.html", b"default-root-hit", "text/plain").await;

    let client = cf_http_client();
    let body = distribution_xml(&new_caller_ref("cfdp-root"), &origin_domain(&bucket), true);
    let (id, etag) = create_distribution(&client, &body).await;

    // Request "/" — should rewrite to "/index.html".
    let resp = client
        .get(format!("{}/_aws/cloudfront/{id}/", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "default-root-hit");

    disable_and_clean(&client, &id, &etag).await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_apply_origin_path_prefix() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-opath").await;
    // The bucket has `v1/index.html`; the distribution's OriginPath=/v1,
    // so a request for `/index.html` should resolve to `v1/index.html`.
    put_s3_object(&bucket, "v1/index.html", b"from-v1", "text/plain").await;

    let client = cf_http_client();
    let body = distribution_xml_with_origin_path(
        &new_caller_ref("cfdp-opath"),
        &origin_domain(&bucket),
        "/v1",
        "index.html",
        true,
    );
    let (id, etag) = create_distribution(&client, &body).await;

    let resp = client
        .get(format!("{}/_aws/cloudfront/{id}/index.html", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "from-v1");

    disable_and_clean(&client, &id, &etag).await;
}

// ---------------------------------------------------------------------------
// Error paths
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_return_404_for_unknown_distribution() {
    let client = cf_http_client();
    let resp = client
        .get(format!("{}/_aws/cloudfront/EDOESNOTEXIST/foo", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_header(&resp, "x-amzn-errortype").as_deref(),
        Some("NoSuchDistribution")
    );
    // Pretty error envelope even on failure.
    assert!(response_header(&resp, "x-amz-cf-id").is_some());
    let body = resp.text().await.unwrap();
    assert!(body.contains("NoSuchDistribution"));
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_return_403_for_disabled_distribution() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-disabled").await;

    let client = cf_http_client();
    let body = distribution_xml(
        &new_caller_ref("cfdp-dis"),
        &origin_domain(&bucket),
        false, // <-- disabled from the start
    );
    let (id, etag) = create_distribution(&client, &body).await;

    let resp = client
        .get(format!("{}/_aws/cloudfront/{id}/anything", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_header(&resp, "x-amzn-errortype").as_deref(),
        Some("DistributionDisabled")
    );

    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_return_404_for_missing_origin_object() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-404").await;

    let client = cf_http_client();
    let body = distribution_xml(&new_caller_ref("cfdp-404"), &origin_domain(&bucket), true);
    let (id, etag) = create_distribution(&client, &body).await;

    let resp = client
        .get(format!(
            "{}/_aws/cloudfront/{id}/definitely-not-there.txt",
            base_url()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_header(&resp, "x-amzn-errortype").as_deref(),
        Some("NoSuchKey")
    );

    disable_and_clean(&client, &id, &etag).await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_return_405_for_disallowed_method() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-405").await;
    put_s3_object(&bucket, "x", b"x", "text/plain").await;

    let client = cf_http_client();
    let body = distribution_xml(&new_caller_ref("cfdp-405"), &origin_domain(&bucket), true);
    let (id, etag) = create_distribution(&client, &body).await;

    // Default behavior's AllowedMethods is GET + HEAD; a POST should 405.
    let resp = client
        .post(format!("{}/_aws/cloudfront/{id}/x", base_url()))
        .body("whatever")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

    disable_and_clean(&client, &id, &etag).await;
}

// ---------------------------------------------------------------------------
// Cache-behaviour matching
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_match_path_pattern_cache_behavior() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-cb").await;
    put_s3_object(&bucket, "photo.jpg", b"jpeg-bytes", "image/jpeg").await;
    put_s3_object(&bucket, "other.txt", b"text-bytes", "text/plain").await;

    let client = cf_http_client();
    let body = distribution_xml_with_cache_behaviors(
        &new_caller_ref("cfdp-cb"),
        &origin_domain(&bucket),
        true,
        "", // no RHP
    );
    let (id, etag) = create_distribution(&client, &body).await;

    // *.jpg goes through the secondary behaviour (AllowedMethods = GET only).
    // Both paths should 200; this just verifies matching doesn't break.
    let jpg = client
        .get(format!("{}/_aws/cloudfront/{id}/photo.jpg", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(jpg.status(), StatusCode::OK);
    assert_eq!(jpg.text().await.unwrap(), "jpeg-bytes");

    let txt = client
        .get(format!("{}/_aws/cloudfront/{id}/other.txt", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(txt.status(), StatusCode::OK);

    // HEAD on .jpg behaviour (which allows only GET) should 405.
    let head = client
        .head(format!("{}/_aws/cloudfront/{id}/photo.jpg", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), StatusCode::METHOD_NOT_ALLOWED);

    disable_and_clean(&client, &id, &etag).await;
}

// ---------------------------------------------------------------------------
// Response-headers policy application
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_apply_response_headers_policy() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-rhp").await;
    put_s3_object(&bucket, "x", b"x", "text/plain").await;

    let client = cf_http_client();

    // Create a response-headers policy with a custom header.
    let rhp_name = format!("rhp-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let rhp_body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ResponseHeadersPolicyConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Comment>apply-test</Comment>
  <Name>{rhp_name}</Name>
</ResponseHeadersPolicyConfig>"#
    );
    let rhp_resp = cf_post(&client, "/2020-05-31/response-headers-policy", &rhp_body).await;
    assert_eq!(rhp_resp.status(), StatusCode::CREATED);
    let rhp_etag = response_etag(&rhp_resp).unwrap();
    let rhp_id = extract_tag(&rhp_resp.text().await.unwrap(), "Id").unwrap();

    // Distribution references the RHP.
    let dist_body = distribution_xml_with_cache_behaviors(
        &new_caller_ref("cfdp-rhp"),
        &origin_domain(&bucket),
        true,
        &rhp_id,
    );
    let (id, etag) = create_distribution(&client, &dist_body).await;

    let resp = client
        .get(format!("{}/_aws/cloudfront/{id}/x", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // Even a minimal RHP applies server-timing / CloudFront defaults to the
    // response. We at least verify the CF response headers still stream.
    assert!(response_header(&resp, "x-amz-cf-id").is_some());

    disable_and_clean(&client, &id, &etag).await;
    cf_delete(
        &client,
        &format!("/2020-05-31/response-headers-policy/{rhp_id}"),
        Some(&rhp_etag),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Custom error responses
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_serve_custom_error_response() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-cer").await;

    let client = cf_http_client();
    let body = distribution_xml_with_custom_errors(
        &new_caller_ref("cfdp-cer"),
        &origin_domain(&bucket),
        true,
    );
    let (id, etag) = create_distribution(&client, &body).await;

    // Request a missing object — CustomErrorResponses maps 404 → 200 + /not-found.html.
    let resp = client
        .get(format!("{}/_aws/cloudfront/{id}/missing.html", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // CloudFront sets a Cache-Control max-age from ErrorCachingMinTTL.
    let cc = response_header(&resp, "cache-control").unwrap_or_default();
    assert!(cc.contains("max-age=10"), "expected max-age=10, got {cc}");

    disable_and_clean(&client, &id, &etag).await;
}

// ---------------------------------------------------------------------------
// Divergence signalling
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_serve_with_warning_on_lambda_edge_association() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-le").await;
    put_s3_object(&bucket, "x", b"served", "text/plain").await;

    let client = cf_http_client();
    let body = distribution_xml_with_lambda_edge(
        &new_caller_ref("cfdp-le"),
        &origin_domain(&bucket),
        true,
    );
    let (id, etag) = create_distribution(&client, &body).await;

    // Request succeeds: Lambda@Edge association only emits a warn by default.
    let resp = client
        .get(format!("{}/_aws/cloudfront/{id}/x", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "served");

    disable_and_clean(&client, &id, &etag).await;
}

// ---------------------------------------------------------------------------
// HEAD + OriginKind classification
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_support_head_method() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-head").await;
    put_s3_object(&bucket, "sz", b"0123456789", "application/octet-stream").await;

    let client = cf_http_client();
    let body = distribution_xml(&new_caller_ref("cfdp-head"), &origin_domain(&bucket), true);
    let (id, etag) = create_distribution(&client, &body).await;

    let resp = client
        .head(format!("{}/_aws/cloudfront/{id}/sz", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(response_header(&resp, "etag").is_some());
    // No body on HEAD.
    assert!(resp.text().await.unwrap().is_empty());

    disable_and_clean(&client, &id, &etag).await;
}

// ---------------------------------------------------------------------------
// Host-based routing
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_dp_should_route_by_host_header_cloudfront_net() {
    let s3 = s3_client();
    let bucket = create_test_bucket(&s3, "cfdp-host").await;
    put_s3_object(&bucket, "h", b"host-routed", "text/plain").await;

    let client = cf_http_client();
    let body = distribution_xml(&new_caller_ref("cfdp-host"), &origin_domain(&bucket), true);
    let (id, etag) = create_distribution(&client, &body).await;

    let lowercased = id.to_lowercase();
    let host = format!("{lowercased}.cloudfront.net");

    let resp = client
        .get(format!("{}/h", base_url()))
        .header("Host", &host)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "host-routed");

    disable_and_clean(&client, &id, &etag).await;
}

// ---------------------------------------------------------------------------
// Common helper: disable then delete a distribution.
// ---------------------------------------------------------------------------

async fn disable_and_clean(client: &reqwest::Client, id: &str, etag_hint: &str) {
    // Read current config + etag.
    let got = cf_get(client, &format!("/2020-05-31/distribution/{id}/config")).await;
    let etag = response_etag(&got).unwrap_or_else(|| etag_hint.to_owned());
    let cfg = got
        .text()
        .await
        .unwrap()
        .replace("<Enabled>true</Enabled>", "<Enabled>false</Enabled>");
    let updated = cf_put(
        client,
        &format!("/2020-05-31/distribution/{id}/config"),
        &cfg,
        Some(&etag),
    )
    .await;
    let new_etag = response_etag(&updated).unwrap();
    cf_delete(
        client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&new_etag),
    )
    .await;
}
