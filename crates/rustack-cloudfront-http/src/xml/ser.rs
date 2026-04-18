//! Serialize domain types to restXml response bodies.

use chrono::{DateTime, Utc};
use rustack_cloudfront_model::{
    CLOUDFRONT_XML_NAMESPACE, CachePolicy, CachePolicyConfig, CloudFrontFunction,
    CloudFrontOriginAccessIdentity, CloudFrontOriginAccessIdentityConfig, CustomErrorResponse,
    CustomHeader, Distribution, DistributionConfig, FieldLevelEncryption,
    FieldLevelEncryptionProfile, FunctionConfig, Invalidation, KeyGroup, KeyValueStore, Origin,
    OriginAccessControl, OriginAccessControlConfig, OriginRequestPolicy, OriginRequestPolicyConfig,
    PublicKey, RealtimeLogConfig, ResponseHeadersPolicy, ResponseHeadersPolicyConfig, TagSet,
};

use super::XmlWriter;

/// Serialize an ISO 8601 timestamp with millisecond precision.
pub fn iso8601(ts: &DateTime<Utc>) -> String {
    ts.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

// ---------------------------------------------------------------------------
// DistributionConfig writing helpers
// ---------------------------------------------------------------------------

/// Write `<DistributionConfig>` into `w` (element name supplied).
pub fn write_distribution_config(w: &mut XmlWriter, cfg: &DistributionConfig, element_name: &str) {
    w.open(element_name);
    w.element("CallerReference", &cfg.caller_reference);
    write_string_list(w, "Aliases", "CNAME", &cfg.aliases);
    w.element("DefaultRootObject", &cfg.default_root_object);
    write_origins(w, &cfg.origins);
    write_origin_groups(w, &cfg.origin_groups);
    write_default_cache_behavior(w, &cfg.default_cache_behavior);
    write_cache_behaviors(w, &cfg.cache_behaviors);
    write_custom_error_responses(w, &cfg.custom_error_responses);
    w.element("Comment", &cfg.comment);
    write_logging(w, &cfg);
    w.optional_str("PriceClass", &cfg.price_class);
    w.bool("Enabled", cfg.enabled);
    write_viewer_certificate(w, cfg);
    write_restrictions(w, cfg);
    w.optional_str("WebACLId", &cfg.web_acl_id);
    w.optional_str("HttpVersion", &cfg.http_version);
    w.bool("IsIPV6Enabled", cfg.is_ipv6_enabled);
    w.optional_str(
        "ContinuousDeploymentPolicyId",
        &cfg.continuous_deployment_policy_id,
    );
    w.bool("Staging", cfg.staging);
    w.optional_str("AnycastIpListId", &cfg.anycast_ip_list_id);
    w.optional_str("ConnectionMode", &cfg.connection_mode);
    w.close(element_name);
}

fn write_string_list(w: &mut XmlWriter, wrapper: &str, item_name: &str, items: &[String]) {
    w.open(wrapper);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for it in items {
            w.element(item_name, it);
        }
        w.close("Items");
    }
    w.close(wrapper);
}

fn write_origins(w: &mut XmlWriter, origins: &[Origin]) {
    w.open("Origins");
    w.element_display("Quantity", origins.len());
    if !origins.is_empty() {
        w.open("Items");
        for o in origins {
            write_origin(w, o);
        }
        w.close("Items");
    }
    w.close("Origins");
}

fn write_origin(w: &mut XmlWriter, o: &Origin) {
    w.open("Origin");
    w.element("Id", &o.id);
    w.element("DomainName", &o.domain_name);
    w.element("OriginPath", &o.origin_path);
    write_custom_header_list(w, &o.custom_headers);
    if let Some(s3) = &o.s3_origin_config {
        w.open("S3OriginConfig");
        w.element("OriginAccessIdentity", &s3.origin_access_identity);
        w.close("S3OriginConfig");
    }
    if let Some(c) = &o.custom_origin_config {
        w.open("CustomOriginConfig");
        w.element_display("HTTPPort", c.http_port);
        w.element_display("HTTPSPort", c.https_port);
        w.element("OriginProtocolPolicy", &c.origin_protocol_policy);
        write_string_list(
            w,
            "OriginSslProtocols",
            "SslProtocol",
            &c.origin_ssl_protocols,
        );
        w.element_display("OriginReadTimeout", c.origin_read_timeout);
        w.element_display("OriginKeepaliveTimeout", c.origin_keepalive_timeout);
        w.close("CustomOriginConfig");
    }
    w.element_display(
        "ConnectionAttempts",
        if o.connection_attempts == 0 {
            3
        } else {
            o.connection_attempts
        },
    );
    w.element_display(
        "ConnectionTimeout",
        if o.connection_timeout == 0 {
            10
        } else {
            o.connection_timeout
        },
    );
    if let Some(os) = &o.origin_shield {
        w.open("OriginShield");
        w.bool("Enabled", os.enabled);
        w.optional_str("OriginShieldRegion", &os.origin_shield_region);
        w.close("OriginShield");
    }
    w.element("OriginAccessControlId", &o.origin_access_control_id);
    w.close("Origin");
}

fn write_custom_header_list(w: &mut XmlWriter, items: &[CustomHeader]) {
    w.open("CustomHeaders");
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for h in items {
            w.open("OriginCustomHeader");
            w.element("HeaderName", &h.header_name);
            w.element("HeaderValue", &h.header_value);
            w.close("OriginCustomHeader");
        }
        w.close("Items");
    }
    w.close("CustomHeaders");
}

