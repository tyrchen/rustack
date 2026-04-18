//! Shared helpers for CloudFront integration tests.
//!
//! CloudFront is restXml, so we exercise the wire format directly via
//! `reqwest` rather than `aws-sdk-cloudfront`. This keeps the tests
//! independent of SDK version churn and sidesteps the rustls TLS-connector
//! initialization path that the default smithy client forces even over plain
//! HTTP endpoints.

use std::time::Duration;

use reqwest::{Client, Response, StatusCode};

/// Base URL for the Rustack server under test.
#[must_use]
pub fn base_url() -> String {
    std::env::var("S3_ENDPOINT_URL").unwrap_or_else(|_| "http://localhost:4566".to_owned())
}

/// Build an HTTP client with generous timeouts.
#[must_use]
pub fn cf_http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("build reqwest client")
}

/// POST XML to a CloudFront management path.
pub async fn cf_post(client: &Client, path: &str, body: &str) -> Response {
    client
        .post(format!("{}{}", base_url(), path))
        .header("Content-Type", "application/xml")
        .body(body.to_owned())
        .send()
        .await
        .expect("POST CloudFront")
}

/// PUT XML to a CloudFront management path, optionally with `If-Match`.
pub async fn cf_put(client: &Client, path: &str, body: &str, if_match: Option<&str>) -> Response {
    let mut req = client
        .put(format!("{}{}", base_url(), path))
        .header("Content-Type", "application/xml")
        .body(body.to_owned());
    if let Some(etag) = if_match {
        req = req.header("If-Match", etag);
    }
    req.send().await.expect("PUT CloudFront")
}

/// GET a CloudFront management path.
pub async fn cf_get(client: &Client, path: &str) -> Response {
    client
        .get(format!("{}{}", base_url(), path))
        .send()
        .await
        .expect("GET CloudFront")
}

/// DELETE a CloudFront management path, optionally with `If-Match`.
pub async fn cf_delete(client: &Client, path: &str, if_match: Option<&str>) -> Response {
    let mut req = client.delete(format!("{}{}", base_url(), path));
    if let Some(etag) = if_match {
        req = req.header("If-Match", etag);
    }
    req.send().await.expect("DELETE CloudFront")
}

/// Extract the first occurrence of `<Tag>value</Tag>` from an XML document.
#[must_use]
pub fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].to_owned())
}

/// Extract a distribution ID (starts with `E`, 14 chars).
#[must_use]
pub fn extract_distribution_id(xml: &str) -> Option<String> {
    extract_tag(xml, "Id").filter(|id| id.len() == 14 && id.starts_with('E'))
}

/// Read the `ETag` response header.
#[must_use]
pub fn response_etag(resp: &Response) -> Option<String> {
    resp.headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Read an arbitrary response header as a string.
#[must_use]
pub fn response_header(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Minimal `DistributionConfig` body, parameterized by caller reference,
/// origin domain, and enabled flag.
#[must_use]
pub fn distribution_xml(caller_ref: &str, origin_domain: &str, enabled: bool) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DistributionConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>{caller_ref}</CallerReference>
  <Aliases><Quantity>0</Quantity></Aliases>
  <DefaultRootObject>index.html</DefaultRootObject>
  <Origins>
    <Quantity>1</Quantity>
    <Items>
      <Origin>
        <Id>primary</Id>
        <DomainName>{origin_domain}</DomainName>
        <OriginPath></OriginPath>
        <CustomHeaders><Quantity>0</Quantity></CustomHeaders>
        <S3OriginConfig><OriginAccessIdentity></OriginAccessIdentity></S3OriginConfig>
        <ConnectionAttempts>3</ConnectionAttempts>
        <ConnectionTimeout>10</ConnectionTimeout>
      </Origin>
    </Items>
  </Origins>
  <DefaultCacheBehavior>
    <TargetOriginId>primary</TargetOriginId>
    <TrustedSigners><Enabled>false</Enabled><Quantity>0</Quantity></TrustedSigners>
    <TrustedKeyGroups><Enabled>false</Enabled><Quantity>0</Quantity></TrustedKeyGroups>
    <ViewerProtocolPolicy>allow-all</ViewerProtocolPolicy>
    <AllowedMethods>
      <Quantity>2</Quantity>
      <Items><Method>GET</Method><Method>HEAD</Method></Items>
      <CachedMethods>
        <Quantity>2</Quantity>
        <Items><Method>GET</Method><Method>HEAD</Method></Items>
      </CachedMethods>
    </AllowedMethods>
    <SmoothStreaming>false</SmoothStreaming>
    <Compress>false</Compress>
    <CachePolicyId>658327ea-f89d-4fab-a63d-7e88639e58f6</CachePolicyId>
  </DefaultCacheBehavior>
  <CacheBehaviors><Quantity>0</Quantity></CacheBehaviors>
  <CustomErrorResponses><Quantity>0</Quantity></CustomErrorResponses>
  <Comment>integration-test</Comment>
  <PriceClass>PriceClass_All</PriceClass>
  <Enabled>{enabled}</Enabled>
  <ViewerCertificate><CloudFrontDefaultCertificate>true</CloudFrontDefaultCertificate></ViewerCertificate>
  <Restrictions><GeoRestriction><RestrictionType>none</RestrictionType><Quantity>0</Quantity></GeoRestriction></Restrictions>
  <HttpVersion>http2</HttpVersion>
  <IsIPV6Enabled>false</IsIPV6Enabled>
</DistributionConfig>"#
    )
}

