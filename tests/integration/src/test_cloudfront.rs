//! Integration tests for the CloudFront **management plane**.
//!
//! These tests require a running Rustack server at `localhost:4566` with
//! CloudFront enabled. They are marked `#[ignore = "requires running Rustack
//! server"]` so they don't run during normal `cargo test`. Run them with:
//!
//! ```text
//! SERVICES=cloudfront,s3 rustack &
//! cargo test -p rustack-integration cloudfront -- --ignored --test-threads=1
//! ```
//!
//! Tests exercise the restXml 2020-05-31 wire format directly via `reqwest`
//! rather than `aws-sdk-cloudfront`, which spares us its rustls bootstrap and
//! lets us assert on headers and error bodies without SDK smoothing.

use reqwest::StatusCode;

use crate::cloudfront_helpers::{
    cf_delete, cf_get, cf_http_client, cf_post, cf_put, create_distribution,
    disable_and_delete_distribution, distribution_xml, extract_distribution_id, extract_tag,
    new_caller_ref, response_etag, response_header,
};

// ---------------------------------------------------------------------------
// Phase 0: Distribution
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_create_get_list_delete_distribution() {
    let client = cf_http_client();
    let caller_ref = new_caller_ref("cf-crud");
    let body = distribution_xml(&caller_ref, "test-bucket.s3.us-east-1.amazonaws.com", false);

    // Create
    let resp = cf_post(&client, "/2020-05-31/distribution", &body).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert!(response_etag(&resp).is_some());
    assert!(response_header(&resp, "location").is_some());
    let xml = resp.text().await.unwrap();
    let id = extract_distribution_id(&xml).unwrap();

    // Get
    let got = cf_get(&client, &format!("/2020-05-31/distribution/{id}")).await;
    assert_eq!(got.status(), StatusCode::OK);
    let etag = response_etag(&got).unwrap();
    let body = got.text().await.unwrap();
    assert!(body.contains(&id), "get body contains distribution id");
    assert!(body.contains("<Status>"), "get body has Status tag");

    // GetConfig
    let got_cfg = cf_get(&client, &format!("/2020-05-31/distribution/{id}/config")).await;
    assert_eq!(got_cfg.status(), StatusCode::OK);
    assert!(response_etag(&got_cfg).is_some());
    assert!(
        got_cfg
            .text()
            .await
            .unwrap()
            .contains("<DistributionConfig")
    );

    // List
    let listed = cf_get(&client, "/2020-05-31/distribution").await;
    assert_eq!(listed.status(), StatusCode::OK);
    let list_body = listed.text().await.unwrap();
    assert!(
        list_body.contains(&id),
        "list should contain our distribution"
    );
    assert!(list_body.contains("<DistributionSummary>"));

    // Delete (distribution is disabled — no disable dance needed)
    let del = cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag),
    )
    .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_reject_delete_without_if_match() {
    let client = cf_http_client();
    let body = distribution_xml(
        &new_caller_ref("cf-no-ifm"),
        "test-bucket.s3.us-east-1.amazonaws.com",
        false,
    );
    let (id, etag) = create_distribution(&client, &body).await;

    let resp = cf_delete(&client, &format!("/2020-05-31/distribution/{id}"), None).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err_body = resp.text().await.unwrap();
    assert!(
        err_body.contains("InvalidIfMatchVersion"),
        "expected InvalidIfMatchVersion: {err_body}"
    );

    // Cleanup
    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_reject_delete_with_stale_if_match() {
    let client = cf_http_client();
    let body = distribution_xml(
        &new_caller_ref("cf-stale-ifm"),
        "test-bucket.s3.us-east-1.amazonaws.com",
        false,
    );
    let (id, etag) = create_distribution(&client, &body).await;

    // Stale ETag → 412.
    let resp = cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some("EWRONGETAG"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    assert!(resp.text().await.unwrap().contains("PreconditionFailed"));

    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_reject_delete_of_enabled_distribution() {
    let client = cf_http_client();
    let caller_ref = new_caller_ref("cf-enabled");
    let body = distribution_xml(&caller_ref, "test-bucket.s3.us-east-1.amazonaws.com", true);

    let (id, etag) = create_distribution(&client, &body).await;

    let resp = cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert!(
        resp.text()
            .await
            .unwrap()
            .contains("DistributionNotDisabled")
    );

    // Clean up via disable-then-delete path.
    disable_and_delete_distribution(&client, &id, &etag, true).await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_update_distribution_and_bump_etag() {
    let client = cf_http_client();
    let caller_ref = new_caller_ref("cf-update");
    let body = distribution_xml(&caller_ref, "test-bucket.s3.us-east-1.amazonaws.com", false);
    let (id, etag1) = create_distribution(&client, &body).await;

    // Change comment and push an update.
    let updated = body.replace(
        "<Comment>integration-test</Comment>",
        "<Comment>integration-test-updated</Comment>",
    );
    let resp = cf_put(
        &client,
        &format!("/2020-05-31/distribution/{id}/config"),
        &updated,
        Some(&etag1),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let etag2 = response_etag(&resp).unwrap();
    assert_ne!(etag1, etag2, "ETag must rotate on update");
    let body2 = resp.text().await.unwrap();
    assert!(body2.contains("integration-test-updated"));

    // Second update with the old ETag should 412.
    let stale = cf_put(
        &client,
        &format!("/2020-05-31/distribution/{id}/config"),
        &updated,
        Some(&etag1),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);

    // Cleanup
    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag2),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_return_404_for_missing_distribution() {
    let client = cf_http_client();
    let resp = cf_get(&client, "/2020-05-31/distribution/EZZZZZZZZZZZZZ").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = resp.text().await.unwrap();
    assert!(body.contains("NoSuchDistribution"));
    assert!(body.contains("<ErrorResponse"));
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_reject_create_without_caller_reference() {
    let client = cf_http_client();
    // Omit CallerReference from an otherwise-valid body.
    let body = distribution_xml("", "test-bucket.s3.us-east-1.amazonaws.com", false)
        .replace("<CallerReference></CallerReference>", "");
    let resp = cf_post(&client, "/2020-05-31/distribution", &body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let t = resp.text().await.unwrap();
    assert!(t.contains("MissingArgument") || t.contains("CallerReference"));
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_reject_create_with_unknown_target_origin_id() {
    let client = cf_http_client();
    // Swap the TargetOriginId to something not in Origins.
    let body = distribution_xml(
        &new_caller_ref("cf-bad-target"),
        "test-bucket.s3.us-east-1.amazonaws.com",
        false,
    )
    .replace(
        "<TargetOriginId>primary</TargetOriginId>",
        "<TargetOriginId>unknown</TargetOriginId>",
    );
    let resp = cf_post(&client, "/2020-05-31/distribution", &body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(resp.text().await.unwrap().contains("does not match"));
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_copy_distribution() {
    let client = cf_http_client();
    let caller_ref = new_caller_ref("cf-src");
    let body = distribution_xml(&caller_ref, "test-bucket.s3.us-east-1.amazonaws.com", false);
    let (src_id, src_etag) = create_distribution(&client, &body).await;

    let copy_body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CopyDistributionRequest xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>{}</CallerReference>
  <Staging>false</Staging>
</CopyDistributionRequest>"#,
        new_caller_ref("cf-copy")
    );
    let resp = cf_post(
        &client,
        &format!("/2020-05-31/distribution/{src_id}/copy"),
        &copy_body,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let xml = resp.text().await.unwrap();
    let copy_id = extract_distribution_id(&xml).unwrap();
    assert_ne!(copy_id, src_id);

    // Cleanup — copy is disabled by default (copied from src which was disabled).
    let copy_etag_resp = cf_get(&client, &format!("/2020-05-31/distribution/{copy_id}")).await;
    let copy_etag = response_etag(&copy_etag_resp).unwrap();
    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{copy_id}"),
        Some(&copy_etag),
    )
    .await;
    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{src_id}"),
        Some(&src_etag),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Phase 0: Invalidation
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_create_get_list_invalidation() {
    let client = cf_http_client();
    let caller_ref = new_caller_ref("cf-inv-host");
    let body = distribution_xml(&caller_ref, "test-bucket.s3.us-east-1.amazonaws.com", false);
    let (id, etag) = create_distribution(&client, &body).await;

    let inv_body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<InvalidationBatch xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Paths>
    <Quantity>2</Quantity>
    <Items><Path>/a</Path><Path>/b/*</Path></Items>
  </Paths>
  <CallerReference>{}</CallerReference>
</InvalidationBatch>"#,
        new_caller_ref("inv")
    );
    let created = cf_post(
        &client,
        &format!("/2020-05-31/distribution/{id}/invalidation"),
        &inv_body,
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let inv_xml = created.text().await.unwrap();
    let inv_id = extract_tag(&inv_xml, "Id").unwrap();
    assert!(inv_id.starts_with('I') && inv_id.len() == 14);
    assert!(inv_xml.contains("<Path>/a</Path>"));
    assert!(inv_xml.contains("<Path>/b/*</Path>"));

    // Get.
    let got = cf_get(
        &client,
        &format!("/2020-05-31/distribution/{id}/invalidation/{inv_id}"),
    )
    .await;
    assert_eq!(got.status(), StatusCode::OK);

    // List.
    let listed = cf_get(
        &client,
        &format!("/2020-05-31/distribution/{id}/invalidation"),
    )
    .await;
    assert_eq!(listed.status(), StatusCode::OK);
    let listed_body = listed.text().await.unwrap();
    assert!(listed_body.contains(&inv_id));

    // Cleanup.
    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_reject_invalidation_without_paths() {
    let client = cf_http_client();
    let body = distribution_xml(
        &new_caller_ref("cf-empty-paths"),
        "test-bucket.s3.us-east-1.amazonaws.com",
        false,
    );
    let (id, etag) = create_distribution(&client, &body).await;

    let inv_body = r#"<?xml version="1.0" encoding="UTF-8"?>
<InvalidationBatch xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Paths><Quantity>0</Quantity></Paths>
  <CallerReference>empty</CallerReference>
</InvalidationBatch>"#;
    let resp = cf_post(
        &client,
        &format!("/2020-05-31/distribution/{id}/invalidation"),
        inv_body,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Phase 0: Origin Access Control
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_crud_origin_access_control() {
    let client = cf_http_client();
    let name = format!("oac-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<OriginAccessControlConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Name>{name}</Name>
  <Description>test oac</Description>
  <SigningProtocol>sigv4</SigningProtocol>
  <SigningBehavior>always</SigningBehavior>
  <OriginAccessControlOriginType>s3</OriginAccessControlOriginType>
</OriginAccessControlConfig>"#
    );
    let created = cf_post(&client, "/2020-05-31/origin-access-control", &body).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    assert!(response_header(&created, "location").is_some());
    let etag1 = response_etag(&created).unwrap();
    let xml = created.text().await.unwrap();
    let id = extract_tag(&xml, "Id").unwrap();
    assert!(id.starts_with('E'));

    // Get.
    let got = cf_get(&client, &format!("/2020-05-31/origin-access-control/{id}")).await;
    assert_eq!(got.status(), StatusCode::OK);
    assert!(got.text().await.unwrap().contains(&name));

    // GetConfig.
    let got_cfg = cf_get(
        &client,
        &format!("/2020-05-31/origin-access-control/{id}/config"),
    )
    .await;
    assert_eq!(got_cfg.status(), StatusCode::OK);

    // Update description.
    let updated_cfg = body.replace(
        "<Description>test oac</Description>",
        "<Description>renamed</Description>",
    );
    let updated = cf_put(
        &client,
        &format!("/2020-05-31/origin-access-control/{id}/config"),
        &updated_cfg,
        Some(&etag1),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let etag2 = response_etag(&updated).unwrap();
    assert_ne!(etag1, etag2);

    // List.
    let listed = cf_get(&client, "/2020-05-31/origin-access-control").await;
    assert_eq!(listed.status(), StatusCode::OK);
    assert!(listed.text().await.unwrap().contains(&id));

    // Delete.
    let del = cf_delete(
        &client,
        &format!("/2020-05-31/origin-access-control/{id}"),
        Some(&etag2),
    )
    .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_reject_oac_without_name() {
    let client = cf_http_client();
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<OriginAccessControlConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Name></Name>
  <SigningProtocol>sigv4</SigningProtocol>
  <SigningBehavior>always</SigningBehavior>
  <OriginAccessControlOriginType>s3</OriginAccessControlOriginType>
</OriginAccessControlConfig>"#;
    let resp = cf_post(&client, "/2020-05-31/origin-access-control", body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Phase 0: Tagging
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_tag_untag_list() {
    let client = cf_http_client();
    let body = distribution_xml(
        &new_caller_ref("cf-tags"),
        "test-bucket.s3.us-east-1.amazonaws.com",
        false,
    );
    let (id, etag) = create_distribution(&client, &body).await;
    let arn = format!("arn:aws:cloudfront::000000000000:distribution/{id}");
    let arn_enc = urlencoding::encode(&arn);

    // Tag.
    let tag_body = r#"<?xml version="1.0" encoding="UTF-8"?>
<Tags xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <TagSet>
    <Tag><Key>env</Key><Value>dev</Value></Tag>
    <Tag><Key>team</Key><Value>platform</Value></Tag>
  </TagSet>
</Tags>"#;
    let tagged = cf_post(
        &client,
        &format!("/2020-05-31/tagging?Operation=Tag&Resource={arn_enc}"),
        tag_body,
    )
    .await;
    assert_eq!(tagged.status(), StatusCode::NO_CONTENT);

    // List.
    let listed = cf_get(&client, &format!("/2020-05-31/tagging?Resource={arn_enc}")).await;
    assert_eq!(listed.status(), StatusCode::OK);
    let xml = listed.text().await.unwrap();
    assert!(xml.contains("<Key>env</Key>"));
    assert!(xml.contains("<Value>dev</Value>"));
    assert!(xml.contains("<Key>team</Key>"));

    // Untag the env tag.
    let untag_body = r#"<?xml version="1.0" encoding="UTF-8"?>
<TagKeys xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Key>env</Key>
</TagKeys>"#;
    let untagged = cf_post(
        &client,
        &format!("/2020-05-31/tagging?Operation=Untag&Resource={arn_enc}"),
        untag_body,
    )
    .await;
    assert_eq!(untagged.status(), StatusCode::NO_CONTENT);

    let listed2 = cf_get(&client, &format!("/2020-05-31/tagging?Resource={arn_enc}")).await;
    let xml2 = listed2.text().await.unwrap();
    assert!(!xml2.contains("<Key>env</Key>"));
    assert!(xml2.contains("<Key>team</Key>"));

    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Phase 1: OAI + managed policies
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_crud_cloudfront_origin_access_identity() {
    let client = cf_http_client();
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<CloudFrontOriginAccessIdentityConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>oai-itest</CallerReference>
  <Comment>integration-oai</Comment>
</CloudFrontOriginAccessIdentityConfig>"#;
    let created = cf_post(
        &client,
        "/2020-05-31/origin-access-identity/cloudfront",
        body,
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let etag = response_etag(&created).unwrap();
    let xml = created.text().await.unwrap();
    let id = extract_tag(&xml, "Id").unwrap();
    assert!(xml.contains("<S3CanonicalUserId>"));

    let got = cf_get(
        &client,
        &format!("/2020-05-31/origin-access-identity/cloudfront/{id}"),
    )
    .await;
    assert_eq!(got.status(), StatusCode::OK);

    let del = cf_delete(
        &client,
        &format!("/2020-05-31/origin-access-identity/cloudfront/{id}"),
        Some(&etag),
    )
    .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_seed_managed_policies() {
    let client = cf_http_client();

    let cache = cf_get(&client, "/2020-05-31/cache-policy").await;
    assert_eq!(cache.status(), StatusCode::OK);
    let cache_body = cache.text().await.unwrap();
    assert!(cache_body.contains("658327ea-f89d-4fab-a63d-7e88639e58f6")); // CachingOptimized
    assert!(cache_body.contains("4135ea2d-6df8-44a3-9df3-4b5a84be39ad")); // CachingDisabled

    let orp = cf_get(&client, "/2020-05-31/origin-request-policy").await;
    assert_eq!(orp.status(), StatusCode::OK);
    let orp_body = orp.text().await.unwrap();
    assert!(orp_body.contains("216adef6-5c7f-47e4-b989-5492eafa07d3")); // Managed-AllViewer

    let rhp = cf_get(&client, "/2020-05-31/response-headers-policy").await;
    assert_eq!(rhp.status(), StatusCode::OK);
    let rhp_body = rhp.text().await.unwrap();
    assert!(rhp_body.contains("60669652-455b-4ae9-85a4-c4c02393f86c")); // Managed-SimpleCORS
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_reject_delete_of_managed_cache_policy() {
    let client = cf_http_client();
    // Fetch ETag first.
    let got = cf_get(
        &client,
        "/2020-05-31/cache-policy/658327ea-f89d-4fab-a63d-7e88639e58f6",
    )
    .await;
    assert_eq!(got.status(), StatusCode::OK);
    let etag = response_etag(&got).unwrap();
    let resp = cf_delete(
        &client,
        "/2020-05-31/cache-policy/658327ea-f89d-4fab-a63d-7e88639e58f6",
        Some(&etag),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(resp.text().await.unwrap().contains("AccessDenied"));
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_create_custom_cache_policy() {
    let client = cf_http_client();
    let name = format!("cp-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CachePolicyConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Comment>test</Comment>
  <Name>{name}</Name>
  <DefaultTTL>86400</DefaultTTL>
  <MaxTTL>3600000</MaxTTL>
  <MinTTL>1</MinTTL>
  <ParametersInCacheKeyAndForwardedToOrigin>
    <EnableAcceptEncodingGzip>true</EnableAcceptEncodingGzip>
    <EnableAcceptEncodingBrotli>true</EnableAcceptEncodingBrotli>
    <HeadersConfig>
      <HeaderBehavior>none</HeaderBehavior>
    </HeadersConfig>
    <CookiesConfig>
      <CookieBehavior>none</CookieBehavior>
    </CookiesConfig>
    <QueryStringsConfig>
      <QueryStringBehavior>none</QueryStringBehavior>
    </QueryStringsConfig>
  </ParametersInCacheKeyAndForwardedToOrigin>
</CachePolicyConfig>"#
    );
    let created = cf_post(&client, "/2020-05-31/cache-policy", &body).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let etag = response_etag(&created).unwrap();
    let xml = created.text().await.unwrap();
    let id = extract_tag(&xml, "Id").unwrap();

    // Delete — custom policy, should succeed.
    let del = cf_delete(
        &client,
        &format!("/2020-05-31/cache-policy/{id}"),
        Some(&etag),
    )
    .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}

// ---------------------------------------------------------------------------
// Phase 1: Response Headers Policy (used by dataplane tests too)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_create_response_headers_policy() {
    let client = cf_http_client();
    let name = format!("rhp-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ResponseHeadersPolicyConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Comment>test rhp</Comment>
  <Name>{name}</Name>
</ResponseHeadersPolicyConfig>"#
    );
    let resp = cf_post(&client, "/2020-05-31/response-headers-policy", &body).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let etag = response_etag(&resp).unwrap();
    let id = extract_tag(&resp.text().await.unwrap(), "Id").unwrap();
    let del = cf_delete(
        &client,
        &format!("/2020-05-31/response-headers-policy/{id}"),
        Some(&etag),
    )
    .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}

// ---------------------------------------------------------------------------
// Phase 2: Public key + Key group
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_crud_public_key_and_key_group() {
    let client = cf_http_client();
    let key_name = format!("pk-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let encoded = "-----BEGIN PUBLIC \
                   KEY-----\nMFwwDQYJKoZIhvcNAQEBBQADSwAwSAJBAK7VhyBXxMlObgJKQ8MhhIg=\n-----END \
                   PUBLIC KEY-----";
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PublicKeyConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>pk-ref</CallerReference>
  <Name>{key_name}</Name>
  <EncodedKey>{encoded}</EncodedKey>
  <Comment>itest public key</Comment>
</PublicKeyConfig>"#
    );
    let created = cf_post(&client, "/2020-05-31/public-key", &body).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let pk_etag = response_etag(&created).unwrap();
    let pk_id = extract_tag(&created.text().await.unwrap(), "Id").unwrap();

    // Make a KeyGroup referencing the PublicKey.
    let kg_name = format!("kg-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let kg_body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<KeyGroupConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Name>{kg_name}</Name>
  <Items>
    <PublicKey>{pk_id}</PublicKey>
  </Items>
  <Comment>itest</Comment>
</KeyGroupConfig>"#
    );
    let kg_created = cf_post(&client, "/2020-05-31/key-group", &kg_body).await;
    assert_eq!(kg_created.status(), StatusCode::CREATED);
    let kg_etag = response_etag(&kg_created).unwrap();
    let kg_id = extract_tag(&kg_created.text().await.unwrap(), "Id").unwrap();

    // Cleanup in reverse order.
    cf_delete(
        &client,
        &format!("/2020-05-31/key-group/{kg_id}"),
        Some(&kg_etag),
    )
    .await;
    cf_delete(
        &client,
        &format!("/2020-05-31/public-key/{pk_id}"),
        Some(&pk_etag),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_reject_public_key_without_encoded_key() {
    let client = cf_http_client();
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<PublicKeyConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>no-key</CallerReference>
  <Name>no-key</Name>
  <EncodedKey></EncodedKey>
</PublicKeyConfig>"#;
    let resp = cf_post(&client, "/2020-05-31/public-key", body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Phase 3: Function + KVS + Monitoring + RealtimeLog
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_create_publish_test_function() {
    let client = cf_http_client();
    let name = format!("fn-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    // Function code is passed verbatim (not base64) for this test — the
    // implementation accepts either and round-trips it.
    let code = "function handler(event) { return event.request; }";
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CreateFunctionRequest xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Name>{name}</Name>
  <FunctionConfig>
    <Comment>itest fn</Comment>
    <Runtime>cloudfront-js-1.0</Runtime>
  </FunctionConfig>
  <FunctionCode>{code}</FunctionCode>
</CreateFunctionRequest>"#
    );
    let created = cf_post(&client, "/2020-05-31/function", &body).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let etag = response_etag(&created).unwrap();

    // Publish (requires If-Match).
    let pub_resp = client
        .post(format!(
            "{}/2020-05-31/function/{name}/publish",
            crate::cloudfront_helpers::base_url()
        ))
        .header("If-Match", &etag)
        .body("")
        .send()
        .await
        .expect("POST publish");
    assert_eq!(pub_resp.status(), StatusCode::OK);
    let etag2 = response_etag(&pub_resp).unwrap();
    assert_ne!(etag, etag2);

    // Test.
    let test_body = r#"<?xml version="1.0" encoding="UTF-8"?>
<TestFunctionRequest xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <IfMatch></IfMatch>
  <EventObject>{"request":{"uri":"/"}}</EventObject>
</TestFunctionRequest>"#;
    let test_resp = cf_post(
        &client,
        &format!("/2020-05-31/function/{name}/test"),
        test_body,
    )
    .await;
    assert_eq!(test_resp.status(), StatusCode::OK);
    let body = test_resp.text().await.unwrap();
    assert!(body.contains("success") || body.contains("testStatus"));

    // Delete.
    let del = cf_delete(
        &client,
        &format!("/2020-05-31/function/{name}"),
        Some(&etag2),
    )
    .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_crud_key_value_store() {
    let client = cf_http_client();
    let name = format!("kvs-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<KeyValueStore xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Name>{name}</Name>
  <Comment>itest</Comment>
</KeyValueStore>"#
    );
    let created = cf_post(&client, "/2020-05-31/key-value-store", &body).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let etag = response_etag(&created).unwrap();
    let id = extract_tag(&created.text().await.unwrap(), "Id").unwrap();

    let del = cf_delete(
        &client,
        &format!("/2020-05-31/key-value-store/{id}"),
        Some(&etag),
    )
    .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_create_and_delete_monitoring_subscription() {
    let client = cf_http_client();
    let body = distribution_xml(
        &new_caller_ref("cf-mon"),
        "test-bucket.s3.us-east-1.amazonaws.com",
        false,
    );
    let (id, etag) = create_distribution(&client, &body).await;

    let mon_body = r#"<?xml version="1.0" encoding="UTF-8"?>
<MonitoringSubscription xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <RealtimeMetricsSubscriptionConfig>
    <RealtimeMetricsSubscriptionStatus>Enabled</RealtimeMetricsSubscriptionStatus>
  </RealtimeMetricsSubscriptionConfig>
</MonitoringSubscription>"#;
    let created = cf_post(
        &client,
        &format!("/2020-05-31/distributions/{id}/monitoring-subscription"),
        mon_body,
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);

    let got = cf_get(
        &client,
        &format!("/2020-05-31/distributions/{id}/monitoring-subscription"),
    )
    .await;
    assert_eq!(got.status(), StatusCode::OK);
    assert!(got.text().await.unwrap().contains("Enabled"));

    let del = cf_delete(
        &client,
        &format!("/2020-05-31/distributions/{id}/monitoring-subscription"),
        None,
    )
    .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    cf_delete(
        &client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&etag),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Phase 4 stubs (must return valid shapes, not 404)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Rustack server"]
async fn test_cf_should_return_empty_stubs_for_phase4_ops() {
    let client = cf_http_client();

    // List streaming distributions.
    let resp = cf_get(&client, "/2020-05-31/streaming-distribution").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let xml = resp.text().await.unwrap();
    assert!(xml.contains("<StreamingDistributionList"));
    assert!(xml.contains("<Quantity>0</Quantity>"));

    // ListDistributionsByCachePolicyId.
    let resp = cf_get(
        &client,
        "/2020-05-31/distributions-by-cache-policy-id/658327ea-f89d-4fab-a63d-7e88639e58f6",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.text().await.unwrap().contains("<DistributionIdList"));
}
