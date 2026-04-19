//! Parse request XML bodies into domain types.

use rustack_cloudfront_model::{
    CacheBehavior, CachePolicyConfig, CachePolicyCookiesConfig, CachePolicyHeadersConfig,
    CachePolicyQueryStringsConfig, CloudFrontError, CloudFrontOriginAccessIdentityConfig,
    CookiePreference, CustomErrorResponse, CustomHeader, CustomOriginConfig, DistributionConfig,
    FieldLevelEncryptionConfig, FieldLevelEncryptionProfileConfig, ForwardedValues,
    FunctionAssociation, FunctionConfig, GeoRestriction, InvalidationBatch, KeyGroupConfig,
    LambdaFunctionAssociation, LoggingConfig, Origin, OriginAccessControlConfig, OriginGroup,
    OriginRequestPolicyConfig, OriginRequestPolicyCookiesConfig, OriginRequestPolicyHeadersConfig,
    OriginRequestPolicyQueryStringsConfig, OriginShield, ParamsInCacheKey, PublicKeyConfig,
    RealtimeLogConfig, ResponseHeadersPolicyConfig, Restrictions, S3OriginConfig, Tag, TagSet,
    ViewerCertificate,
};

use crate::xml::de::{Node, parse};

/// Parse the incoming body into the root node.
///
/// # Errors
/// Returns `MalformedInput` on XML errors.
pub fn parse_root(body: &[u8]) -> Result<Node, CloudFrontError> {
    parse(body).map_err(CloudFrontError::MalformedInput)
}

/// Parse `<DistributionConfig>`.
pub fn parse_distribution_config(n: &Node) -> DistributionConfig {
    DistributionConfig {
        caller_reference: n.child_text("CallerReference").to_owned(),
        aliases: n.string_items("Aliases", "CNAME"),
        default_root_object: n.child_text("DefaultRootObject").to_owned(),
        origins: n
            .items_named("Origins", "Origin")
            .into_iter()
            .map(parse_origin)
            .collect(),
        origin_groups: n
            .items_named("OriginGroups", "OriginGroup")
            .into_iter()
            .map(parse_origin_group)
            .collect(),
        default_cache_behavior: n
            .child("DefaultCacheBehavior")
            .map(parse_cache_behavior)
            .unwrap_or_default(),
        cache_behaviors: n
            .items_named("CacheBehaviors", "CacheBehavior")
            .into_iter()
            .map(parse_cache_behavior)
            .collect(),
        custom_error_responses: n
            .items_named("CustomErrorResponses", "CustomErrorResponse")
            .into_iter()
            .map(parse_custom_error_response)
            .collect(),
        comment: n.child_text("Comment").to_owned(),
        logging: n.child("Logging").map(parse_logging).unwrap_or_default(),
        price_class: n.child_text("PriceClass").to_owned(),
        enabled: n.child_bool("Enabled"),
        viewer_certificate: n
            .child("ViewerCertificate")
            .map(parse_viewer_certificate)
            .unwrap_or_default(),
        restrictions: n
            .child("Restrictions")
            .map(parse_restrictions)
            .unwrap_or_default(),
        web_acl_id: n.child_text("WebACLId").to_owned(),
        http_version: n.child_text("HttpVersion").to_owned(),
        is_ipv6_enabled: n.child_bool("IsIPV6Enabled"),
        continuous_deployment_policy_id: n.child_text("ContinuousDeploymentPolicyId").to_owned(),
        staging: n.child_bool("Staging"),
        anycast_ip_list_id: n.child_text("AnycastIpListId").to_owned(),
        connection_mode: n.child_text("ConnectionMode").to_owned(),
        tenant_config_parameters: Vec::new(),
    }
}