/// DistributionConfig with `DefaultRootObject` overridden and `OriginPath` set.
#[must_use]
pub fn distribution_xml_with_origin_path(
    caller_ref: &str,
    origin_domain: &str,
    origin_path: &str,
    default_root: &str,
    enabled: bool,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DistributionConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>{caller_ref}</CallerReference>
  <Aliases><Quantity>0</Quantity></Aliases>
  <DefaultRootObject>{default_root}</DefaultRootObject>
  <Origins>
    <Quantity>1</Quantity>
    <Items>
      <Origin>
        <Id>primary</Id>
        <DomainName>{origin_domain}</DomainName>
        <OriginPath>{origin_path}</OriginPath>
        <CustomHeaders><Quantity>0</Quantity></CustomHeaders>
        <S3OriginConfig><OriginAccessIdentity></OriginAccessIdentity></S3OriginConfig>
      </Origin>
    </Items>
  </Origins>
  <DefaultCacheBehavior>
    <TargetOriginId>primary</TargetOriginId>
    <ViewerProtocolPolicy>allow-all</ViewerProtocolPolicy>
    <AllowedMethods>
      <Quantity>2</Quantity>
      <Items><Method>GET</Method><Method>HEAD</Method></Items>
      <CachedMethods>
        <Quantity>2</Quantity>
        <Items><Method>GET</Method><Method>HEAD</Method></Items>
      </CachedMethods>
    </AllowedMethods>
    <Compress>false</Compress>
    <CachePolicyId>658327ea-f89d-4fab-a63d-7e88639e58f6</CachePolicyId>
  </DefaultCacheBehavior>
  <CacheBehaviors><Quantity>0</Quantity></CacheBehaviors>
  <CustomErrorResponses><Quantity>0</Quantity></CustomErrorResponses>
  <Comment>integration-test</Comment>
  <PriceClass>PriceClass_All</PriceClass>
  <Enabled>{enabled}</Enabled>
  <ViewerCertificate><CloudFrontDefaultCertificate>true</CloudFrontDefaultCertificate></ViewerCertificate>
  <Restrictions><GeoRestriction><RestrictionType>none</RestrictionType><Quantity>0</Quantity></GeoRestriction></Restrictions>
  <HttpVersion>http2</HttpVersion>
  <IsIPV6Enabled>false</IsIPV6Enabled>
</DistributionConfig>"#
    )
}