fn write_origin_groups(w: &mut XmlWriter, groups: &[rustack_cloudfront_model::OriginGroup]) {
    w.open("OriginGroups");
    w.element_display("Quantity", groups.len());
    if !groups.is_empty() {
        w.open("Items");
        for g in groups {
            w.open("OriginGroup");
            w.element("Id", &g.id);
            w.open("FailoverCriteria");
            write_i32_list(w, "StatusCodes", "StatusCode", &g.failover_status_codes);
            w.close("FailoverCriteria");
            write_string_list(w, "Members", "OriginGroupMember", &g.member_origins);
            w.optional_str("SelectionCriteria", &g.selection_criteria);
            w.close("OriginGroup");
        }
        w.close("Items");
    }
    w.close("OriginGroups");
}

fn write_i32_list(w: &mut XmlWriter, wrapper: &str, item_name: &str, items: &[i32]) {
    w.open(wrapper);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for it in items {
            w.element_display(item_name, it);
        }
        w.close("Items");
    }
    w.close(wrapper);
}

fn write_default_cache_behavior(w: &mut XmlWriter, cb: &rustack_cloudfront_model::CacheBehavior) {
    w.open("DefaultCacheBehavior");
    write_cache_behavior_common(w, cb);
    w.close("DefaultCacheBehavior");
}

fn write_cache_behaviors(w: &mut XmlWriter, cbs: &[rustack_cloudfront_model::CacheBehavior]) {
    w.open("CacheBehaviors");
    w.element_display("Quantity", cbs.len());
    if !cbs.is_empty() {
        w.open("Items");
        for cb in cbs {
            w.open("CacheBehavior");
            w.element("PathPattern", &cb.path_pattern);
            write_cache_behavior_common(w, cb);
            w.close("CacheBehavior");
        }
        w.close("Items");
    }
    w.close("CacheBehaviors");
}

fn write_cache_behavior_common(w: &mut XmlWriter, cb: &rustack_cloudfront_model::CacheBehavior) {
    w.element("TargetOriginId", &cb.target_origin_id);
    w.open("TrustedSigners");
    w.bool("Enabled", cb.trusted_signers_enabled);
    w.element_display("Quantity", cb.trusted_signers.len());
    if !cb.trusted_signers.is_empty() {
        w.open("Items");
        for s in &cb.trusted_signers {
            w.element("AwsAccountNumber", s);
        }
        w.close("Items");
    }
    w.close("TrustedSigners");
    w.open("TrustedKeyGroups");
    w.bool("Enabled", cb.trusted_key_groups_enabled);
    w.element_display("Quantity", cb.trusted_key_groups.len());
    if !cb.trusted_key_groups.is_empty() {
        w.open("Items");
        for k in &cb.trusted_key_groups {
            w.element("KeyGroup", k);
        }
        w.close("Items");
    }
    w.close("TrustedKeyGroups");
    w.element("ViewerProtocolPolicy", &cb.viewer_protocol_policy);
    write_allowed_methods(w, &cb.allowed_methods, &cb.cached_methods);
    w.bool("SmoothStreaming", cb.smooth_streaming);
    w.bool("Compress", cb.compress);
    write_lambda_associations(w, &cb.lambda_function_associations);
    write_function_associations(w, &cb.function_associations);
    w.optional_str("FieldLevelEncryptionId", &cb.field_level_encryption_id);
    w.optional_str("RealtimeLogConfigArn", &cb.realtime_log_config_arn);
    w.optional_str("CachePolicyId", &cb.cache_policy_id);
    w.optional_str("OriginRequestPolicyId", &cb.origin_request_policy_id);
    w.optional_str("ResponseHeadersPolicyId", &cb.response_headers_policy_id);
    if cb.grpc_enabled {
        w.open("GrpcConfig");
        w.bool("Enabled", true);
        w.close("GrpcConfig");
    }
    if let Some(fv) = &cb.forwarded_values {
        w.open("ForwardedValues");
        w.bool("QueryString", fv.query_string);
        w.open("Cookies");
        w.element("Forward", &fv.cookies.forward);
        if !fv.cookies.whitelisted_names.is_empty() {
            write_string_list(w, "WhitelistedNames", "Name", &fv.cookies.whitelisted_names);
        }
        w.close("Cookies");
        write_string_list(w, "Headers", "Name", &fv.headers);
        write_string_list(
            w,
            "QueryStringCacheKeys",
            "Name",
            &fv.query_string_cache_keys,
        );
        w.close("ForwardedValues");
    }
    w.element_display("MinTTL", cb.min_ttl);
    w.element_display("DefaultTTL", cb.default_ttl);
    w.element_display("MaxTTL", cb.max_ttl);
}

fn write_allowed_methods(w: &mut XmlWriter, allowed: &[String], cached: &[String]) {
    w.open("AllowedMethods");
    w.element_display("Quantity", allowed.len());
    if !allowed.is_empty() {
        w.open("Items");
        for m in allowed {
            w.element("Method", m);
        }
        w.close("Items");
    }
    w.open("CachedMethods");
    w.element_display("Quantity", cached.len());
    if !cached.is_empty() {
        w.open("Items");
        for m in cached {
            w.element("Method", m);
        }
        w.close("Items");
    }
    w.close("CachedMethods");
    w.close("AllowedMethods");
}