fn parse_origin(n: &Node) -> Origin {
    Origin {
        id: n.child_text("Id").to_owned(),
        domain_name: n.child_text("DomainName").to_owned(),
        origin_path: n.child_text("OriginPath").to_owned(),
        custom_headers: n
            .items_named("CustomHeaders", "OriginCustomHeader")
            .into_iter()
            .map(|c| CustomHeader {
                header_name: c.child_text("HeaderName").to_owned(),
                header_value: c.child_text("HeaderValue").to_owned(),
            })
            .collect(),
        s3_origin_config: n.child("S3OriginConfig").map(|s| S3OriginConfig {
            origin_access_identity: s.child_text("OriginAccessIdentity").to_owned(),
        }),
        custom_origin_config: n.child("CustomOriginConfig").map(|c| CustomOriginConfig {
            http_port: c.child_i32("HTTPPort"),
            https_port: c.child_i32("HTTPSPort"),
            origin_protocol_policy: c.child_text("OriginProtocolPolicy").to_owned(),
            origin_ssl_protocols: c.string_items("OriginSslProtocols", "SslProtocol"),
            origin_read_timeout: c.child_i32("OriginReadTimeout"),
            origin_keepalive_timeout: c.child_i32("OriginKeepaliveTimeout"),
        }),
        connection_attempts: n.child_i32("ConnectionAttempts"),
        connection_timeout: n.child_i32("ConnectionTimeout"),
        origin_shield: n.child("OriginShield").map(|os| OriginShield {
            enabled: os.child_bool("Enabled"),
            origin_shield_region: os.child_text("OriginShieldRegion").to_owned(),
        }),
        origin_access_control_id: n.child_text("OriginAccessControlId").to_owned(),
        vpc_origin_config: None,
    }
}

fn parse_origin_group(n: &Node) -> OriginGroup {
    OriginGroup {
        id: n.child_text("Id").to_owned(),
        failover_status_codes: n
            .child("FailoverCriteria")
            .map(|f| {
                f.items_named("StatusCodes", "StatusCode")
                    .into_iter()
                    .map(|i| i.text.parse().unwrap_or(0))
                    .collect()
            })
            .unwrap_or_default(),
        member_origins: n
            .items_named("Members", "OriginGroupMember")
            .into_iter()
            .map(|x| x.child_text("OriginId").to_owned())
            .collect(),
        selection_criteria: n.child_text("SelectionCriteria").to_owned(),
    }
}

fn parse_cache_behavior(n: &Node) -> CacheBehavior {
    CacheBehavior {
        path_pattern: n.child_text("PathPattern").to_owned(),
        target_origin_id: n.child_text("TargetOriginId").to_owned(),
        viewer_protocol_policy: n.child_text("ViewerProtocolPolicy").to_owned(),
        allowed_methods: n
            .child("AllowedMethods")
            .map(|a| a.direct_string_items("Method"))
            .unwrap_or_default(),
        cached_methods: n
            .child("AllowedMethods")
            .and_then(|a| a.child("CachedMethods"))
            .map(|c| c.direct_string_items("Method"))
            .unwrap_or_default(),
        smooth_streaming: n.child_bool("SmoothStreaming"),
        compress: n.child_bool("Compress"),
        field_level_encryption_id: n.child_text("FieldLevelEncryptionId").to_owned(),
        realtime_log_config_arn: n.child_text("RealtimeLogConfigArn").to_owned(),
        cache_policy_id: n.child_text("CachePolicyId").to_owned(),
        origin_request_policy_id: n.child_text("OriginRequestPolicyId").to_owned(),
        response_headers_policy_id: n.child_text("ResponseHeadersPolicyId").to_owned(),
        grpc_enabled: n
            .child("GrpcConfig")
            .map(|g| g.child_bool("Enabled"))
            .unwrap_or(false),
        trusted_signers: n
            .child("TrustedSigners")
            .map(|t| t.direct_string_items("AwsAccountNumber"))
            .unwrap_or_default(),
        trusted_signers_enabled: n
            .child("TrustedSigners")
            .map(|t| t.child_bool("Enabled"))
            .unwrap_or(false),
        trusted_key_groups: n
            .child("TrustedKeyGroups")
            .map(|t| t.direct_string_items("KeyGroup"))
            .unwrap_or_default(),
        trusted_key_groups_enabled: n
            .child("TrustedKeyGroups")
            .map(|t| t.child_bool("Enabled"))
            .unwrap_or(false),
        lambda_function_associations: n
            .items_named("LambdaFunctionAssociations", "LambdaFunctionAssociation")
            .into_iter()
            .map(|la| LambdaFunctionAssociation {
                lambda_function_arn: la.child_text("LambdaFunctionARN").to_owned(),
                event_type: la.child_text("EventType").to_owned(),
                include_body: la.child_bool("IncludeBody"),
            })
            .collect(),
        function_associations: n
            .items_named("FunctionAssociations", "FunctionAssociation")
            .into_iter()
            .map(|fa| FunctionAssociation {
                function_arn: fa.child_text("FunctionARN").to_owned(),
                event_type: fa.child_text("EventType").to_owned(),
            })
            .collect(),
        forwarded_values: n.child("ForwardedValues").map(|fv| ForwardedValues {
            query_string: fv.child_bool("QueryString"),
            cookies: fv
                .child("Cookies")
                .map(parse_cookie_preference)
                .unwrap_or_default(),
            headers: fv
                .child("Headers")
                .map(|h| h.direct_string_items("Name"))
                .unwrap_or_default(),
            query_string_cache_keys: fv
                .child("QueryStringCacheKeys")
                .map(|q| q.direct_string_items("Name"))
                .unwrap_or_default(),
        }),
        min_ttl: n.child_i64("MinTTL"),
        default_ttl: n.child_i64("DefaultTTL"),
        max_ttl: n.child_i64("MaxTTL"),
    }
}