/// DistributionConfig with an ordered `CacheBehaviors` list (one extra behavior for path `*.jpg`).
#[must_use]
pub fn distribution_xml_with_cache_behaviors(
    caller_ref: &str,
    origin_domain: &str,
    enabled: bool,
    response_headers_policy_id: &str,
) -> String {
    let rhp_line = if response_headers_policy_id.is_empty() {
        String::new()
    } else {
        format!("<ResponseHeadersPolicyId>{response_headers_policy_id}</ResponseHeadersPolicyId>")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DistributionConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>{caller_ref}</CallerReference>
  <Aliases><Quantity>0</Quantity></Aliases>
  <DefaultRootObject></DefaultRootObject>
  <Origins>
    <Quantity>1</Quantity>
    <Items>
      <Origin>
        <Id>primary</Id>
        <DomainName>{origin_domain}</DomainName>
        <OriginPath></OriginPath>
        <CustomHeaders><Quantity>0</Quantity></CustomHeaders>
        <S3OriginConfig><OriginAccessIdentity></OriginAccessIdentity></S3OriginConfig>
      </Origin>
    </Items>
  </Origins>
  <DefaultCacheBehavior>
    <TargetOriginId>primary</TargetOriginId>
    <ViewerProtocolPolicy>allow-all</ViewerProtocolPolicy>
    <AllowedMethods>
      <Quantity>2</Quantity>
      <Items><Method>GET</Method><Method>HEAD</Method></Items>
      <CachedMethods>
        <Quantity>2</Quantity>
        <Items><Method>GET</Method><Method>HEAD</Method></Items>
      </CachedMethods>
    </AllowedMethods>
    <Compress>false</Compress>
    <CachePolicyId>658327ea-f89d-4fab-a63d-7e88639e58f6</CachePolicyId>
    {rhp_line}
  </DefaultCacheBehavior>
  <CacheBehaviors>
    <Quantity>1</Quantity>
    <Items>
      <CacheBehavior>
        <PathPattern>*.jpg</PathPattern>
        <TargetOriginId>primary</TargetOriginId>
        <ViewerProtocolPolicy>allow-all</ViewerProtocolPolicy>
        <AllowedMethods>
          <Quantity>1</Quantity>
          <Items><Method>GET</Method></Items>
          <CachedMethods>
            <Quantity>1</Quantity>
            <Items><Method>GET</Method></Items>
          </CachedMethods>
        </AllowedMethods>
        <Compress>false</Compress>
        <CachePolicyId>658327ea-f89d-4fab-a63d-7e88639e58f6</CachePolicyId>
      </CacheBehavior>
    </Items>
  </CacheBehaviors>
  <CustomErrorResponses><Quantity>0</Quantity></CustomErrorResponses>
  <Comment>cache-behavior-test</Comment>
  <PriceClass>PriceClass_All</PriceClass>
  <Enabled>{enabled}</Enabled>
  <ViewerCertificate><CloudFrontDefaultCertificate>true</CloudFrontDefaultCertificate></ViewerCertificate>
  <Restrictions><GeoRestriction><RestrictionType>none</RestrictionType><Quantity>0</Quantity></GeoRestriction></Restrictions>
</DistributionConfig>"#
    )
}

/// DistributionConfig with a Lambda\@Edge association on the default behaviour.
#[must_use]
pub fn distribution_xml_with_lambda_edge(
    caller_ref: &str,
    origin_domain: &str,
    enabled: bool,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DistributionConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>{caller_ref}</CallerReference>
  <Aliases><Quantity>0</Quantity></Aliases>
  <DefaultRootObject></DefaultRootObject>
  <Origins>
    <Quantity>1</Quantity>
    <Items>
      <Origin>
        <Id>primary</Id>
        <DomainName>{origin_domain}</DomainName>
        <OriginPath></OriginPath>
        <CustomHeaders><Quantity>0</Quantity></CustomHeaders>
        <S3OriginConfig><OriginAccessIdentity></OriginAccessIdentity></S3OriginConfig>
      </Origin>
    </Items>
  </Origins>
  <DefaultCacheBehavior>
    <TargetOriginId>primary</TargetOriginId>
    <ViewerProtocolPolicy>allow-all</ViewerProtocolPolicy>
    <AllowedMethods>
      <Quantity>2</Quantity>
      <Items><Method>GET</Method><Method>HEAD</Method></Items>
      <CachedMethods>
        <Quantity>2</Quantity>
        <Items><Method>GET</Method><Method>HEAD</Method></Items>
      </CachedMethods>
    </AllowedMethods>
    <Compress>false</Compress>
    <CachePolicyId>658327ea-f89d-4fab-a63d-7e88639e58f6</CachePolicyId>
    <LambdaFunctionAssociations>
      <Quantity>1</Quantity>
      <Items>
        <LambdaFunctionAssociation>
          <LambdaFunctionARN>arn:aws:lambda:us-east-1:000000000000:function:edge-fn:1</LambdaFunctionARN>
          <EventType>viewer-request</EventType>
          <IncludeBody>false</IncludeBody>
        </LambdaFunctionAssociation>
      </Items>
    </LambdaFunctionAssociations>
  </DefaultCacheBehavior>
  <CacheBehaviors><Quantity>0</Quantity></CacheBehaviors>
  <CustomErrorResponses><Quantity>0</Quantity></CustomErrorResponses>
  <Comment>lambda-edge-test</Comment>
  <PriceClass>PriceClass_All</PriceClass>
  <Enabled>{enabled}</Enabled>
  <ViewerCertificate><CloudFrontDefaultCertificate>true</CloudFrontDefaultCertificate></ViewerCertificate>
  <Restrictions><GeoRestriction><RestrictionType>none</RestrictionType><Quantity>0</Quantity></GeoRestriction></Restrictions>
</DistributionConfig>"#
    )
}

