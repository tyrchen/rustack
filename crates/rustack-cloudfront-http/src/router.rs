//! Route CloudFront REST paths to operation identifiers.
//!
//! CloudFront uses a 2020-05-31 path-versioned REST API, disambiguated by
//! HTTP method + path + optional query flag. The table here covers every
//! operation shipped across all phases.

use rustack_cloudfront_model::CloudFrontError;

/// An identified CloudFront operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch {
    /// Operation name (matching the Smithy operation name).
    pub operation: Operation,
    /// Path-bound parameters extracted by the router.
    pub path_params: PathParams,
}

/// Path parameters extracted from the URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathParams {
    /// Resource ID parameter (distribution, OAC, policy, etc.).
    pub id: String,
    /// Secondary resource ID (e.g. invalidation ID within a distribution).
    pub secondary_id: String,
    /// Resource name parameter (e.g. function name).
    pub name: String,
}

/// Identified operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Operation {
    // Distribution
    CreateDistribution,
    CreateDistributionWithTags,
    GetDistribution,
    GetDistributionConfig,
    UpdateDistribution,
    DeleteDistribution,
    ListDistributions,
    CopyDistribution,
    // Invalidation
    CreateInvalidation,
    GetInvalidation,
    ListInvalidations,
    // OAC
    CreateOriginAccessControl,
    GetOriginAccessControl,
    GetOriginAccessControlConfig,
    UpdateOriginAccessControl,
    DeleteOriginAccessControl,
    ListOriginAccessControls,
    // OAI
    CreateCloudFrontOriginAccessIdentity,
    GetCloudFrontOriginAccessIdentity,
    GetCloudFrontOriginAccessIdentityConfig,
    UpdateCloudFrontOriginAccessIdentity,
    DeleteCloudFrontOriginAccessIdentity,
    ListCloudFrontOriginAccessIdentities,
    // Cache policy
    CreateCachePolicy,
    GetCachePolicy,
    GetCachePolicyConfig,
    UpdateCachePolicy,
    DeleteCachePolicy,
    ListCachePolicies,
    // Origin request policy
    CreateOriginRequestPolicy,
    GetOriginRequestPolicy,
    GetOriginRequestPolicyConfig,
    UpdateOriginRequestPolicy,
    DeleteOriginRequestPolicy,
    ListOriginRequestPolicies,
    // Response headers policy
    CreateResponseHeadersPolicy,
    GetResponseHeadersPolicy,
    GetResponseHeadersPolicyConfig,
    UpdateResponseHeadersPolicy,
    DeleteResponseHeadersPolicy,
    ListResponseHeadersPolicies,
    // Key group
    CreateKeyGroup,
    GetKeyGroup,
    GetKeyGroupConfig,
    UpdateKeyGroup,
    DeleteKeyGroup,
    ListKeyGroups,
    // Public key
    CreatePublicKey,
    GetPublicKey,
    GetPublicKeyConfig,
    UpdatePublicKey,
    DeletePublicKey,
    ListPublicKeys,
    // Functions
    CreateFunction,
    DescribeFunction,
    GetFunction,
    UpdateFunction,
    DeleteFunction,
    PublishFunction,
    TestFunction,
    ListFunctions,
    // FLE
    CreateFieldLevelEncryptionConfig,
    GetFieldLevelEncryption,
    GetFieldLevelEncryptionConfig,
    UpdateFieldLevelEncryptionConfig,
    DeleteFieldLevelEncryptionConfig,
    ListFieldLevelEncryptionConfigs,
    CreateFieldLevelEncryptionProfile,
    GetFieldLevelEncryptionProfile,
    GetFieldLevelEncryptionProfileConfig,
    UpdateFieldLevelEncryptionProfile,
    DeleteFieldLevelEncryptionProfile,
    ListFieldLevelEncryptionProfiles,
    // Monitoring subscription
    CreateMonitoringSubscription,
    GetMonitoringSubscription,
    DeleteMonitoringSubscription,
    // KVS
    CreateKeyValueStore,
    DescribeKeyValueStore,
    UpdateKeyValueStore,
    DeleteKeyValueStore,
    ListKeyValueStores,
    // Realtime log
    CreateRealtimeLogConfig,
    GetRealtimeLogConfig,
    UpdateRealtimeLogConfig,
    DeleteRealtimeLogConfig,
    ListRealtimeLogConfigs,
    // Tagging
    TagResource,
    UntagResource,
    ListTagsForResource,
    // Phase 4 stubs
    AssociateAlias,
    ListConflictingAliases,
    UpdateDistributionWithStagingConfig,
    GetResourcePolicy,
    PutResourcePolicy,
    DeleteResourcePolicy,
    GetManagedCertificateDetails,
    VerifyDnsConfiguration,
    ListDistributionsByCachePolicyId,
    ListDistributionsByKeyGroup,
    ListDistributionsByOriginRequestPolicyId,
    ListDistributionsByRealtimeLogConfig,
    ListDistributionsByResponseHeadersPolicyId,
    ListDistributionsByVpcOriginId,
    ListDistributionsByWebACLId,
    ListDistributionsByAnycastIpListId,
    AssociateDistributionWebACL,
    DisassociateDistributionWebACL,
    CreateContinuousDeploymentPolicy,
    GetContinuousDeploymentPolicy,
    GetContinuousDeploymentPolicyConfig,
    UpdateContinuousDeploymentPolicy,
    DeleteContinuousDeploymentPolicy,
    ListContinuousDeploymentPolicies,
    CreateStreamingDistribution,
    CreateStreamingDistributionWithTags,
    GetStreamingDistribution,
    GetStreamingDistributionConfig,
    UpdateStreamingDistribution,
    DeleteStreamingDistribution,
    ListStreamingDistributions,
    CreateAnycastIpList,
    GetAnycastIpList,
    UpdateAnycastIpList,
    DeleteAnycastIpList,
    ListAnycastIpLists,
    CreateVpcOrigin,
    GetVpcOrigin,
    UpdateVpcOrigin,
    DeleteVpcOrigin,
    ListVpcOrigins,
    CreateTrustStore,
    GetTrustStore,
    UpdateTrustStore,
    DeleteTrustStore,
    ListTrustStores,
    ListDomainConflicts,
    UpdateDomainAssociation,
}