fn parse_cookie_preference(n: &Node) -> CookiePreference {
    CookiePreference {
        forward: n.child_text("Forward").to_owned(),
        whitelisted_names: n
            .child("WhitelistedNames")
            .map(|w| w.direct_string_items("Name"))
            .unwrap_or_default(),
    }
}

fn parse_custom_error_response(n: &Node) -> CustomErrorResponse {
    CustomErrorResponse {
        error_code: n.child_i32("ErrorCode"),
        response_page_path: n.child_text("ResponsePagePath").to_owned(),
        response_code: n.child_text("ResponseCode").to_owned(),
        error_caching_min_ttl: n.child_i64("ErrorCachingMinTTL"),
    }
}

fn parse_logging(n: &Node) -> LoggingConfig {
    LoggingConfig {
        enabled: n.child_bool("Enabled"),
        include_cookies: n.child_bool("IncludeCookies"),
        bucket: n.child_text("Bucket").to_owned(),
        prefix: n.child_text("Prefix").to_owned(),
    }
}

fn parse_viewer_certificate(n: &Node) -> ViewerCertificate {
    ViewerCertificate {
        cloud_front_default_certificate: n.child_bool("CloudFrontDefaultCertificate"),
        acm_certificate_arn: n.child_text("ACMCertificateArn").to_owned(),
        iam_certificate_id: n.child_text("IAMCertificateId").to_owned(),
        minimum_protocol_version: n.child_text("MinimumProtocolVersion").to_owned(),
        ssl_support_method: n.child_text("SSLSupportMethod").to_owned(),
        certificate: n.child_text("Certificate").to_owned(),
        certificate_source: n.child_text("CertificateSource").to_owned(),
    }
}

fn parse_restrictions(n: &Node) -> Restrictions {
    Restrictions {
        geo_restriction: n
            .child("GeoRestriction")
            .map(|g| GeoRestriction {
                restriction_type: g.child_text("RestrictionType").to_owned(),
                locations: g.direct_string_items("Location"),
            })
            .unwrap_or_default(),
    }
}

/// Parse `<InvalidationBatch>`.
pub fn parse_invalidation_batch(n: &Node) -> InvalidationBatch {
    InvalidationBatch {
        paths: n.string_items("Paths", "Path"),
        caller_reference: n.child_text("CallerReference").to_owned(),
    }
}