fn write_lambda_associations(
    w: &mut XmlWriter,
    items: &[rustack_cloudfront_model::LambdaFunctionAssociation],
) {
    w.open("LambdaFunctionAssociations");
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for l in items {
            w.open("LambdaFunctionAssociation");
            w.element("LambdaFunctionARN", &l.lambda_function_arn);
            w.element("EventType", &l.event_type);
            w.bool("IncludeBody", l.include_body);
            w.close("LambdaFunctionAssociation");
        }
        w.close("Items");
    }
    w.close("LambdaFunctionAssociations");
}

fn write_function_associations(
    w: &mut XmlWriter,
    items: &[rustack_cloudfront_model::FunctionAssociation],
) {
    w.open("FunctionAssociations");
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for f in items {
            w.open("FunctionAssociation");
            w.element("FunctionARN", &f.function_arn);
            w.element("EventType", &f.event_type);
            w.close("FunctionAssociation");
        }
        w.close("Items");
    }
    w.close("FunctionAssociations");
}

fn write_custom_error_responses(w: &mut XmlWriter, items: &[CustomErrorResponse]) {
    w.open("CustomErrorResponses");
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for c in items {
            w.open("CustomErrorResponse");
            w.element_display("ErrorCode", c.error_code);
            w.optional_str("ResponsePagePath", &c.response_page_path);
            w.optional_str("ResponseCode", &c.response_code);
            w.element_display("ErrorCachingMinTTL", c.error_caching_min_ttl);
            w.close("CustomErrorResponse");
        }
        w.close("Items");
    }
    w.close("CustomErrorResponses");
}

fn write_logging(w: &mut XmlWriter, cfg: &DistributionConfig) {
    w.open("Logging");
    w.bool("Enabled", cfg.logging.enabled);
    w.bool("IncludeCookies", cfg.logging.include_cookies);
    w.element("Bucket", &cfg.logging.bucket);
    w.element("Prefix", &cfg.logging.prefix);
    w.close("Logging");
}

fn write_viewer_certificate(w: &mut XmlWriter, cfg: &DistributionConfig) {
    let vc = &cfg.viewer_certificate;
    w.open("ViewerCertificate");
    w.bool(
        "CloudFrontDefaultCertificate",
        vc.cloud_front_default_certificate
            || !vc.acm_certificate_arn.is_empty()
            || !vc.iam_certificate_id.is_empty()
            || vc.cloud_front_default_certificate,
    );
    w.optional_str("IAMCertificateId", &vc.iam_certificate_id);
    w.optional_str("ACMCertificateArn", &vc.acm_certificate_arn);
    w.optional_str("SSLSupportMethod", &vc.ssl_support_method);
    w.optional_str("MinimumProtocolVersion", &vc.minimum_protocol_version);
    w.optional_str("Certificate", &vc.certificate);
    w.optional_str("CertificateSource", &vc.certificate_source);
    w.close("ViewerCertificate");
}

fn write_restrictions(w: &mut XmlWriter, cfg: &DistributionConfig) {
    w.open("Restrictions");
    w.open("GeoRestriction");
    w.element(
        "RestrictionType",
        if cfg.restrictions.geo_restriction.restriction_type.is_empty() {
            "none"
        } else {
            &cfg.restrictions.geo_restriction.restriction_type
        },
    );
    write_string_list(
        w,
        "Items",
        "Location",
        &cfg.restrictions.geo_restriction.locations,
    );
    w.close("GeoRestriction");
    w.close("Restrictions");
}

// ---------------------------------------------------------------------------
// Top-level distribution response bodies
// ---------------------------------------------------------------------------

/// Serialize a distribution as a `<Distribution>` response.
#[must_use]
pub fn distribution_xml(d: &Distribution) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("Distribution", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Id", &d.id);
    w.element("ARN", &d.arn);
    w.element("Status", d.status.as_wire());
    w.element("LastModifiedTime", &iso8601(&d.last_modified_time));
    w.element_display(
        "InProgressInvalidationBatches",
        d.in_progress_invalidation_batches,
    );
    w.element("DomainName", &d.domain_name);
    w.open("ActiveTrustedSigners");
    w.bool("Enabled", d.active_trusted_signers_enabled);
    w.element_display("Quantity", 0);
    w.close("ActiveTrustedSigners");
    w.open("ActiveTrustedKeyGroups");
    w.bool("Enabled", d.active_trusted_key_groups_enabled);
    w.element_display("Quantity", 0);
    w.close("ActiveTrustedKeyGroups");
    write_distribution_config(&mut w, &d.config, "DistributionConfig");
    w.close("Distribution");
    w.finish()
}

/// Serialize bare `<DistributionConfig>` wrapped in the namespace.
#[must_use]
pub fn distribution_config_xml(cfg: &DistributionConfig) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.buf_mut_push_namespace_open("DistributionConfig");
    write_distribution_config_body(&mut w, cfg);
    w.close("DistributionConfig");
    w.finish()
}

