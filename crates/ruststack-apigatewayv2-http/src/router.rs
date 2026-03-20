//! API Gateway v2 URL router.
//!
//! Matches incoming HTTP requests against the route table defined in the model
//! crate and extracts path parameters.

use std::collections::HashMap;

use ruststack_apigatewayv2_model::error::ApiGatewayV2Error;
use ruststack_apigatewayv2_model::operations::{APIGATEWAYV2_ROUTES, ApiGatewayV2Operation};

/// Extracted path parameters from a matched route.
#[derive(Debug, Clone, Default)]
pub struct PathParams {
    params: HashMap<String, String>,
}

impl PathParams {
    /// Get a path parameter by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(String::as_str)
    }

    /// Insert a path parameter.
    pub fn insert(&mut self, name: String, value: String) {
        self.params.insert(name, value);
    }

    /// Returns the number of extracted parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.params.len()
    }

    /// Returns `true` if no parameters were extracted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// Iterate over all extracted parameters.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// Match an HTTP request path against a route pattern, extracting parameters.
fn match_path(path: &str, pattern: &str) -> Option<PathParams> {
    let path_segments: Vec<&str> = path.split('/').collect();
    let pattern_segments: Vec<&str> = pattern.split('/').collect();

    if path_segments.len() != pattern_segments.len() {
        return None;
    }

    let mut params = PathParams::default();
    for (ps, pp) in path_segments.iter().zip(pattern_segments.iter()) {
        if pp.starts_with('{') && pp.ends_with('}') {
            let name = &pp[1..pp.len() - 1];
            params.insert(name.to_owned(), (*ps).to_owned());
        } else if ps != pp {
            return None;
        }
    }
    Some(params)
}

/// Resolve an HTTP request to an API Gateway v2 operation.
///
/// Returns the matched operation, extracted path parameters, and success status code.
///
/// # Errors
///
/// Returns `ApiGatewayV2Error` if no route matches.
pub fn resolve_operation(
    method: &http::Method,
    path: &str,
) -> Result<(ApiGatewayV2Operation, PathParams, u16), ApiGatewayV2Error> {
    for route in APIGATEWAYV2_ROUTES {
        if route.method == *method {
            if let Some(params) = match_path(path, route.path_pattern) {
                return Ok((route.operation, params, route.success_status));
            }
        }
    }
    Err(ApiGatewayV2Error::unknown_operation(method, path))
}

