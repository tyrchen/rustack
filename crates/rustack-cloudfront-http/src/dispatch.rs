//! Dispatch identified operations to the provider.
//!
//! This is the "business" layer of the HTTP crate: it receives a
//! `RouteMatch`, parses the request body / query, calls into the
//! `RustackCloudFront` provider, and turns the domain result into an HTTP
//! response with correct status codes and ETag/Location headers.

use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, Response, StatusCode, Uri};
use rustack_cloudfront_core::RustackCloudFront;
use rustack_cloudfront_model::{
    CloudFrontError, CloudFrontOriginAccessIdentityConfig, FunctionConfig, KeyValueStore,
};

use crate::{
    request::{
        parse_cache_policy_config, parse_distribution_config, parse_fle_config,
        parse_fle_profile_config, parse_function_config, parse_invalidation_batch,
        parse_key_group_config, parse_oac_config, parse_oai_config,
        parse_origin_request_policy_config, parse_public_key_config, parse_realtime_log_config,
        parse_response_headers_policy_config, parse_root, parse_tag_keys, parse_tag_set,
    },
    response::{empty_204, error_response, xml_response},
    router::{Operation, PathParams, RouteMatch},
    service::HttpBody,
    xml::ser,
};

/// Handler trait — exists for symmetry with other services. The built-in
/// implementation dispatches directly to `RustackCloudFront`.
pub trait CloudFrontHandler: Send + Sync + 'static {
    /// Get the underlying provider as an owned `Arc`.
    ///
    /// `create_distribution`, `copy_distribution`, `update_distribution`,
    /// and `create_invalidation` take `self: &Arc<Self>` so they can spawn
    /// propagation tasks that outlive the caller. The service layer stores
    /// an `Arc<H>`; this method converts that into the owning `Arc<RustackCloudFront>`
    /// needed by the provider.
    fn provider_arc(&self) -> Arc<RustackCloudFront>;
}

/// The canonical handler: an `Arc<RustackCloudFront>` wrapped in the service's
/// outer `Arc`. The double-Arc is required because the hyper service owns the
/// handler by `Arc<H>` while the provider's methods need `Arc<RustackCloudFront>`.
impl CloudFrontHandler for Arc<RustackCloudFront> {
    fn provider_arc(&self) -> Arc<RustackCloudFront> {
        Arc::clone(self)
    }
}

/// Dispatch a parsed route to the provider and produce an HTTP response.
#[allow(clippy::too_many_lines)]
pub async fn dispatch(
    handler: &dyn CloudFrontHandler,
    route: RouteMatch,
    uri: &Uri,
    _headers: &HeaderMap,
    if_match: Option<&str>,
    body: Bytes,
    request_id: &str,
) -> Response<HttpBody> {
    let provider = handler.provider_arc();
    match handle(&provider, route, uri, if_match, body).await {
        Ok(resp) => resp,
        Err(err) => error_response(&err, request_id),
    }
}