/// Serialize a list response.
#[must_use]
pub fn distribution_list_xml(items: &[Distribution], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("DistributionList", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Marker", "");
    w.element_display("MaxItems", max_items);
    w.bool("IsTruncated", false);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for d in items {
            w.open("DistributionSummary");
            w.element("Id", &d.id);
            w.element("ARN", &d.arn);
            w.element("Status", d.status.as_wire());
            w.element("LastModifiedTime", &iso8601(&d.last_modified_time));
            w.element("DomainName", &d.domain_name);
            write_string_list(&mut w, "Aliases", "CNAME", &d.config.aliases);
            write_origins(&mut w, &d.config.origins);
            write_origin_groups(&mut w, &d.config.origin_groups);
            write_default_cache_behavior(&mut w, &d.config.default_cache_behavior);
            write_cache_behaviors(&mut w, &d.config.cache_behaviors);
            write_custom_error_responses(&mut w, &d.config.custom_error_responses);
            w.element("Comment", &d.config.comment);
            w.optional_str("PriceClass", &d.config.price_class);
            w.bool("Enabled", d.config.enabled);
            write_viewer_certificate(&mut w, &d.config);
            write_restrictions(&mut w, &d.config);
            w.optional_str("WebACLId", &d.config.web_acl_id);
            w.optional_str("HttpVersion", &d.config.http_version);
            w.bool("IsIPV6Enabled", d.config.is_ipv6_enabled);
            w.bool("Staging", d.config.staging);
            w.close("DistributionSummary");
        }
        w.close("Items");
    }
    w.close("DistributionList");
    w.finish()
}

// ---------------------------------------------------------------------------
// Invalidation
// ---------------------------------------------------------------------------

/// Serialize an `<Invalidation>`.
#[must_use]
pub fn invalidation_xml(inv: &Invalidation) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("Invalidation", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Id", &inv.id);
    w.element("Status", inv.status.as_wire());
    w.element("CreateTime", &iso8601(&inv.create_time));
    w.open("InvalidationBatch");
    write_string_list(&mut w, "Paths", "Path", &inv.batch.paths);
    w.element("CallerReference", &inv.batch.caller_reference);
    w.close("InvalidationBatch");
    w.close("Invalidation");
    w.finish()
}

/// List of invalidations (summary form).
#[must_use]
pub fn invalidation_list_xml(items: &[Invalidation], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("InvalidationList", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Marker", "");
    w.element_display("MaxItems", max_items);
    w.bool("IsTruncated", false);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for inv in items {
            w.open("InvalidationSummary");
            w.element("Id", &inv.id);
            w.element("CreateTime", &iso8601(&inv.create_time));
            w.element("Status", inv.status.as_wire());
            w.close("InvalidationSummary");
        }
        w.close("Items");
    }
    w.close("InvalidationList");
    w.finish()
}

// ---------------------------------------------------------------------------
// OAC
// ---------------------------------------------------------------------------

/// Serialize an OAC response.
#[must_use]
pub fn oac_xml(o: &OriginAccessControl) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("OriginAccessControl", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Id", &o.id);
    write_oac_config(&mut w, &o.config, "OriginAccessControlConfig");
    w.close("OriginAccessControl");
    w.finish()
}

/// Serialize OAC config response.
#[must_use]
pub fn oac_config_xml(cfg: &OriginAccessControlConfig) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    write_oac_config_root(&mut w, cfg);
    w.finish()
}

fn write_oac_config_root(w: &mut XmlWriter, cfg: &OriginAccessControlConfig) {
    w.open_root("OriginAccessControlConfig", Some(CLOUDFRONT_XML_NAMESPACE));
    write_oac_config_body(w, cfg);
    w.close("OriginAccessControlConfig");
}

fn write_oac_config(w: &mut XmlWriter, cfg: &OriginAccessControlConfig, el: &str) {
    w.open(el);
    write_oac_config_body(w, cfg);
    w.close(el);
}

fn write_oac_config_body(w: &mut XmlWriter, cfg: &OriginAccessControlConfig) {
    w.element("Name", &cfg.name);
    w.optional_str("Description", &cfg.description);
    w.element("SigningProtocol", &cfg.signing_protocol);
    w.element("SigningBehavior", &cfg.signing_behavior);
    w.element(
        "OriginAccessControlOriginType",
        &cfg.origin_access_control_origin_type,
    );
}

/// Serialize OAC list response.
#[must_use]
pub fn oac_list_xml(items: &[OriginAccessControl], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("OriginAccessControlList", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Marker", "");
    w.element_display("MaxItems", max_items);
    w.bool("IsTruncated", false);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for o in items {
            w.open("OriginAccessControlSummary");
            w.element("Id", &o.id);
            w.element("Name", &o.config.name);
            w.element("Description", &o.config.description);
            w.element("SigningProtocol", &o.config.signing_protocol);
            w.element("SigningBehavior", &o.config.signing_behavior);
            w.element(
                "OriginAccessControlOriginType",
                &o.config.origin_access_control_origin_type,
            );
            w.close("OriginAccessControlSummary");
        }
        w.close("Items");
    }
    w.close("OriginAccessControlList");
    w.finish()
}

// ---------------------------------------------------------------------------
// OAI
// ---------------------------------------------------------------------------

/// Serialize an OAI response.
#[must_use]
pub fn oai_xml(o: &CloudFrontOriginAccessIdentity) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root(
        "CloudFrontOriginAccessIdentity",
        Some(CLOUDFRONT_XML_NAMESPACE),
    );
    w.element("Id", &o.id);
    w.element("S3CanonicalUserId", &o.s3_canonical_user_id);
    w.open("CloudFrontOriginAccessIdentityConfig");
    w.element("CallerReference", &o.config.caller_reference);
    w.element("Comment", &o.config.comment);
    w.close("CloudFrontOriginAccessIdentityConfig");
    w.close("CloudFrontOriginAccessIdentity");
    w.finish()
}