/// Resolve an HTTP request to a CloudFront operation.
#[allow(clippy::too_many_lines)]
pub fn resolve(method: &http::Method, uri: &http::Uri) -> Result<RouteMatch, CloudFrontError> {
    let path = uri.path();
    let query = uri.query().unwrap_or("");
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();

    // Every CloudFront URL begins with the API version.
    if segments.first().copied() != Some("2020-05-31") {
        return Err(CloudFrontError::InvalidArgument(format!(
            "Unknown path {path}"
        )));
    }
    let rest: Vec<&str> = segments.iter().skip(1).copied().collect();

    use http::Method;
    let m = method.clone();

    let op = match rest.as_slice() {
        // ----- Tagging -----
        ["tagging"] if m == Method::POST => {
            if query_has(query, "Operation=Tag") {
                Operation::TagResource
            } else if query_has(query, "Operation=Untag") {
                Operation::UntagResource
            } else {
                return Err(CloudFrontError::InvalidArgument(
                    "tagging requires Operation=Tag or Operation=Untag".into(),
                ));
            }
        }
        ["tagging"] if m == Method::GET => Operation::ListTagsForResource,

        // ----- Distribution -----
        ["distribution"] if m == Method::POST => {
            if query_has(query, "WithTags") {
                Operation::CreateDistributionWithTags
            } else {
                Operation::CreateDistribution
            }
        }
        ["distribution"] if m == Method::GET => Operation::ListDistributions,
        ["distribution", id] if m == Method::GET => {
            return ok_with_id(Operation::GetDistribution, id);
        }
        ["distribution", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteDistribution, id);
        }
        ["distribution", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetDistributionConfig, id);
        }
        ["distribution", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateDistribution, id);
        }
        ["distribution", id, "copy"] if m == Method::POST => {
            return ok_with_id(Operation::CopyDistribution, id);
        }
        ["distribution", id, "promote-staging-config"] if m == Method::POST => {
            return ok_with_id(Operation::UpdateDistributionWithStagingConfig, id);
        }
        ["distribution", id, "invalidation"] if m == Method::POST => {
            return ok_with_id(Operation::CreateInvalidation, id);
        }
        ["distribution", id, "invalidation"] if m == Method::GET => {
            return ok_with_id(Operation::ListInvalidations, id);
        }
        ["distribution", id, "invalidation", inv_id] if m == Method::GET => {
            return ok_with_two(Operation::GetInvalidation, id, inv_id);
        }

        // ----- OAC -----
        ["origin-access-control"] if m == Method::POST => Operation::CreateOriginAccessControl,
        ["origin-access-control"] if m == Method::GET => Operation::ListOriginAccessControls,
        ["origin-access-control", id] if m == Method::GET => {
            return ok_with_id(Operation::GetOriginAccessControl, id);
        }
        ["origin-access-control", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteOriginAccessControl, id);
        }
        ["origin-access-control", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetOriginAccessControlConfig, id);
        }
        ["origin-access-control", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateOriginAccessControl, id);
        }

        // ----- OAI -----
        ["origin-access-identity", "cloudfront"] if m == Method::POST => {
            Operation::CreateCloudFrontOriginAccessIdentity
        }
        ["origin-access-identity", "cloudfront"] if m == Method::GET => {
            Operation::ListCloudFrontOriginAccessIdentities
        }
        ["origin-access-identity", "cloudfront", id] if m == Method::GET => {
            return ok_with_id(Operation::GetCloudFrontOriginAccessIdentity, id);
        }
        ["origin-access-identity", "cloudfront", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteCloudFrontOriginAccessIdentity, id);
        }
        ["origin-access-identity", "cloudfront", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetCloudFrontOriginAccessIdentityConfig, id);
        }
        ["origin-access-identity", "cloudfront", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateCloudFrontOriginAccessIdentity, id);
        }

        // ----- Policies: cache-policy -----
        ["cache-policy"] if m == Method::POST => Operation::CreateCachePolicy,
        ["cache-policy"] if m == Method::GET => Operation::ListCachePolicies,
        ["cache-policy", id] if m == Method::GET => {
            return ok_with_id(Operation::GetCachePolicy, id);
        }
        ["cache-policy", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteCachePolicy, id);
        }
        ["cache-policy", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetCachePolicyConfig, id);
        }
        ["cache-policy", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateCachePolicy, id);
        }

        // ----- origin-request-policy -----
        ["origin-request-policy"] if m == Method::POST => Operation::CreateOriginRequestPolicy,
        ["origin-request-policy"] if m == Method::GET => Operation::ListOriginRequestPolicies,
        ["origin-request-policy", id] if m == Method::GET => {
            return ok_with_id(Operation::GetOriginRequestPolicy, id);
        }
        ["origin-request-policy", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteOriginRequestPolicy, id);
        }
        ["origin-request-policy", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetOriginRequestPolicyConfig, id);
        }
        ["origin-request-policy", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateOriginRequestPolicy, id);
        }

        // ----- response-headers-policy -----
        ["response-headers-policy"] if m == Method::POST => Operation::CreateResponseHeadersPolicy,
        ["response-headers-policy"] if m == Method::GET => Operation::ListResponseHeadersPolicies,
        ["response-headers-policy", id] if m == Method::GET => {
            return ok_with_id(Operation::GetResponseHeadersPolicy, id);
        }
        ["response-headers-policy", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteResponseHeadersPolicy, id);
        }
        ["response-headers-policy", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetResponseHeadersPolicyConfig, id);
        }
        ["response-headers-policy", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateResponseHeadersPolicy, id);
        }

        // ----- key-group -----
        ["key-group"] if m == Method::POST => Operation::CreateKeyGroup,
        ["key-group"] if m == Method::GET => Operation::ListKeyGroups,
        ["key-group", id] if m == Method::GET => {
            return ok_with_id(Operation::GetKeyGroup, id);
        }
        ["key-group", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteKeyGroup, id);
        }
        ["key-group", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetKeyGroupConfig, id);
        }
        ["key-group", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateKeyGroup, id);
        }

        // ----- public-key -----
        ["public-key"] if m == Method::POST => Operation::CreatePublicKey,
        ["public-key"] if m == Method::GET => Operation::ListPublicKeys,
        ["public-key", id] if m == Method::GET => {
            return ok_with_id(Operation::GetPublicKey, id);
        }
        ["public-key", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeletePublicKey, id);
        }
        ["public-key", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetPublicKeyConfig, id);
        }
        ["public-key", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdatePublicKey, id);
        }

        // ----- function -----
        ["function"] if m == Method::POST => Operation::CreateFunction,
        ["function"] if m == Method::GET => Operation::ListFunctions,
        ["function", name] if m == Method::GET => {
            return ok_with_name(Operation::DescribeFunction, name);
        }
        ["function", name] if m == Method::DELETE => {
            return ok_with_name(Operation::DeleteFunction, name);
        }
        ["function", name] if m == Method::PUT => {
            return ok_with_name(Operation::UpdateFunction, name);
        }
        ["function", name, "publish"] if m == Method::POST => {
            return ok_with_name(Operation::PublishFunction, name);
        }
        ["function", name, "test"] if m == Method::POST => {
            return ok_with_name(Operation::TestFunction, name);
        }
        ["function", name, "development"] if m == Method::POST => {
            return ok_with_name(Operation::GetFunction, name);
        }

        // ----- FLE -----
        ["field-level-encryption"] if m == Method::POST => {
            Operation::CreateFieldLevelEncryptionConfig
        }
        ["field-level-encryption"] if m == Method::GET => {
            Operation::ListFieldLevelEncryptionConfigs
        }
        ["field-level-encryption", id] if m == Method::GET => {
            return ok_with_id(Operation::GetFieldLevelEncryption, id);
        }
        ["field-level-encryption", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteFieldLevelEncryptionConfig, id);
        }
        ["field-level-encryption", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetFieldLevelEncryptionConfig, id);
        }
        ["field-level-encryption", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateFieldLevelEncryptionConfig, id);
        }
        ["field-level-encryption-profile"] if m == Method::POST => {
            Operation::CreateFieldLevelEncryptionProfile
        }
        ["field-level-encryption-profile"] if m == Method::GET => {
            Operation::ListFieldLevelEncryptionProfiles
        }
        ["field-level-encryption-profile", id] if m == Method::GET => {
            return ok_with_id(Operation::GetFieldLevelEncryptionProfile, id);
        }
        ["field-level-encryption-profile", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteFieldLevelEncryptionProfile, id);
        }
        ["field-level-encryption-profile", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetFieldLevelEncryptionProfileConfig, id);
        }
        ["field-level-encryption-profile", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateFieldLevelEncryptionProfile, id);
        }

        // ----- Monitoring subscription -----
        ["distributions", dist_id, "monitoring-subscription"] if m == Method::POST => {
            return ok_with_id(Operation::CreateMonitoringSubscription, dist_id);
        }
        ["distributions", dist_id, "monitoring-subscription"] if m == Method::GET => {
            return ok_with_id(Operation::GetMonitoringSubscription, dist_id);
        }
        ["distributions", dist_id, "monitoring-subscription"] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteMonitoringSubscription, dist_id);
        }

        // ----- KVS -----
        ["key-value-store"] if m == Method::POST => Operation::CreateKeyValueStore,
        ["key-value-store"] if m == Method::GET => Operation::ListKeyValueStores,
        ["key-value-store", id] if m == Method::GET => {
            return ok_with_id(Operation::DescribeKeyValueStore, id);
        }
        ["key-value-store", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteKeyValueStore, id);
        }
        ["key-value-store", id] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateKeyValueStore, id);
        }

        // ----- Realtime log -----
        ["realtime-log-config"] if m == Method::POST => Operation::CreateRealtimeLogConfig,
        ["realtime-log-config"] if m == Method::GET => Operation::ListRealtimeLogConfigs,
        ["realtime-log-config"] if m == Method::PUT => Operation::UpdateRealtimeLogConfig,
        ["realtime-log-config"] if m == Method::DELETE => Operation::DeleteRealtimeLogConfig,
        ["get-realtime-log-config"] if m == Method::POST => Operation::GetRealtimeLogConfig,

        // ----- Phase 4 stubs: keep the shapes so SDKs don't fail -----
        ["distributions-by-cache-policy-id", id] if m == Method::GET => {
            return ok_with_id(Operation::ListDistributionsByCachePolicyId, id);
        }
        ["distributions-by-key-group", id] if m == Method::GET => {
            return ok_with_id(Operation::ListDistributionsByKeyGroup, id);
        }
        ["distributions-by-origin-request-policy-id", id] if m == Method::GET => {
            return ok_with_id(Operation::ListDistributionsByOriginRequestPolicyId, id);
        }
        ["distributions-by-realtime-log-config"] if m == Method::POST => {
            Operation::ListDistributionsByRealtimeLogConfig
        }
        ["distributions-by-response-headers-policy-id", id] if m == Method::GET => {
            return ok_with_id(Operation::ListDistributionsByResponseHeadersPolicyId, id);
        }
        ["distributions-by-vpc-origin-id", id] if m == Method::GET => {
            return ok_with_id(Operation::ListDistributionsByVpcOriginId, id);
        }
        ["distributions-by-web-acl-id", id] if m == Method::GET => {
            return ok_with_id(Operation::ListDistributionsByWebACLId, id);
        }
        ["distributions-by-anycast-ip-list-id", id] if m == Method::GET => {
            return ok_with_id(Operation::ListDistributionsByAnycastIpListId, id);
        }
        ["distribution", id, "associate-alias"] if m == Method::PUT => {
            return ok_with_id(Operation::AssociateAlias, id);
        }
        ["conflicting-alias"] if m == Method::GET => Operation::ListConflictingAliases,
        ["resource-policy", "arn"] if m == Method::GET => Operation::GetResourcePolicy,
        ["resource-policy", "arn"] if m == Method::PUT => Operation::PutResourcePolicy,
        ["resource-policy", "arn"] if m == Method::DELETE => Operation::DeleteResourcePolicy,
        ["managed-certificate-details"] if m == Method::GET => {
            Operation::GetManagedCertificateDetails
        }
        ["verify-dns-configuration"] if m == Method::POST => Operation::VerifyDnsConfiguration,
        ["continuous-deployment-policy"] if m == Method::POST => {
            Operation::CreateContinuousDeploymentPolicy
        }
        ["continuous-deployment-policy"] if m == Method::GET => {
            Operation::ListContinuousDeploymentPolicies
        }
        ["continuous-deployment-policy", id] if m == Method::GET => {
            return ok_with_id(Operation::GetContinuousDeploymentPolicy, id);
        }
        ["continuous-deployment-policy", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteContinuousDeploymentPolicy, id);
        }
        ["continuous-deployment-policy", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetContinuousDeploymentPolicyConfig, id);
        }
        ["continuous-deployment-policy", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateContinuousDeploymentPolicy, id);
        }
        ["streaming-distribution"] if m == Method::POST => {
            if query_has(query, "WithTags") {
                Operation::CreateStreamingDistributionWithTags
            } else {
                Operation::CreateStreamingDistribution
            }
        }
        ["streaming-distribution"] if m == Method::GET => Operation::ListStreamingDistributions,
        ["streaming-distribution", id] if m == Method::GET => {
            return ok_with_id(Operation::GetStreamingDistribution, id);
        }
        ["streaming-distribution", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteStreamingDistribution, id);
        }
        ["streaming-distribution", id, "config"] if m == Method::GET => {
            return ok_with_id(Operation::GetStreamingDistributionConfig, id);
        }
        ["streaming-distribution", id, "config"] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateStreamingDistribution, id);
        }
        ["anycast-ip-list"] if m == Method::POST => Operation::CreateAnycastIpList,
        ["anycast-ip-list"] if m == Method::GET => Operation::ListAnycastIpLists,
        ["anycast-ip-list", id] if m == Method::GET => {
            return ok_with_id(Operation::GetAnycastIpList, id);
        }
        ["anycast-ip-list", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteAnycastIpList, id);
        }
        ["anycast-ip-list", id] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateAnycastIpList, id);
        }
        ["vpc-origin"] if m == Method::POST => Operation::CreateVpcOrigin,
        ["vpc-origin"] if m == Method::GET => Operation::ListVpcOrigins,
        ["vpc-origin", id] if m == Method::GET => {
            return ok_with_id(Operation::GetVpcOrigin, id);
        }
        ["vpc-origin", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteVpcOrigin, id);
        }
        ["vpc-origin", id] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateVpcOrigin, id);
        }
        ["truststore"] if m == Method::POST => Operation::CreateTrustStore,
        ["truststore"] if m == Method::GET => Operation::ListTrustStores,
        ["truststore", id] if m == Method::GET => {
            return ok_with_id(Operation::GetTrustStore, id);
        }
        ["truststore", id] if m == Method::DELETE => {
            return ok_with_id(Operation::DeleteTrustStore, id);
        }
        ["truststore", id] if m == Method::PUT => {
            return ok_with_id(Operation::UpdateTrustStore, id);
        }
        ["domain-conflicts"] if m == Method::POST => Operation::ListDomainConflicts,
        ["domain-association"] if m == Method::POST => Operation::UpdateDomainAssociation,
        ["distribution", id, "web-acl"] if m == Method::PUT => {
            return ok_with_id(Operation::AssociateDistributionWebACL, id);
        }
        ["distribution", id, "web-acl"] if m == Method::DELETE => {
            return ok_with_id(Operation::DisassociateDistributionWebACL, id);
        }

        _ => {
            return Err(CloudFrontError::InvalidArgument(format!(
                "No route matched {method} {path}"
            )));
        }
    };

    Ok(RouteMatch {
        operation: op,
        path_params: PathParams::default(),
    })
}