/// Parse `<OriginAccessControlConfig>`.
pub fn parse_oac_config(n: &Node) -> OriginAccessControlConfig {
    OriginAccessControlConfig {
        name: n.child_text("Name").to_owned(),
        description: n.child_text("Description").to_owned(),
        signing_protocol: {
            let s = n.child_text("SigningProtocol");
            if s.is_empty() {
                "sigv4".to_owned()
            } else {
                s.to_owned()
            }
        },
        signing_behavior: {
            let s = n.child_text("SigningBehavior");
            if s.is_empty() {
                "always".to_owned()
            } else {
                s.to_owned()
            }
        },
        origin_access_control_origin_type: {
            let s = n.child_text("OriginAccessControlOriginType");
            if s.is_empty() {
                "s3".to_owned()
            } else {
                s.to_owned()
            }
        },
    }
}

/// Parse `<CloudFrontOriginAccessIdentityConfig>`.
pub fn parse_oai_config(n: &Node) -> CloudFrontOriginAccessIdentityConfig {
    CloudFrontOriginAccessIdentityConfig {
        caller_reference: n.child_text("CallerReference").to_owned(),
        comment: n.child_text("Comment").to_owned(),
    }
}

/// Parse `<CachePolicyConfig>`.
pub fn parse_cache_policy_config(n: &Node) -> CachePolicyConfig {
    let params = n.child("ParametersInCacheKeyAndForwardedToOrigin");
    CachePolicyConfig {
        comment: n.child_text("Comment").to_owned(),
        name: n.child_text("Name").to_owned(),
        default_ttl: n.child_i64("DefaultTTL"),
        max_ttl: n.child_i64("MaxTTL"),
        min_ttl: n.child_i64("MinTTL"),
        parameters_in_cache_key_and_forwarded_to_origin: ParamsInCacheKey {
            enable_accept_encoding_gzip: params
                .map(|p| p.child_bool("EnableAcceptEncodingGzip"))
                .unwrap_or(false),
            enable_accept_encoding_brotli: params
                .map(|p| p.child_bool("EnableAcceptEncodingBrotli"))
                .unwrap_or(false),
            headers_config: params
                .and_then(|p| p.child("HeadersConfig"))
                .map(|h| CachePolicyHeadersConfig {
                    header_behavior: h.child_text("HeaderBehavior").to_owned(),
                    headers: h.string_items("Headers", "Name"),
                })
                .unwrap_or_default(),
            cookies_config: params
                .and_then(|p| p.child("CookiesConfig"))
                .map(|c| CachePolicyCookiesConfig {
                    cookie_behavior: c.child_text("CookieBehavior").to_owned(),
                    cookies: c.string_items("Cookies", "Name"),
                })
                .unwrap_or_default(),
            query_strings_config: params
                .and_then(|p| p.child("QueryStringsConfig"))
                .map(|q| CachePolicyQueryStringsConfig {
                    query_string_behavior: q.child_text("QueryStringBehavior").to_owned(),
                    query_strings: q.string_items("QueryStrings", "Name"),
                })
                .unwrap_or_default(),
        },
    }
}

/// Parse `<OriginRequestPolicyConfig>`.
pub fn parse_origin_request_policy_config(n: &Node) -> OriginRequestPolicyConfig {
    OriginRequestPolicyConfig {
        comment: n.child_text("Comment").to_owned(),
        name: n.child_text("Name").to_owned(),
        headers_config: n
            .child("HeadersConfig")
            .map(|h| OriginRequestPolicyHeadersConfig {
                header_behavior: h.child_text("HeaderBehavior").to_owned(),
                headers: h.string_items("Headers", "Name"),
            })
            .unwrap_or_default(),
        cookies_config: n
            .child("CookiesConfig")
            .map(|c| OriginRequestPolicyCookiesConfig {
                cookie_behavior: c.child_text("CookieBehavior").to_owned(),
                cookies: c.string_items("Cookies", "Name"),
            })
            .unwrap_or_default(),
        query_strings_config: n
            .child("QueryStringsConfig")
            .map(|q| OriginRequestPolicyQueryStringsConfig {
                query_string_behavior: q.child_text("QueryStringBehavior").to_owned(),
                query_strings: q.string_items("QueryStrings", "Name"),
            })
            .unwrap_or_default(),
    }
}