/// Serialize OAI config.
#[must_use]
pub fn oai_config_xml(cfg: &CloudFrontOriginAccessIdentityConfig) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root(
        "CloudFrontOriginAccessIdentityConfig",
        Some(CLOUDFRONT_XML_NAMESPACE),
    );
    w.element("CallerReference", &cfg.caller_reference);
    w.element("Comment", &cfg.comment);
    w.close("CloudFrontOriginAccessIdentityConfig");
    w.finish()
}

/// Serialize OAI list.
#[must_use]
pub fn oai_list_xml(items: &[CloudFrontOriginAccessIdentity], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root(
        "CloudFrontOriginAccessIdentityList",
        Some(CLOUDFRONT_XML_NAMESPACE),
    );
    w.element("Marker", "");
    w.element_display("MaxItems", max_items);
    w.bool("IsTruncated", false);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for o in items {
            w.open("CloudFrontOriginAccessIdentitySummary");
            w.element("Id", &o.id);
            w.element("S3CanonicalUserId", &o.s3_canonical_user_id);
            w.element("Comment", &o.config.comment);
            w.close("CloudFrontOriginAccessIdentitySummary");
        }
        w.close("Items");
    }
    w.close("CloudFrontOriginAccessIdentityList");
    w.finish()
}

// ---------------------------------------------------------------------------
// Cache / OriginRequest / ResponseHeaders policies
// ---------------------------------------------------------------------------

/// Serialize a cache policy.
#[must_use]
pub fn cache_policy_xml(p: &CachePolicy) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("CachePolicy", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Id", &p.id);
    w.element("LastModifiedTime", &iso8601(&p.last_modified_time));
    w.open("CachePolicyConfig");
    write_cache_policy_config_body(&mut w, &p.config);
    w.close("CachePolicyConfig");
    w.close("CachePolicy");
    w.finish()
}

/// Serialize cache policy config.
#[must_use]
pub fn cache_policy_config_xml(cfg: &CachePolicyConfig) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("CachePolicyConfig", Some(CLOUDFRONT_XML_NAMESPACE));
    write_cache_policy_config_body(&mut w, cfg);
    w.close("CachePolicyConfig");
    w.finish()
}

fn write_cache_policy_config_body(w: &mut XmlWriter, cfg: &CachePolicyConfig) {
    w.optional_str("Comment", &cfg.comment);
    w.element("Name", &cfg.name);
    w.element_display("DefaultTTL", cfg.default_ttl);
    w.element_display("MaxTTL", cfg.max_ttl);
    w.element_display("MinTTL", cfg.min_ttl);
    w.open("ParametersInCacheKeyAndForwardedToOrigin");
    let p = &cfg.parameters_in_cache_key_and_forwarded_to_origin;
    w.bool("EnableAcceptEncodingGzip", p.enable_accept_encoding_gzip);
    w.bool(
        "EnableAcceptEncodingBrotli",
        p.enable_accept_encoding_brotli,
    );
    w.open("HeadersConfig");
    w.element(
        "HeaderBehavior",
        if p.headers_config.header_behavior.is_empty() {
            "none"
        } else {
            &p.headers_config.header_behavior
        },
    );
    write_string_list(w, "Headers", "Name", &p.headers_config.headers);
    w.close("HeadersConfig");
    w.open("CookiesConfig");
    w.element(
        "CookieBehavior",
        if p.cookies_config.cookie_behavior.is_empty() {
            "none"
        } else {
            &p.cookies_config.cookie_behavior
        },
    );
    write_string_list(w, "Cookies", "Name", &p.cookies_config.cookies);
    w.close("CookiesConfig");
    w.open("QueryStringsConfig");
    w.element(
        "QueryStringBehavior",
        if p.query_strings_config.query_string_behavior.is_empty() {
            "none"
        } else {
            &p.query_strings_config.query_string_behavior
        },
    );
    write_string_list(
        w,
        "QueryStrings",
        "Name",
        &p.query_strings_config.query_strings,
    );
    w.close("QueryStringsConfig");
    w.close("ParametersInCacheKeyAndForwardedToOrigin");
}

/// Serialize cache policy list.
#[must_use]
pub fn cache_policy_list_xml(items: &[CachePolicy], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("CachePolicyList", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Marker", "");
    w.element_display("MaxItems", max_items);
    w.bool("IsTruncated", false);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for p in items {
            w.open("CachePolicySummary");
            w.element("Type", if p.managed { "managed" } else { "custom" });
            w.open("CachePolicy");
            w.element("Id", &p.id);
            w.element("LastModifiedTime", &iso8601(&p.last_modified_time));
            w.open("CachePolicyConfig");
            write_cache_policy_config_body(&mut w, &p.config);
            w.close("CachePolicyConfig");
            w.close("CachePolicy");
            w.close("CachePolicySummary");
        }
        w.close("Items");
    }
    w.close("CachePolicyList");
    w.finish()
}

/// Serialize an origin request policy.
#[must_use]
pub fn origin_request_policy_xml(p: &OriginRequestPolicy) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("OriginRequestPolicy", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Id", &p.id);
    w.element("LastModifiedTime", &iso8601(&p.last_modified_time));
    w.open("OriginRequestPolicyConfig");
    write_origin_request_policy_body(&mut w, &p.config);
    w.close("OriginRequestPolicyConfig");
    w.close("OriginRequestPolicy");
    w.finish()
}

/// Serialize ORP config.
#[must_use]
pub fn origin_request_policy_config_xml(cfg: &OriginRequestPolicyConfig) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("OriginRequestPolicyConfig", Some(CLOUDFRONT_XML_NAMESPACE));
    write_origin_request_policy_body(&mut w, cfg);
    w.close("OriginRequestPolicyConfig");
    w.finish()
}