/// Simple percent-decoding for path segments.
///
/// Handles `%XX` sequences commonly found in ARN-encoded path parameters.
#[must_use]
pub fn percent_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            // Malformed percent-encoding, keep literal.
            result.push('%');
            result.push_str(&hex);
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- match_path tests ----

    #[test]
    fn test_should_match_exact_path() {
        let params = match_path("/v2/apis", "/v2/apis").expect("should match");
        assert!(params.params.is_empty());
    }

    #[test]
    fn test_should_match_path_with_single_param() {
        let params = match_path("/v2/apis/abc123", "/v2/apis/{apiId}").expect("should match");
        assert_eq!(params.get("apiId"), Some("abc123"));
    }

    #[test]
    fn test_should_match_path_with_multiple_params() {
        let params = match_path(
            "/v2/apis/abc123/routes/rt-456",
            "/v2/apis/{apiId}/routes/{routeId}",
        )
        .expect("should match");
        assert_eq!(params.get("apiId"), Some("abc123"));
        assert_eq!(params.get("routeId"), Some("rt-456"));
    }

    #[test]
    fn test_should_match_deep_nested_path() {
        let params = match_path(
            "/v2/apis/abc123/routes/rt-456/routeresponses/rr-789",
            "/v2/apis/{apiId}/routes/{routeId}/routeresponses/{routeResponseId}",
        )
        .expect("should match");
        assert_eq!(params.get("apiId"), Some("abc123"));
        assert_eq!(params.get("routeId"), Some("rt-456"));
        assert_eq!(params.get("routeResponseId"), Some("rr-789"));
    }

    #[test]
    fn test_should_not_match_shorter_path() {
        assert!(match_path("/v2/apis", "/v2/apis/{apiId}").is_none());
    }

    #[test]
    fn test_should_not_match_longer_path() {
        assert!(match_path("/v2/apis/abc123/extra", "/v2/apis/{apiId}").is_none());
    }

    #[test]
    fn test_should_not_match_wrong_literal_segment() {
        assert!(match_path("/v2/wrong/abc123", "/v2/apis/{apiId}").is_none());
    }

    // ---- resolve_operation tests ----

    #[test]
    fn test_should_resolve_create_api() {
        let (op, params, status) =
            resolve_operation(&http::Method::POST, "/v2/apis").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateApi);
        assert!(params.params.is_empty());
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_apis() {
        let (op, _, status) =
            resolve_operation(&http::Method::GET, "/v2/apis").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetApis);
        assert_eq!(status, 200);
    }

    #[test]
    fn test_should_resolve_get_api() {
        let (op, params, status) =
            resolve_operation(&http::Method::GET, "/v2/apis/abc123").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetApi);
        assert_eq!(params.get("apiId"), Some("abc123"));
        assert_eq!(status, 200);
    }

    #[test]
    fn test_should_resolve_update_api() {
        let (op, params, _) =
            resolve_operation(&http::Method::PATCH, "/v2/apis/abc123").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::UpdateApi);
        assert_eq!(params.get("apiId"), Some("abc123"));
    }

    #[test]
    fn test_should_resolve_delete_api() {
        let (op, _, status) =
            resolve_operation(&http::Method::DELETE, "/v2/apis/abc123").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::DeleteApi);
        assert_eq!(status, 204);
    }

    #[test]
    fn test_should_resolve_create_route() {
        let (op, params, status) = resolve_operation(&http::Method::POST, "/v2/apis/abc123/routes")
            .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateRoute);
        assert_eq!(params.get("apiId"), Some("abc123"));
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_route() {
        let (op, params, _) =
            resolve_operation(&http::Method::GET, "/v2/apis/abc123/routes/rt-456")
                .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetRoute);
        assert_eq!(params.get("apiId"), Some("abc123"));
        assert_eq!(params.get("routeId"), Some("rt-456"));
    }

    #[test]
    fn test_should_resolve_delete_route() {
        let (op, _, status) =
            resolve_operation(&http::Method::DELETE, "/v2/apis/abc123/routes/rt-456")
                .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::DeleteRoute);
        assert_eq!(status, 204);
    }

    #[test]
    fn test_should_resolve_create_integration() {
        let (op, _, status) =
            resolve_operation(&http::Method::POST, "/v2/apis/abc123/integrations")
                .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateIntegration);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_integration() {
        let (op, params, _) =
            resolve_operation(&http::Method::GET, "/v2/apis/abc123/integrations/int-789")
                .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetIntegration);
        assert_eq!(params.get("integrationId"), Some("int-789"));
    }

    #[test]
    fn test_should_resolve_create_stage() {
        let (op, _, status) = resolve_operation(&http::Method::POST, "/v2/apis/abc123/stages")
            .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateStage);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_stage() {
        let (op, params, _) = resolve_operation(&http::Method::GET, "/v2/apis/abc123/stages/prod")
            .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetStage);
        assert_eq!(params.get("stageName"), Some("prod"));
    }

    #[test]
    fn test_should_resolve_create_deployment() {
        let (op, _, status) = resolve_operation(&http::Method::POST, "/v2/apis/abc123/deployments")
            .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateDeployment);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_route_response() {
        let (op, params, status) = resolve_operation(
            &http::Method::GET,
            "/v2/apis/abc123/routes/rt-456/routeresponses/rr-789",
        )
        .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetRouteResponse);
        assert_eq!(params.get("apiId"), Some("abc123"));
        assert_eq!(params.get("routeId"), Some("rt-456"));
        assert_eq!(params.get("routeResponseId"), Some("rr-789"));
        assert_eq!(status, 200);
    }

    #[test]
    fn test_should_resolve_create_route_response() {
        let (op, _, status) = resolve_operation(
            &http::Method::POST,
            "/v2/apis/abc123/routes/rt-456/routeresponses",
        )
        .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateRouteResponse);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_model_template() {
        let (op, params, _) =
            resolve_operation(&http::Method::GET, "/v2/apis/abc123/models/mod-1/template")
                .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetModelTemplate);
        assert_eq!(params.get("modelId"), Some("mod-1"));
    }

    #[test]
    fn test_should_resolve_create_authorizer() {
        let (op, _, status) = resolve_operation(&http::Method::POST, "/v2/apis/abc123/authorizers")
            .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateAuthorizer);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_create_domain_name() {
        let (op, _, status) =
            resolve_operation(&http::Method::POST, "/v2/domainnames").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateDomainName);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_domain_name() {
        let (op, params, _) = resolve_operation(&http::Method::GET, "/v2/domainnames/example.com")
            .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetDomainName);
        assert_eq!(params.get("domainName"), Some("example.com"));
    }

    #[test]
    fn test_should_resolve_create_vpc_link() {
        let (op, _, status) =
            resolve_operation(&http::Method::POST, "/v2/vpclinks").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateVpcLink);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_delete_vpc_link() {
        let (op, _, status) =
            resolve_operation(&http::Method::DELETE, "/v2/vpclinks/vpc-1").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::DeleteVpcLink);
        assert_eq!(status, 202);
    }

    #[test]
    fn test_should_resolve_tag_resource() {
        let (op, params, status) =
            resolve_operation(&http::Method::POST, "/v2/tags/some-arn").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::TagResource);
        assert_eq!(params.get("resource-arn"), Some("some-arn"));
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_tags() {
        let (op, _, _) =
            resolve_operation(&http::Method::GET, "/v2/tags/some-arn").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetTags);
    }

    #[test]
    fn test_should_resolve_untag_resource() {
        let (op, _, status) =
            resolve_operation(&http::Method::DELETE, "/v2/tags/some-arn").expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::UntagResource);
        assert_eq!(status, 204);
    }

    #[test]
    fn test_should_resolve_get_api_mapping() {
        let (op, params, _) = resolve_operation(
            &http::Method::GET,
            "/v2/domainnames/example.com/apimappings/map-1",
        )
        .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::GetApiMapping);
        assert_eq!(params.get("domainName"), Some("example.com"));
        assert_eq!(params.get("apiMappingId"), Some("map-1"));
    }

    #[test]
    fn test_should_resolve_create_api_mapping() {
        let (op, _, status) = resolve_operation(
            &http::Method::POST,
            "/v2/domainnames/example.com/apimappings",
        )
        .expect("should resolve");
        assert_eq!(op, ApiGatewayV2Operation::CreateApiMapping);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_error_on_unknown_route() {
        let err =
            resolve_operation(&http::Method::GET, "/v2/nonexistent").expect_err("should error");
        assert_eq!(
            err.code,
            ruststack_apigatewayv2_model::error::ApiGatewayV2ErrorCode::UnknownOperation
        );
    }

    #[test]
    fn test_should_error_on_wrong_method() {
        let err = resolve_operation(&http::Method::PATCH, "/v2/apis").expect_err("should error");
        assert_eq!(
            err.code,
            ruststack_apigatewayv2_model::error::ApiGatewayV2ErrorCode::UnknownOperation
        );
    }

    // ---- percent_decode tests ----

    #[test]
    fn test_should_decode_percent_encoded_colons() {
        let decoded = percent_decode("arn%3Aaws%3Aapigateway");
        assert_eq!(decoded, "arn:aws:apigateway");
    }

    #[test]
    fn test_should_pass_through_plain_text() {
        let decoded = percent_decode("my-api-id");
        assert_eq!(decoded, "my-api-id");
    }

    #[test]
    fn test_should_handle_malformed_percent_encoding() {
        let decoded = percent_decode("bad%ZZstuff");
        assert_eq!(decoded, "bad%ZZstuff");
    }
}