/// Parse `<ResponseHeadersPolicyConfig>` (minimal fields only).
pub fn parse_response_headers_policy_config(n: &Node) -> ResponseHeadersPolicyConfig {
    ResponseHeadersPolicyConfig {
        comment: n.child_text("Comment").to_owned(),
        name: n.child_text("Name").to_owned(),
        ..Default::default()
    }
}

/// Parse `<KeyGroupConfig>`.
pub fn parse_key_group_config(n: &Node) -> KeyGroupConfig {
    KeyGroupConfig {
        name: n.child_text("Name").to_owned(),
        items: n.direct_string_items("PublicKey"),
        comment: n.child_text("Comment").to_owned(),
    }
}

/// Parse `<PublicKeyConfig>`.
pub fn parse_public_key_config(n: &Node) -> PublicKeyConfig {
    PublicKeyConfig {
        caller_reference: n.child_text("CallerReference").to_owned(),
        name: n.child_text("Name").to_owned(),
        encoded_key: n.child_text("EncodedKey").to_owned(),
        comment: n.child_text("Comment").to_owned(),
    }
}

/// Parse `<FunctionConfig>`.
pub fn parse_function_config(n: &Node) -> FunctionConfig {
    FunctionConfig {
        comment: n.child_text("Comment").to_owned(),
        runtime: n.child_text("Runtime").to_owned(),
        key_value_store_associations: n
            .child("KeyValueStoreAssociations")
            .map(|k| k.direct_string_items("KeyValueStoreAssociation"))
            .unwrap_or_default(),
    }
}

/// Parse `<FieldLevelEncryptionConfig>`.
pub fn parse_fle_config(n: &Node) -> FieldLevelEncryptionConfig {
    FieldLevelEncryptionConfig {
        caller_reference: n.child_text("CallerReference").to_owned(),
        comment: n.child_text("Comment").to_owned(),
        query_arg_profile_config_enabled: n
            .child("QueryArgProfileConfig")
            .map(|_| true)
            .unwrap_or(false),
        content_type_profile_config_enabled: n
            .child("ContentTypeProfileConfig")
            .map(|_| true)
            .unwrap_or(false),
    }
}

/// Parse `<FieldLevelEncryptionProfileConfig>`.
pub fn parse_fle_profile_config(n: &Node) -> FieldLevelEncryptionProfileConfig {
    FieldLevelEncryptionProfileConfig {
        name: n.child_text("Name").to_owned(),
        caller_reference: n.child_text("CallerReference").to_owned(),
        comment: n.child_text("Comment").to_owned(),
    }
}

/// Parse `<RealtimeLogConfig>` at creation.
pub fn parse_realtime_log_config(n: &Node) -> RealtimeLogConfig {
    RealtimeLogConfig {
        arn: n.child_text("ARN").to_owned(),
        name: n.child_text("Name").to_owned(),
        sampling_rate: n.child_i64("SamplingRate"),
        end_points: Vec::new(),
        fields: n.string_items("Fields", "Field"),
    }
}

/// Parse a tagging payload: `<Tagging><TagSet><Tag>...</Tag></TagSet></Tagging>`.
pub fn parse_tag_set(n: &Node) -> TagSet {
    n.child("TagSet")
        .or_else(|| n.child("Tags"))
        .map(|ts| {
            ts.children_named("Tag")
                .map(|t| Tag {
                    key: t.child_text("Key").to_owned(),
                    value: t.child_text("Value").to_owned(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a list of tag keys: `<TagKeys><Key>foo</Key></TagKeys>`.
pub fn parse_tag_keys(n: &Node) -> Vec<String> {
    n.children_named("Key").map(|k| k.text.clone()).collect()
}