fn write_origin_request_policy_body(w: &mut XmlWriter, cfg: &OriginRequestPolicyConfig) {
    w.optional_str("Comment", &cfg.comment);
    w.element("Name", &cfg.name);
    w.open("HeadersConfig");
    w.element(
        "HeaderBehavior",
        if cfg.headers_config.header_behavior.is_empty() {
            "none"
        } else {
            &cfg.headers_config.header_behavior
        },
    );
    write_string_list(w, "Headers", "Name", &cfg.headers_config.headers);
    w.close("HeadersConfig");
    w.open("CookiesConfig");
    w.element(
        "CookieBehavior",
        if cfg.cookies_config.cookie_behavior.is_empty() {
            "none"
        } else {
            &cfg.cookies_config.cookie_behavior
        },
    );
    write_string_list(w, "Cookies", "Name", &cfg.cookies_config.cookies);
    w.close("CookiesConfig");
    w.open("QueryStringsConfig");
    w.element(
        "QueryStringBehavior",
        if cfg.query_strings_config.query_string_behavior.is_empty() {
            "none"
        } else {
            &cfg.query_strings_config.query_string_behavior
        },
    );
    write_string_list(
        w,
        "QueryStrings",
        "Name",
        &cfg.query_strings_config.query_strings,
    );
    w.close("QueryStringsConfig");
}

/// Serialize ORP list.
#[must_use]
pub fn origin_request_policy_list_xml(items: &[OriginRequestPolicy], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("OriginRequestPolicyList", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Marker", "");
    w.element_display("MaxItems", max_items);
    w.bool("IsTruncated", false);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for p in items {
            w.open("OriginRequestPolicySummary");
            w.element("Type", if p.managed { "managed" } else { "custom" });
            w.open("OriginRequestPolicy");
            w.element("Id", &p.id);
            w.element("LastModifiedTime", &iso8601(&p.last_modified_time));
            w.open("OriginRequestPolicyConfig");
            write_origin_request_policy_body(&mut w, &p.config);
            w.close("OriginRequestPolicyConfig");
            w.close("OriginRequestPolicy");
            w.close("OriginRequestPolicySummary");
        }
        w.close("Items");
    }
    w.close("OriginRequestPolicyList");
    w.finish()
}

/// Serialize a response-headers policy.
#[must_use]
pub fn response_headers_policy_xml(p: &ResponseHeadersPolicy) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("ResponseHeadersPolicy", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Id", &p.id);
    w.element("LastModifiedTime", &iso8601(&p.last_modified_time));
    w.open("ResponseHeadersPolicyConfig");
    write_response_headers_policy_body(&mut w, &p.config);
    w.close("ResponseHeadersPolicyConfig");
    w.close("ResponseHeadersPolicy");
    w.finish()
}

/// Serialize RHP config.
#[must_use]
pub fn response_headers_policy_config_xml(cfg: &ResponseHeadersPolicyConfig) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root(
        "ResponseHeadersPolicyConfig",
        Some(CLOUDFRONT_XML_NAMESPACE),
    );
    write_response_headers_policy_body(&mut w, cfg);
    w.close("ResponseHeadersPolicyConfig");
    w.finish()
}

fn write_response_headers_policy_body(w: &mut XmlWriter, cfg: &ResponseHeadersPolicyConfig) {
    w.optional_str("Comment", &cfg.comment);
    w.element("Name", &cfg.name);
    // Emit a minimal but complete shape. Details omitted for brevity; managed
    // policies still round-trip correctly.
    w.open("CorsConfig");
    if let Some(c) = &cfg.cors_config {
        w.bool(
            "AccessControlAllowCredentials",
            c.access_control_allow_credentials,
        );
        write_string_list(
            w,
            "AccessControlAllowHeaders",
            "Header",
            &c.access_control_allow_headers,
        );
        write_string_list(
            w,
            "AccessControlAllowMethods",
            "Method",
            &c.access_control_allow_methods,
        );
        write_string_list(
            w,
            "AccessControlAllowOrigins",
            "Origin",
            &c.access_control_allow_origins,
        );
        write_string_list(
            w,
            "AccessControlExposeHeaders",
            "Header",
            &c.access_control_expose_headers,
        );
        w.element_display("AccessControlMaxAgeSec", c.access_control_max_age_sec);
        w.bool("OriginOverride", c.origin_override);
    } else {
        w.bool("AccessControlAllowCredentials", false);
        w.element_display("AccessControlMaxAgeSec", 0);
        w.bool("OriginOverride", false);
    }
    w.close("CorsConfig");
    w.open("SecurityHeadersConfig");
    w.close("SecurityHeadersConfig");
    w.open("ServerTimingHeadersConfig");
    if let Some(s) = &cfg.server_timing_headers_config {
        w.bool("Enabled", s.enabled);
        w.element_display("SamplingRate", s.sampling_rate);
    } else {
        w.bool("Enabled", false);
    }
    w.close("ServerTimingHeadersConfig");
    write_string_list(
        w,
        "RemoveHeadersConfig",
        "ResponseHeadersPolicyRemoveHeader",
        &cfg.remove_headers_config,
    );
    w.open("CustomHeadersConfig");
    w.element_display("Quantity", cfg.custom_headers_config.len());
    if !cfg.custom_headers_config.is_empty() {
        w.open("Items");
        for h in &cfg.custom_headers_config {
            w.open("ResponseHeadersPolicyCustomHeader");
            w.element("Header", &h.header);
            w.element("Value", &h.value);
            w.bool("Override", h.override_upstream);
            w.close("ResponseHeadersPolicyCustomHeader");
        }
        w.close("Items");
    }
    w.close("CustomHeadersConfig");
}