fn ok_with_id(op: Operation, id: &str) -> Result<RouteMatch, CloudFrontError> {
    Ok(RouteMatch {
        operation: op,
        path_params: PathParams {
            id: id.to_owned(),
            ..PathParams::default()
        },
    })
}

fn ok_with_two(op: Operation, id: &str, sec: &str) -> Result<RouteMatch, CloudFrontError> {
    Ok(RouteMatch {
        operation: op,
        path_params: PathParams {
            id: id.to_owned(),
            secondary_id: sec.to_owned(),
            ..PathParams::default()
        },
    })
}

fn ok_with_name(op: Operation, name: &str) -> Result<RouteMatch, CloudFrontError> {
    Ok(RouteMatch {
        operation: op,
        path_params: PathParams {
            name: name.to_owned(),
            ..PathParams::default()
        },
    })
}

fn query_has(q: &str, key: &str) -> bool {
    q.split('&')
        .any(|kv| kv == key || kv.starts_with(&format!("{key}=")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_url(method: &str, uri: &str) -> RouteMatch {
        let m = http::Method::from_bytes(method.as_bytes()).unwrap();
        let u: http::Uri = uri.parse().unwrap();
        resolve(&m, &u).expect("route")
    }

    #[test]
    fn test_create_distribution() {
        let r = parse_url("POST", "/2020-05-31/distribution");
        assert_eq!(r.operation, Operation::CreateDistribution);
    }

    #[test]
    fn test_create_distribution_with_tags() {
        let r = parse_url("POST", "/2020-05-31/distribution?WithTags");
        assert_eq!(r.operation, Operation::CreateDistributionWithTags);
    }

    #[test]
    fn test_update_distribution() {
        let r = parse_url("PUT", "/2020-05-31/distribution/E1ABC/config");
        assert_eq!(r.operation, Operation::UpdateDistribution);
        assert_eq!(r.path_params.id, "E1ABC");
    }

    #[test]
    fn test_get_invalidation() {
        let r = parse_url("GET", "/2020-05-31/distribution/E1ABC/invalidation/I42");
        assert_eq!(r.operation, Operation::GetInvalidation);
        assert_eq!(r.path_params.id, "E1ABC");
        assert_eq!(r.path_params.secondary_id, "I42");
    }

    #[test]
    fn test_tag_resource() {
        let r = parse_url(
            "POST",
            "/2020-05-31/tagging?Operation=Tag&Resource=arn:aws:cloudfront::000:dist/E1",
        );
        assert_eq!(r.operation, Operation::TagResource);
    }
}