/// DistributionConfig with a `CustomErrorResponses` entry mapping 404 → `/not-found.html`.
#[must_use]
pub fn distribution_xml_with_custom_errors(
    caller_ref: &str,
    origin_domain: &str,
    enabled: bool,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DistributionConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>{caller_ref}</CallerReference>
  <Aliases><Quantity>0</Quantity></Aliases>
  <DefaultRootObject></DefaultRootObject>
  <Origins>
    <Quantity>1</Quantity>
    <Items>
      <Origin>
        <Id>primary</Id>
        <DomainName>{origin_domain}</DomainName>
        <OriginPath></OriginPath>
        <CustomHeaders><Quantity>0</Quantity></CustomHeaders>
        <S3OriginConfig><OriginAccessIdentity></OriginAccessIdentity></S3OriginConfig>
      </Origin>
    </Items>
  </Origins>
  <DefaultCacheBehavior>
    <TargetOriginId>primary</TargetOriginId>
    <ViewerProtocolPolicy>allow-all</ViewerProtocolPolicy>
    <AllowedMethods>
      <Quantity>2</Quantity>
      <Items><Method>GET</Method><Method>HEAD</Method></Items>
      <CachedMethods>
        <Quantity>2</Quantity>
        <Items><Method>GET</Method><Method>HEAD</Method></Items>
      </CachedMethods>
    </AllowedMethods>
    <Compress>false</Compress>
    <CachePolicyId>658327ea-f89d-4fab-a63d-7e88639e58f6</CachePolicyId>
  </DefaultCacheBehavior>
  <CacheBehaviors><Quantity>0</Quantity></CacheBehaviors>
  <CustomErrorResponses>
    <Quantity>1</Quantity>
    <Items>
      <CustomErrorResponse>
        <ErrorCode>404</ErrorCode>
        <ResponsePagePath>/not-found.html</ResponsePagePath>
        <ResponseCode>200</ResponseCode>
        <ErrorCachingMinTTL>10</ErrorCachingMinTTL>
      </CustomErrorResponse>
    </Items>
  </CustomErrorResponses>
  <Comment>custom-error-test</Comment>
  <PriceClass>PriceClass_All</PriceClass>
  <Enabled>{enabled}</Enabled>
  <ViewerCertificate><CloudFrontDefaultCertificate>true</CloudFrontDefaultCertificate></ViewerCertificate>
  <Restrictions><GeoRestriction><RestrictionType>none</RestrictionType><Quantity>0</Quantity></GeoRestriction></Restrictions>
</DistributionConfig>"#
    )
}

/// Create a distribution and return `(id, etag)`. Panics on non-201.
pub async fn create_distribution(client: &Client, body: &str) -> (String, String) {
    let resp = cf_post(client, "/2020-05-31/distribution", body).await;
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "create_distribution status"
    );
    let etag = response_etag(&resp).expect("etag on create");
    let xml = resp.text().await.expect("create body");
    let id = extract_distribution_id(&xml).expect("distribution id in response");
    (id, etag)
}

/// Delete a distribution. If `enabled` is true, first disable via Update.
pub async fn disable_and_delete_distribution(client: &Client, id: &str, etag: &str, enabled: bool) {
    let current_etag = etag.to_owned();
    let current_etag = if enabled {
        let got = cf_get(client, &format!("/2020-05-31/distribution/{id}/config")).await;
        assert_eq!(got.status(), StatusCode::OK, "get config for delete");
        let cur_etag = response_etag(&got).expect("etag");
        let body = got
            .text()
            .await
            .unwrap()
            .replace("<Enabled>true</Enabled>", "<Enabled>false</Enabled>");
        let put = cf_put(
            client,
            &format!("/2020-05-31/distribution/{id}/config"),
            &body,
            Some(&cur_etag),
        )
        .await;
        assert_eq!(put.status(), StatusCode::OK, "update to disabled");
        response_etag(&put).expect("etag after update")
    } else {
        current_etag
    };
    let resp = cf_delete(
        client,
        &format!("/2020-05-31/distribution/{id}"),
        Some(&current_etag),
    )
    .await;
    assert!(
        resp.status() == StatusCode::NO_CONTENT,
        "delete status was {}",
        resp.status()
    );
}

/// Unique caller reference per test.
#[must_use]
pub fn new_caller_ref(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}