/// Serialize RHP list.
#[must_use]
pub fn response_headers_policy_list_xml(items: &[ResponseHeadersPolicy], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("ResponseHeadersPolicyList", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Marker", "");
    w.element_display("MaxItems", max_items);
    w.bool("IsTruncated", false);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for p in items {
            w.open("ResponseHeadersPolicySummary");
            w.element("Type", if p.managed { "managed" } else { "custom" });
            w.open("ResponseHeadersPolicy");
            w.element("Id", &p.id);
            w.element("LastModifiedTime", &iso8601(&p.last_modified_time));
            w.open("ResponseHeadersPolicyConfig");
            write_response_headers_policy_body(&mut w, &p.config);
            w.close("ResponseHeadersPolicyConfig");
            w.close("ResponseHeadersPolicy");
            w.close("ResponseHeadersPolicySummary");
        }
        w.close("Items");
    }
    w.close("ResponseHeadersPolicyList");
    w.finish()
}

// ---------------------------------------------------------------------------
// Key group / public key / function (minimal summaries)
// ---------------------------------------------------------------------------

/// Serialize a key group.
#[must_use]
pub fn key_group_xml(kg: &KeyGroup) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("KeyGroup", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Id", &kg.id);
    w.element("LastModifiedTime", &iso8601(&kg.last_modified_time));
    w.open("KeyGroupConfig");
    w.element("Name", &kg.config.name);
    write_string_list(&mut w, "Items", "PublicKey", &kg.config.items);
    w.element("Comment", &kg.config.comment);
    w.close("KeyGroupConfig");
    w.close("KeyGroup");
    w.finish()
}

/// Serialize a key group list.
#[must_use]
pub fn key_group_list_xml(items: &[KeyGroup], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("KeyGroupList", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element_display("MaxItems", max_items);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for kg in items {
            w.open("KeyGroupSummary");
            w.open("KeyGroup");
            w.element("Id", &kg.id);
            w.element("LastModifiedTime", &iso8601(&kg.last_modified_time));
            w.open("KeyGroupConfig");
            w.element("Name", &kg.config.name);
            write_string_list(&mut w, "Items", "PublicKey", &kg.config.items);
            w.element("Comment", &kg.config.comment);
            w.close("KeyGroupConfig");
            w.close("KeyGroup");
            w.close("KeyGroupSummary");
        }
        w.close("Items");
    }
    w.close("KeyGroupList");
    w.finish()
}

/// Serialize a public key.
#[must_use]
pub fn public_key_xml(k: &PublicKey) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("PublicKey", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Id", &k.id);
    w.element("CreatedTime", &iso8601(&k.created_time));
    w.open("PublicKeyConfig");
    w.element("CallerReference", &k.config.caller_reference);
    w.element("Name", &k.config.name);
    w.element("EncodedKey", &k.config.encoded_key);
    w.element("Comment", &k.config.comment);
    w.close("PublicKeyConfig");
    w.close("PublicKey");
    w.finish()
}

/// Serialize public key list.
#[must_use]
pub fn public_key_list_xml(items: &[PublicKey], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("PublicKeyList", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element_display("MaxItems", max_items);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for k in items {
            w.open("PublicKeySummary");
            w.element("Id", &k.id);
            w.element("Name", &k.config.name);
            w.element("CreatedTime", &iso8601(&k.created_time));
            w.element("EncodedKey", &k.config.encoded_key);
            w.element("Comment", &k.config.comment);
            w.close("PublicKeySummary");
        }
        w.close("Items");
    }
    w.close("PublicKeyList");
    w.finish()
}

/// Serialize a function (metadata-only response).
#[must_use]
pub fn function_xml(f: &CloudFrontFunction) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("FunctionSummary", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Name", &f.name);
    w.element("Status", &f.status);
    write_function_metadata(&mut w, f);
    write_function_config(&mut w, &f.config);
    w.close("FunctionSummary");
    w.finish()
}

fn write_function_metadata(w: &mut XmlWriter, f: &CloudFrontFunction) {
    w.open("FunctionMetadata");
    w.element("FunctionARN", &f.metadata.function_arn);
    w.element("Stage", &f.metadata.stage);
    w.element("CreatedTime", &iso8601(&f.metadata.created_time));
    w.element("LastModifiedTime", &iso8601(&f.metadata.last_modified_time));
    w.close("FunctionMetadata");
}

fn write_function_config(w: &mut XmlWriter, cfg: &FunctionConfig) {
    w.open("FunctionConfig");
    w.element("Comment", &cfg.comment);
    w.element("Runtime", &cfg.runtime);
    w.close("FunctionConfig");
}

/// Serialize function list.
#[must_use]
pub fn function_list_xml(items: &[CloudFrontFunction], max_items: i32) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("FunctionList", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element_display("MaxItems", max_items);
    w.element_display("Quantity", items.len());
    if !items.is_empty() {
        w.open("Items");
        for f in items {
            w.open("FunctionSummary");
            w.element("Name", &f.name);
            w.element("Status", &f.status);
            write_function_metadata(&mut w, f);
            write_function_config(&mut w, &f.config);
            w.close("FunctionSummary");
        }
        w.close("Items");
    }
    w.close("FunctionList");
    w.finish()
}