async fn handle(
    provider: &Arc<RustackCloudFront>,
    route: RouteMatch,
    uri: &Uri,
    if_match: Option<&str>,
    body: Bytes,
) -> Result<Response<HttpBody>, CloudFrontError> {
    let p = &route.path_params;
    match route.operation {
        // Distribution
        Operation::CreateDistribution | Operation::CreateDistributionWithTags => {
            let root = parse_root(&body)?;
            let (config_node, tags) = if root.name == "DistributionConfigWithTags" {
                let cn = root.child("DistributionConfig").ok_or_else(|| {
                    CloudFrontError::MalformedInput(
                        "DistributionConfigWithTags missing DistributionConfig".into(),
                    )
                })?;
                let tags = root.child("Tags").map(parse_tag_set).unwrap_or_default();
                (cn.clone(), tags)
            } else {
                (root.clone(), Vec::new())
            };
            let cfg = parse_distribution_config(&config_node);
            let dist = provider.create_distribution(cfg, tags)?;
            let body = ser::distribution_xml(&dist);
            let mut resp = xml_response(StatusCode::CREATED, body, Some(&dist.etag));
            let loc = format!(
                "https://cloudfront.amazonaws.com/2020-05-31/distribution/{}",
                dist.id
            );
            if let Ok(hv) = HeaderValue::from_str(&loc) {
                resp.headers_mut().insert(http::header::LOCATION, hv);
            }
            Ok(resp)
        }
        Operation::GetDistribution => {
            let d = provider.get_distribution(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::distribution_xml(&d),
                Some(&d.etag),
            ))
        }
        Operation::GetDistributionConfig => {
            let d = provider.get_distribution(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::distribution_config_xml(&d.config),
                Some(&d.etag),
            ))
        }
        Operation::UpdateDistribution => {
            let root = parse_root(&body)?;
            let cfg = parse_distribution_config(&root);
            let d = provider.update_distribution(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::distribution_xml(&d),
                Some(&d.etag),
            ))
        }
        Operation::DeleteDistribution => {
            provider.delete_distribution(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListDistributions => {
            let items = provider.list_distributions();
            let max = query_max_items(uri);
            Ok(xml_response(
                StatusCode::OK,
                ser::distribution_list_xml(&items, max),
                None,
            ))
        }
        Operation::CopyDistribution => {
            let root = parse_root(&body)?;
            let caller_ref = root.child_text("CallerReference").to_owned();
            let staging = root.child_bool("Staging");
            let d = provider.copy_distribution(&p.id, &caller_ref, staging)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::distribution_xml(&d),
                Some(&d.etag),
            ))
        }

        // Invalidation
        Operation::CreateInvalidation => {
            let root = parse_root(&body)?;
            let batch = parse_invalidation_batch(&root);
            let inv = provider.create_invalidation(&p.id, batch)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::invalidation_xml(&inv),
                None,
            ))
        }
        Operation::GetInvalidation => {
            let inv = provider.get_invalidation(&p.id, &p.secondary_id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::invalidation_xml(&inv),
                None,
            ))
        }
        Operation::ListInvalidations => {
            let items = provider.list_invalidations(&p.id);
            let max = query_max_items(uri);
            Ok(xml_response(
                StatusCode::OK,
                ser::invalidation_list_xml(&items, max),
                None,
            ))
        }

        // OAC
        Operation::CreateOriginAccessControl => {
            let root = parse_root(&body)?;
            let cfg = parse_oac_config(&root);
            let o = provider.create_oac(cfg)?;
            let xml = ser::oac_xml(&o);
            let mut resp = xml_response(StatusCode::CREATED, xml, Some(&o.etag));
            let loc = format!(
                "https://cloudfront.amazonaws.com/2020-05-31/origin-access-control/{}",
                o.id
            );
            if let Ok(hv) = HeaderValue::from_str(&loc) {
                resp.headers_mut().insert(http::header::LOCATION, hv);
            }
            Ok(resp)
        }
        Operation::GetOriginAccessControl => {
            let o = provider.get_oac(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::oac_xml(&o),
                Some(&o.etag),
            ))
        }
        Operation::GetOriginAccessControlConfig => {
            let o = provider.get_oac(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::oac_config_xml(&o.config),
                Some(&o.etag),
            ))
        }
        Operation::UpdateOriginAccessControl => {
            let root = parse_root(&body)?;
            let cfg = parse_oac_config(&root);
            let o = provider.update_oac(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::oac_xml(&o),
                Some(&o.etag),
            ))
        }
        Operation::DeleteOriginAccessControl => {
            provider.delete_oac(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListOriginAccessControls => {
            let items = provider.list_oacs();
            let max = query_max_items(uri);
            Ok(xml_response(
                StatusCode::OK,
                ser::oac_list_xml(&items, max),
                None,
            ))
        }

        // OAI
        Operation::CreateCloudFrontOriginAccessIdentity => {
            let root = parse_root(&body)?;
            let cfg: CloudFrontOriginAccessIdentityConfig = parse_oai_config(&root);
            let o = provider.create_oai(cfg)?;
            let mut resp = xml_response(StatusCode::CREATED, ser::oai_xml(&o), Some(&o.etag));
            let loc = format!(
                "https://cloudfront.amazonaws.com/2020-05-31/origin-access-identity/cloudfront/{}",
                o.id
            );
            if let Ok(hv) = HeaderValue::from_str(&loc) {
                resp.headers_mut().insert(http::header::LOCATION, hv);
            }
            Ok(resp)
        }
        Operation::GetCloudFrontOriginAccessIdentity => {
            let o = provider.get_oai(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::oai_xml(&o),
                Some(&o.etag),
            ))
        }
        Operation::GetCloudFrontOriginAccessIdentityConfig => {
            let o = provider.get_oai(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::oai_config_xml(&o.config),
                Some(&o.etag),
            ))
        }
        Operation::UpdateCloudFrontOriginAccessIdentity => {
            let root = parse_root(&body)?;
            let cfg = parse_oai_config(&root);
            let o = provider.update_oai(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::oai_xml(&o),
                Some(&o.etag),
            ))
        }
        Operation::DeleteCloudFrontOriginAccessIdentity => {
            provider.delete_oai(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListCloudFrontOriginAccessIdentities => {
            let items = provider.list_oais();
            Ok(xml_response(
                StatusCode::OK,
                ser::oai_list_xml(&items, query_max_items(uri)),
                None,
            ))
        }

        // Cache policies
        Operation::CreateCachePolicy => {
            let root = parse_root(&body)?;
            let cfg = parse_cache_policy_config(&root);
            let p = provider.create_cache_policy(cfg)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::cache_policy_xml(&p),
                Some(&p.etag),
            ))
        }
        Operation::GetCachePolicy => {
            let pol = provider.get_cache_policy(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::cache_policy_xml(&pol),
                Some(&pol.etag),
            ))
        }
        Operation::GetCachePolicyConfig => {
            let pol = provider.get_cache_policy(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::cache_policy_config_xml(&pol.config),
                Some(&pol.etag),
            ))
        }
        Operation::UpdateCachePolicy => {
            let root = parse_root(&body)?;
            let cfg = parse_cache_policy_config(&root);
            let pol = provider.update_cache_policy(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::cache_policy_xml(&pol),
                Some(&pol.etag),
            ))
        }
        Operation::DeleteCachePolicy => {
            provider.delete_cache_policy(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListCachePolicies => {
            let items = provider.list_cache_policies();
            Ok(xml_response(
                StatusCode::OK,
                ser::cache_policy_list_xml(&items, query_max_items(uri)),
                None,
            ))
        }

        // Origin request policy
        Operation::CreateOriginRequestPolicy => {
            let root = parse_root(&body)?;
            let cfg = parse_origin_request_policy_config(&root);
            let pol = provider.create_origin_request_policy(cfg)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::origin_request_policy_xml(&pol),
                Some(&pol.etag),
            ))
        }
        Operation::GetOriginRequestPolicy => {
            let pol = provider.get_origin_request_policy(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::origin_request_policy_xml(&pol),
                Some(&pol.etag),
            ))
        }
        Operation::GetOriginRequestPolicyConfig => {
            let pol = provider.get_origin_request_policy(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::origin_request_policy_config_xml(&pol.config),
                Some(&pol.etag),
            ))
        }
        Operation::UpdateOriginRequestPolicy => {
            let root = parse_root(&body)?;
            let cfg = parse_origin_request_policy_config(&root);
            let pol = provider.update_origin_request_policy(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::origin_request_policy_xml(&pol),
                Some(&pol.etag),
            ))
        }
        Operation::DeleteOriginRequestPolicy => {
            provider.delete_origin_request_policy(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListOriginRequestPolicies => {
            let items = provider.list_origin_request_policies();
            Ok(xml_response(
                StatusCode::OK,
                ser::origin_request_policy_list_xml(&items, query_max_items(uri)),
                None,
            ))
        }

        // Response headers policy
        Operation::CreateResponseHeadersPolicy => {
            let root = parse_root(&body)?;
            let cfg = parse_response_headers_policy_config(&root);
            let pol = provider.create_response_headers_policy(cfg)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::response_headers_policy_xml(&pol),
                Some(&pol.etag),
            ))
        }
        Operation::GetResponseHeadersPolicy => {
            let pol = provider.get_response_headers_policy(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::response_headers_policy_xml(&pol),
                Some(&pol.etag),
            ))
        }
        Operation::GetResponseHeadersPolicyConfig => {
            let pol = provider.get_response_headers_policy(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::response_headers_policy_config_xml(&pol.config),
                Some(&pol.etag),
            ))
        }
        Operation::UpdateResponseHeadersPolicy => {
            let root = parse_root(&body)?;
            let cfg = parse_response_headers_policy_config(&root);
            let pol = provider.update_response_headers_policy(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::response_headers_policy_xml(&pol),
                Some(&pol.etag),
            ))
        }
        Operation::DeleteResponseHeadersPolicy => {
            provider.delete_response_headers_policy(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListResponseHeadersPolicies => {
            let items = provider.list_response_headers_policies();
            Ok(xml_response(
                StatusCode::OK,
                ser::response_headers_policy_list_xml(&items, query_max_items(uri)),
                None,
            ))
        }

        // Key group
        Operation::CreateKeyGroup => {
            let root = parse_root(&body)?;
            let cfg = parse_key_group_config(&root);
            let kg = provider.create_key_group(cfg)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::key_group_xml(&kg),
                Some(&kg.etag),
            ))
        }
        Operation::GetKeyGroup | Operation::GetKeyGroupConfig => {
            let kg = provider.get_key_group(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::key_group_xml(&kg),
                Some(&kg.etag),
            ))
        }
        Operation::UpdateKeyGroup => {
            let root = parse_root(&body)?;
            let cfg = parse_key_group_config(&root);
            let kg = provider.update_key_group(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::key_group_xml(&kg),
                Some(&kg.etag),
            ))
        }
        Operation::DeleteKeyGroup => {
            provider.delete_key_group(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListKeyGroups => {
            let items = provider.list_key_groups();
            Ok(xml_response(
                StatusCode::OK,
                ser::key_group_list_xml(&items, query_max_items(uri)),
                None,
            ))
        }

        // Public key
        Operation::CreatePublicKey => {
            let root = parse_root(&body)?;
            let cfg = parse_public_key_config(&root);
            let k = provider.create_public_key(cfg)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::public_key_xml(&k),
                Some(&k.etag),
            ))
        }
        Operation::GetPublicKey | Operation::GetPublicKeyConfig => {
            let k = provider.get_public_key(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::public_key_xml(&k),
                Some(&k.etag),
            ))
        }
        Operation::UpdatePublicKey => {
            let root = parse_root(&body)?;
            let cfg = parse_public_key_config(&root);
            let k = provider.update_public_key(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::public_key_xml(&k),
                Some(&k.etag),
            ))
        }
        Operation::DeletePublicKey => {
            provider.delete_public_key(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListPublicKeys => {
            let items = provider.list_public_keys();
            Ok(xml_response(
                StatusCode::OK,
                ser::public_key_list_xml(&items, query_max_items(uri)),
                None,
            ))
        }

        // Functions
        Operation::CreateFunction => {
            let root = parse_root(&body)?;
            let name = root.child_text("Name").to_owned();
            let cfg_node = root.child("FunctionConfig");
            let cfg: FunctionConfig = cfg_node.map(parse_function_config).unwrap_or_default();
            let code = base64_decode_or_raw(root.child_text("FunctionCode"));
            let f = provider.create_function(name, cfg, code)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::function_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::DescribeFunction | Operation::GetFunction => {
            let f = provider.get_function(&route.path_params.name)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::function_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::UpdateFunction => {
            let root = parse_root(&body)?;
            let cfg = root
                .child("FunctionConfig")
                .map(parse_function_config)
                .unwrap_or_default();
            let code = base64_decode_or_raw(root.child_text("FunctionCode"));
            let f = provider.update_function(&route.path_params.name, if_match, cfg, code)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::function_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::DeleteFunction => {
            provider.delete_function(&route.path_params.name, if_match)?;
            Ok(empty_204())
        }
        Operation::PublishFunction => {
            let f = provider.publish_function(&route.path_params.name, if_match)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::function_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::TestFunction => {
            let root = parse_root(&body)?;
            let event = root.child_text("EventObject").as_bytes().to_vec();
            let (result, util) = provider.test_function(&route.path_params.name, &event)?;
            let s = String::from_utf8_lossy(&result);
            let mut w = crate::xml::XmlWriter::new();
            w.declaration();
            w.open_root(
                "TestResult",
                Some(rustack_cloudfront_model::CLOUDFRONT_XML_NAMESPACE),
            );
            w.element("FunctionSummary", "");
            w.element("ComputeUtilization", &util);
            w.open("FunctionExecutionLogList");
            w.close("FunctionExecutionLogList");
            w.element("FunctionErrorMessage", "");
            w.element("FunctionOutput", &s);
            w.close("TestResult");
            Ok(xml_response(StatusCode::OK, w.finish(), None))
        }
        Operation::ListFunctions => {
            let items = provider.list_functions();
            Ok(xml_response(
                StatusCode::OK,
                ser::function_list_xml(&items, query_max_items(uri)),
                None,
            ))
        }

        // FLE
        Operation::CreateFieldLevelEncryptionConfig => {
            let root = parse_root(&body)?;
            let cfg = parse_fle_config(&root);
            let f = provider.create_fle_config(cfg)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::fle_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::GetFieldLevelEncryption | Operation::GetFieldLevelEncryptionConfig => {
            let f = provider.get_fle_config(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::fle_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::UpdateFieldLevelEncryptionConfig => {
            let root = parse_root(&body)?;
            let cfg = parse_fle_config(&root);
            let f = provider.update_fle_config(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::fle_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::DeleteFieldLevelEncryptionConfig => {
            provider.delete_fle_config(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListFieldLevelEncryptionConfigs => {
            let items = provider.list_fle_configs();
            let mut w = crate::xml::XmlWriter::new();
            w.declaration();
            w.open_root(
                "FieldLevelEncryptionList",
                Some(rustack_cloudfront_model::CLOUDFRONT_XML_NAMESPACE),
            );
            w.element_display("Quantity", items.len());
            w.close("FieldLevelEncryptionList");
            Ok(xml_response(StatusCode::OK, w.finish(), None))
        }
        Operation::CreateFieldLevelEncryptionProfile => {
            let root = parse_root(&body)?;
            let cfg = parse_fle_profile_config(&root);
            let f = provider.create_fle_profile(cfg)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::fle_profile_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::GetFieldLevelEncryptionProfile
        | Operation::GetFieldLevelEncryptionProfileConfig => {
            let f = provider.get_fle_profile(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::fle_profile_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::UpdateFieldLevelEncryptionProfile => {
            let root = parse_root(&body)?;
            let cfg = parse_fle_profile_config(&root);
            let f = provider.update_fle_profile(&p.id, if_match, cfg)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::fle_profile_xml(&f),
                Some(&f.etag),
            ))
        }
        Operation::DeleteFieldLevelEncryptionProfile => {
            provider.delete_fle_profile(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListFieldLevelEncryptionProfiles => {
            let items = provider.list_fle_profiles();
            let mut w = crate::xml::XmlWriter::new();
            w.declaration();
            w.open_root(
                "FieldLevelEncryptionProfileList",
                Some(rustack_cloudfront_model::CLOUDFRONT_XML_NAMESPACE),
            );
            w.element_display("Quantity", items.len());
            w.close("FieldLevelEncryptionProfileList");
            Ok(xml_response(StatusCode::OK, w.finish(), None))
        }

        // Monitoring subscription
        Operation::CreateMonitoringSubscription => {
            let root = parse_root(&body)?;
            let enabled = root
                .child("RealtimeMetricsSubscriptionConfig")
                .map(|r| r.child_text("RealtimeMetricsSubscriptionStatus") == "Enabled")
                .unwrap_or(false);
            let sub = provider.create_monitoring_subscription(&p.id, enabled)?;
            let mut w = crate::xml::XmlWriter::new();
            w.declaration();
            w.open_root(
                "MonitoringSubscription",
                Some(rustack_cloudfront_model::CLOUDFRONT_XML_NAMESPACE),
            );
            w.open("RealtimeMetricsSubscriptionConfig");
            w.element(
                "RealtimeMetricsSubscriptionStatus",
                &sub.realtime_metrics_subscription_status,
            );
            w.close("RealtimeMetricsSubscriptionConfig");
            w.close("MonitoringSubscription");
            Ok(xml_response(StatusCode::CREATED, w.finish(), None))
        }
        Operation::GetMonitoringSubscription => {
            let sub = provider.get_monitoring_subscription(&p.id)?;
            let mut w = crate::xml::XmlWriter::new();
            w.declaration();
            w.open_root(
                "MonitoringSubscription",
                Some(rustack_cloudfront_model::CLOUDFRONT_XML_NAMESPACE),
            );
            w.open("RealtimeMetricsSubscriptionConfig");
            w.element(
                "RealtimeMetricsSubscriptionStatus",
                &sub.realtime_metrics_subscription_status,
            );
            w.close("RealtimeMetricsSubscriptionConfig");
            w.close("MonitoringSubscription");
            Ok(xml_response(StatusCode::OK, w.finish(), None))
        }
        Operation::DeleteMonitoringSubscription => {
            provider.delete_monitoring_subscription(&p.id)?;
            Ok(empty_204())
        }

        // KVS
        Operation::CreateKeyValueStore => {
            let root = parse_root(&body)?;
            let name = root.child_text("Name").to_owned();
            let comment = root.child_text("Comment").to_owned();
            let k: KeyValueStore = provider.create_kvs(name, comment)?;
            Ok(xml_response(
                StatusCode::CREATED,
                ser::kvs_xml(&k),
                Some(&k.etag),
            ))
        }
        Operation::DescribeKeyValueStore => {
            let k = provider.get_kvs(&p.id)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::kvs_xml(&k),
                Some(&k.etag),
            ))
        }
        Operation::UpdateKeyValueStore => {
            let root = parse_root(&body)?;
            let comment = root.child_text("Comment").to_owned();
            let k = provider.update_kvs(&p.id, if_match, comment)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::kvs_xml(&k),
                Some(&k.etag),
            ))
        }
        Operation::DeleteKeyValueStore => {
            provider.delete_kvs(&p.id, if_match)?;
            Ok(empty_204())
        }
        Operation::ListKeyValueStores => {
            let items = provider.list_kvs();
            let mut w = crate::xml::XmlWriter::new();
            w.declaration();
            w.open_root(
                "KeyValueStoreList",
                Some(rustack_cloudfront_model::CLOUDFRONT_XML_NAMESPACE),
            );
            w.element_display("MaxItems", query_max_items(uri));
            w.element_display("Quantity", items.len());
            if !items.is_empty() {
                w.open("Items");
                for k in &items {
                    w.open("KeyValueStore");
                    w.element("Name", &k.name);
                    w.element("Id", &k.id);
                    w.element("Comment", &k.comment);
                    w.element("ARN", &k.arn);
                    w.element("Status", &k.status);
                    w.element("LastModifiedTime", &ser::iso8601(&k.last_modified_time));
                    w.close("KeyValueStore");
                }
                w.close("Items");
            }
            w.close("KeyValueStoreList");
            Ok(xml_response(StatusCode::OK, w.finish(), None))
        }

        // Realtime log
        Operation::CreateRealtimeLogConfig | Operation::UpdateRealtimeLogConfig => {
            let root = parse_root(&body)?;
            let cfg = parse_realtime_log_config(&root);
            let r = match route.operation {
                Operation::CreateRealtimeLogConfig => provider.create_realtime_log_config(cfg)?,
                _ => provider.update_realtime_log_config(cfg)?,
            };
            Ok(xml_response(
                StatusCode::OK,
                ser::realtime_log_config_xml(&r),
                None,
            ))
        }
        Operation::GetRealtimeLogConfig => {
            let root = parse_root(&body)?;
            let name = root.child_text("Name");
            let r = provider.get_realtime_log_config(name)?;
            Ok(xml_response(
                StatusCode::OK,
                ser::realtime_log_config_xml(&r),
                None,
            ))
        }
        Operation::DeleteRealtimeLogConfig => {
            // Delete is POST-ish with name in body normally; accept query
            // `Name=` or body `<Name>`.
            let name = if let Some(q) = uri.query() {
                q.split('&')
                    .find_map(|kv| kv.strip_prefix("Name="))
                    .unwrap_or("")
                    .to_owned()
            } else {
                let root = parse_root(&body)?;
                root.child_text("Name").to_owned()
            };
            provider.delete_realtime_log_config(&name)?;
            Ok(empty_204())
        }
        Operation::ListRealtimeLogConfigs => {
            let items = provider.list_realtime_log_configs();
            let mut w = crate::xml::XmlWriter::new();
            w.declaration();
            w.open_root(
                "RealtimeLogConfigs",
                Some(rustack_cloudfront_model::CLOUDFRONT_XML_NAMESPACE),
            );
            w.element_display("MaxItems", query_max_items(uri));
            w.element_display("Quantity", items.len());
            if !items.is_empty() {
                w.open("Items");
                for r in &items {
                    w.raw(
                        &ser::realtime_log_config_xml(r)
                            .trim_start_matches("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
                    );
                }
                w.close("Items");
            }
            w.close("RealtimeLogConfigs");
            Ok(xml_response(StatusCode::OK, w.finish(), None))
        }

        // Tagging
        Operation::TagResource => {
            let arn = query_resource(uri)?;
            let root = parse_root(&body)?;
            let tags = parse_tag_set(&root);
            provider.tag_resource(&arn, &tags)?;
            Ok(empty_204())
        }
        Operation::UntagResource => {
            let arn = query_resource(uri)?;
            let root = parse_root(&body)?;
            let keys = parse_tag_keys(&root);
            provider.untag_resource(&arn, &keys)?;
            Ok(empty_204())
        }
        Operation::ListTagsForResource => {
            let arn = query_resource(uri)?;
            let tags = provider.list_tags_for_resource(&arn)?;
            Ok(xml_response(StatusCode::OK, ser::tags_xml(&tags), None))
        }

        // Phase 4 stubs — return empty success/list responses.
        op => stub_response(op, uri, &route.path_params),
    }
}

fn base64_decode_or_raw(s: &str) -> Vec<u8> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    STANDARD.decode(s).unwrap_or_else(|_| s.as_bytes().to_vec())
}

fn query_max_items(uri: &Uri) -> i32 {
    uri.query()
        .and_then(|q| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix("MaxItems="))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(100)
}

fn query_resource(uri: &Uri) -> Result<String, CloudFrontError> {
    let q = uri.query().unwrap_or("");
    for kv in q.split('&') {
        if let Some(v) = kv.strip_prefix("Resource=") {
            return Ok(percent_decode(v));
        }
    }
    Err(CloudFrontError::MissingArgument(
        "Resource query parameter is required".into(),
    ))
}

fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

fn stub_response(
    op: Operation,
    _uri: &Uri,
    _p: &PathParams,
) -> Result<Response<HttpBody>, CloudFrontError> {
    let root_name = match op {
        Operation::ListConflictingAliases => "ConflictingAliasesList",
        Operation::ListDistributionsByCachePolicyId
        | Operation::ListDistributionsByKeyGroup
        | Operation::ListDistributionsByOriginRequestPolicyId
        | Operation::ListDistributionsByRealtimeLogConfig
        | Operation::ListDistributionsByResponseHeadersPolicyId
        | Operation::ListDistributionsByVpcOriginId
        | Operation::ListDistributionsByWebACLId
        | Operation::ListDistributionsByAnycastIpListId => "DistributionIdList",
        Operation::ListStreamingDistributions => "StreamingDistributionList",
        Operation::ListContinuousDeploymentPolicies => "ContinuousDeploymentPolicyList",
        Operation::ListAnycastIpLists => "AnycastIpListCollection",
        Operation::ListVpcOrigins => "VpcOriginList",
        Operation::ListTrustStores => "TrustStoreList",
        Operation::ListDomainConflicts => "DomainConflictList",
        _ => "StubResult",
    };
    let mut w = crate::xml::XmlWriter::new();
    w.declaration();
    w.open_root(
        root_name,
        Some(rustack_cloudfront_model::CLOUDFRONT_XML_NAMESPACE),
    );
    w.element_display("Quantity", 0);
    w.bool("IsTruncated", false);
    w.element_display("MaxItems", 100);
    w.close(root_name);
    Ok(xml_response(StatusCode::OK, w.finish(), None))
}

// Silence unused import warnings when Method is not otherwise used in this file.
#[allow(dead_code)]
fn _force_method_usage(_m: Method) {}