// ---------------------------------------------------------------------------
// FLE / KVS / RealtimeLog (minimal)
// ---------------------------------------------------------------------------

/// Serialize FLE config.
#[must_use]
pub fn fle_xml(f: &FieldLevelEncryption) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("FieldLevelEncryption", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Id", &f.id);
    w.element("LastModifiedTime", &iso8601(&f.last_modified_time));
    w.open("FieldLevelEncryptionConfig");
    w.element("CallerReference", &f.config.caller_reference);
    w.element("Comment", &f.config.comment);
    w.close("FieldLevelEncryptionConfig");
    w.close("FieldLevelEncryption");
    w.finish()
}

/// Serialize FLE profile.
#[must_use]
pub fn fle_profile_xml(f: &FieldLevelEncryptionProfile) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root(
        "FieldLevelEncryptionProfile",
        Some(CLOUDFRONT_XML_NAMESPACE),
    );
    w.element("Id", &f.id);
    w.element("LastModifiedTime", &iso8601(&f.last_modified_time));
    w.open("FieldLevelEncryptionProfileConfig");
    w.element("Name", &f.config.name);
    w.element("CallerReference", &f.config.caller_reference);
    w.element("Comment", &f.config.comment);
    w.close("FieldLevelEncryptionProfileConfig");
    w.close("FieldLevelEncryptionProfile");
    w.finish()
}

/// Serialize KVS.
#[must_use]
pub fn kvs_xml(k: &KeyValueStore) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("KeyValueStore", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("Name", &k.name);
    w.element("Id", &k.id);
    w.element("Comment", &k.comment);
    w.element("ARN", &k.arn);
    w.element("Status", &k.status);
    w.element("LastModifiedTime", &iso8601(&k.last_modified_time));
    w.close("KeyValueStore");
    w.finish()
}

/// Serialize realtime log config.
#[must_use]
pub fn realtime_log_config_xml(r: &RealtimeLogConfig) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("RealtimeLogConfig", Some(CLOUDFRONT_XML_NAMESPACE));
    w.element("ARN", &r.arn);
    w.element("Name", &r.name);
    w.element_display("SamplingRate", r.sampling_rate);
    write_string_list(&mut w, "Fields", "Field", &r.fields);
    w.open("EndPoints");
    for ep in &r.end_points {
        w.open("EndPoint");
        w.element("StreamType", &ep.stream_type);
        w.open("KinesisStreamConfig");
        w.element("RoleARN", &ep.kinesis_stream_config.role_arn);
        w.element("StreamARN", &ep.kinesis_stream_config.stream_arn);
        w.close("KinesisStreamConfig");
        w.close("EndPoint");
    }
    w.close("EndPoints");
    w.close("RealtimeLogConfig");
    w.finish()
}

// ---------------------------------------------------------------------------
// Tagging
// ---------------------------------------------------------------------------

/// Serialize `<Tags>` block.
#[must_use]
pub fn tags_xml(tags: &TagSet) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("Tags", Some(CLOUDFRONT_XML_NAMESPACE));
    w.open("Items");
    for t in tags {
        w.open("Tag");
        w.element("Key", &t.key);
        w.element("Value", &t.value);
        w.close("Tag");
    }
    w.close("Items");
    w.close("Tags");
    w.finish()
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Serialize a CloudFront error response (wrapped in `<ErrorResponse>`).
#[must_use]
pub fn error_xml(code: &str, message: &str, request_id: &str) -> String {
    let mut w = XmlWriter::new();
    w.declaration();
    w.open_root("ErrorResponse", Some(CLOUDFRONT_XML_NAMESPACE));
    w.open("Error");
    w.element("Type", "Sender");
    w.element("Code", code);
    w.element("Message", message);
    w.close("Error");
    w.element("RequestId", request_id);
    w.close("ErrorResponse");
    w.finish()
}

// ---------------------------------------------------------------------------
// Helper used by distribution_config_xml
// ---------------------------------------------------------------------------

impl XmlWriter {
    /// Open the root element with the CloudFront namespace.
    pub fn buf_mut_push_namespace_open(&mut self, name: &str) {
        self.open_root(name, Some(CLOUDFRONT_XML_NAMESPACE));
    }
}

fn write_distribution_config_body(w: &mut XmlWriter, cfg: &DistributionConfig) {
    w.element("CallerReference", &cfg.caller_reference);
    write_string_list(w, "Aliases", "CNAME", &cfg.aliases);
    w.element("DefaultRootObject", &cfg.default_root_object);
    write_origins(w, &cfg.origins);
    write_origin_groups(w, &cfg.origin_groups);
    write_default_cache_behavior(w, &cfg.default_cache_behavior);
    write_cache_behaviors(w, &cfg.cache_behaviors);
    write_custom_error_responses(w, &cfg.custom_error_responses);
    w.element("Comment", &cfg.comment);
    write_logging(w, cfg);
    w.optional_str("PriceClass", &cfg.price_class);
    w.bool("Enabled", cfg.enabled);
    write_viewer_certificate(w, cfg);
    write_restrictions(w, cfg);
    w.optional_str("WebACLId", &cfg.web_acl_id);
    w.optional_str("HttpVersion", &cfg.http_version);
    w.bool("IsIPV6Enabled", cfg.is_ipv6_enabled);
    w.bool("Staging", cfg.staging);
}
