//! Lambda request router.
//!
//! Lambda uses the `restJson1` protocol where operations are identified by
//! HTTP method and URL path pattern. Path segments wrapped in `{param}` are
//! extracted as named parameters.
//!
//! ```text
//! POST /2015-03-31/functions               -> CreateFunction
//! GET  /2015-03-31/functions/{FunctionName} -> GetFunction
//! ```

use http::Method;
use ruststack_lambda_model::{
    error::LambdaError,
    operations::{LAMBDA_ROUTES, LambdaOperation},
};

/// Extracted path parameters from a matched route.
///
/// For a pattern like `/2015-03-31/functions/{FunctionName}/aliases/{Name}`,
/// matching against `/2015-03-31/functions/my-func/aliases/live` yields
/// entries `[("FunctionName", "my-func"), ("Name", "live")]`.
#[derive(Debug, Clone, Default)]
pub struct PathParams {
    entries: Vec<(String, String)>,
}

impl PathParams {
    /// Look up a path parameter by name.
    ///
    /// Returns `None` if no parameter with the given name was extracted.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Returns the number of extracted parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no parameters were extracted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Match a request path against a pattern, extracting `{param}` placeholders.
///
/// Returns `None` if the path does not match the pattern. Both the path and
/// pattern are split by `/` and compared segment-by-segment. A pattern segment
/// starting with `{` and ending with `}` is treated as a wildcard that captures
/// the corresponding path segment.
fn match_path(path: &str, pattern: &str) -> Option<PathParams> {
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let pattern_segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();

    if path_segments.len() != pattern_segments.len() {
        return None;
    }

    let mut params = PathParams::default();

    for (path_seg, pat_seg) in path_segments.iter().zip(pattern_segments.iter()) {
        if let Some(name) = pat_seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            // URL-decode the path segment value (e.g., function ARNs may be encoded).
            let decoded = percent_decode(path_seg);
            params.entries.push((name.to_owned(), decoded));
        } else if *path_seg != *pat_seg {
            return None;
        }
    }

    Some(params)
}

/// Simple percent-decoding for path segments.
///
/// Handles `%XX` sequences commonly found in ARN-encoded path parameters.
fn percent_decode(input: &str) -> String {
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

/// Normalize the date prefix in a Lambda API path.
///
/// The AWS SDK may use different date versions than our route table defines.
/// For example, the SDK sends `GET /2016-08-19/account-settings` but our
/// route table uses `/2015-03-31/account-settings`. This function detects the
/// YYYY-MM-DD prefix and maps the resource suffix to the correct canonical date.
fn normalize_date_prefix(path: &str) -> Option<String> {
    let trimmed = path.strip_prefix('/')?;
    let (date_part, rest) = trimmed.split_once('/')?;

    // Validate it looks like a date: YYYY-MM-DD (10 chars).
    if date_part.len() != 10 || date_part.as_bytes()[4] != b'-' || date_part.as_bytes()[7] != b'-' {
        return None;
    }

    // Map resource prefix to canonical date.
    // The AWS SDK may use different date versions in the path prefix. We
    // normalize everything to the date used in LAMBDA_ROUTES so that the
    // route matcher can find the operation.
    let canonical_date = if rest.starts_with("functions") {
        // Function URLs use 2021-10-31, everything else uses 2015-03-31.
        let is_url_path = rest.contains("/url") || rest.ends_with("/urls");
        if is_url_path {
            "2021-10-31"
        } else {
            "2015-03-31"
        }
    } else if rest.starts_with("tags") || rest == "account-settings" {
        "2015-03-31"
    } else if rest.starts_with("layers") {
        "2018-10-31"
    } else if rest.starts_with("event-source-mappings") {
        "2015-03-31"
    } else {
        return None;
    };

    if date_part == canonical_date {
        return None; // Already canonical.
    }

    Some(format!("/{canonical_date}/{rest}"))
}

/// Resolve a Lambda operation from the HTTP method and request path.
///
/// Iterates the [`LAMBDA_ROUTES`] table and returns the first matching
/// operation along with extracted path parameters and the expected success
/// HTTP status code.
///
/// # Errors
///
/// Returns [`LambdaError`] with `UnknownOperation` if no route matches.
pub fn resolve_operation(
    method: &Method,
    path: &str,
) -> Result<(LambdaOperation, PathParams, u16), LambdaError> {
    // Normalize the date prefix: the AWS SDK may use different date versions
    // (e.g., 2016-08-19, 2017-03-31) than the canonical ones in our route table
    // (2015-03-31, 2021-10-31). We normalize any YYYY-MM-DD prefix to the
    // canonical date used by the matching route pattern.
    let normalized = normalize_date_prefix(path);
    let path = normalized.as_deref().unwrap_or(path);

    for route in LAMBDA_ROUTES {
        if route.method != *method {
            continue;
        }
        if let Some(params) = match_path(path, route.path_pattern) {
            tracing::debug!(
                operation = %route.operation,
                path = %path,
                "matched Lambda route"
            );
            return Ok((route.operation, params, route.success_status));
        }
    }

    Err(LambdaError::unknown_operation(method, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- match_path tests ----

    #[test]
    fn test_should_match_exact_path() {
        let params =
            match_path("/2015-03-31/functions", "/2015-03-31/functions").expect("should match");
        assert!(params.is_empty());
    }

    #[test]
    fn test_should_match_path_with_single_param() {
        let params = match_path(
            "/2015-03-31/functions/my-func",
            "/2015-03-31/functions/{FunctionName}",
        )
        .expect("should match");
        assert_eq!(params.len(), 1);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
    }

    #[test]
    fn test_should_match_path_with_multiple_params() {
        let params = match_path(
            "/2015-03-31/functions/my-func/aliases/live",
            "/2015-03-31/functions/{FunctionName}/aliases/{Name}",
        )
        .expect("should match");
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
        assert_eq!(params.get("Name"), Some("live"));
    }

    #[test]
    fn test_should_not_match_shorter_path() {
        assert!(
            match_path(
                "/2015-03-31/functions",
                "/2015-03-31/functions/{FunctionName}"
            )
            .is_none()
        );
    }

    #[test]
    fn test_should_not_match_longer_path() {
        assert!(
            match_path(
                "/2015-03-31/functions/my-func/extra",
                "/2015-03-31/functions/{FunctionName}"
            )
            .is_none()
        );
    }

    #[test]
    fn test_should_not_match_wrong_literal_segment() {
        assert!(
            match_path(
                "/2015-03-31/wrong/my-func",
                "/2015-03-31/functions/{FunctionName}"
            )
            .is_none()
        );
    }

    #[test]
    fn test_should_percent_decode_path_param() {
        let params = match_path(
            "/2015-03-31/functions/arn%3Aaws%3Alambda%3Aus-east-1%3A123456789012%3Afunction%\
             3Amy-func",
            "/2015-03-31/functions/{FunctionName}",
        )
        .expect("should match");
        assert_eq!(
            params.get("FunctionName"),
            Some("arn:aws:lambda:us-east-1:123456789012:function:my-func")
        );
    }

    // ---- resolve_operation tests ----

    #[test]
    fn test_should_resolve_create_function() {
        let (op, params, status) =
            resolve_operation(&Method::POST, "/2015-03-31/functions").expect("should resolve");
        assert_eq!(op, LambdaOperation::CreateFunction);
        assert!(params.is_empty());
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_list_functions() {
        let (op, _, status) =
            resolve_operation(&Method::GET, "/2015-03-31/functions").expect("should resolve");
        assert_eq!(op, LambdaOperation::ListFunctions);
        assert_eq!(status, 200);
    }

    #[test]
    fn test_should_resolve_get_function() {
        let (op, params, status) = resolve_operation(&Method::GET, "/2015-03-31/functions/my-func")
            .expect("should resolve");
        assert_eq!(op, LambdaOperation::GetFunction);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
        assert_eq!(status, 200);
    }

    #[test]
    fn test_should_resolve_delete_function() {
        let (op, params, status) =
            resolve_operation(&Method::DELETE, "/2015-03-31/functions/my-func")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::DeleteFunction);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
        assert_eq!(status, 204);
    }

    #[test]
    fn test_should_resolve_invoke() {
        let (op, params, status) =
            resolve_operation(&Method::POST, "/2015-03-31/functions/my-func/invocations")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::Invoke);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
        assert_eq!(status, 200);
    }

    #[test]
    fn test_should_resolve_update_function_code() {
        let (op, params, _) = resolve_operation(&Method::PUT, "/2015-03-31/functions/my-func/code")
            .expect("should resolve");
        assert_eq!(op, LambdaOperation::UpdateFunctionCode);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
    }

    #[test]
    fn test_should_resolve_get_function_configuration() {
        let (op, params, _) =
            resolve_operation(&Method::GET, "/2015-03-31/functions/my-func/configuration")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::GetFunctionConfiguration);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
    }

    #[test]
    fn test_should_resolve_update_function_configuration() {
        let (op, _, _) =
            resolve_operation(&Method::PUT, "/2015-03-31/functions/my-func/configuration")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::UpdateFunctionConfiguration);
    }

    #[test]
    fn test_should_resolve_create_alias() {
        let (op, params, status) =
            resolve_operation(&Method::POST, "/2015-03-31/functions/my-func/aliases")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::CreateAlias);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_list_aliases() {
        let (op, _, _) = resolve_operation(&Method::GET, "/2015-03-31/functions/my-func/aliases")
            .expect("should resolve");
        assert_eq!(op, LambdaOperation::ListAliases);
    }

    #[test]
    fn test_should_resolve_get_alias() {
        let (op, params, _) =
            resolve_operation(&Method::GET, "/2015-03-31/functions/my-func/aliases/live")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::GetAlias);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
        assert_eq!(params.get("Name"), Some("live"));
    }

    #[test]
    fn test_should_resolve_update_alias() {
        let (op, _, _) =
            resolve_operation(&Method::PUT, "/2015-03-31/functions/my-func/aliases/live")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::UpdateAlias);
    }

    #[test]
    fn test_should_resolve_delete_alias() {
        let (op, _, status) = resolve_operation(
            &Method::DELETE,
            "/2015-03-31/functions/my-func/aliases/live",
        )
        .expect("should resolve");
        assert_eq!(op, LambdaOperation::DeleteAlias);
        assert_eq!(status, 204);
    }

    #[test]
    fn test_should_resolve_publish_version() {
        let (op, _, status) =
            resolve_operation(&Method::POST, "/2015-03-31/functions/my-func/versions")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::PublishVersion);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_list_versions() {
        let (op, _, _) = resolve_operation(&Method::GET, "/2015-03-31/functions/my-func/versions")
            .expect("should resolve");
        assert_eq!(op, LambdaOperation::ListVersionsByFunction);
    }

    #[test]
    fn test_should_resolve_add_permission() {
        let (op, _, status) =
            resolve_operation(&Method::POST, "/2015-03-31/functions/my-func/policy")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::AddPermission);
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_policy() {
        let (op, _, _) = resolve_operation(&Method::GET, "/2015-03-31/functions/my-func/policy")
            .expect("should resolve");
        assert_eq!(op, LambdaOperation::GetPolicy);
    }

    #[test]
    fn test_should_resolve_remove_permission() {
        let (op, params, status) = resolve_operation(
            &Method::DELETE,
            "/2015-03-31/functions/my-func/policy/stmt-1",
        )
        .expect("should resolve");
        assert_eq!(op, LambdaOperation::RemovePermission);
        assert_eq!(params.get("StatementId"), Some("stmt-1"));
        assert_eq!(status, 204);
    }

    #[test]
    fn test_should_resolve_tag_resource() {
        let (op, params, status) = resolve_operation(
            &Method::POST,
            "/2015-03-31/tags/arn%3Aaws%3Alambda%3Aus-east-1%3A123%3Afunction%3Af",
        )
        .expect("should resolve");
        assert_eq!(op, LambdaOperation::TagResource);
        assert_eq!(
            params.get("Resource"),
            Some("arn:aws:lambda:us-east-1:123:function:f")
        );
        assert_eq!(status, 204);
    }

    #[test]
    fn test_should_resolve_list_tags() {
        let (op, _, _) =
            resolve_operation(&Method::GET, "/2015-03-31/tags/some-arn").expect("should resolve");
        assert_eq!(op, LambdaOperation::ListTags);
    }

    #[test]
    fn test_should_resolve_untag_resource() {
        let (op, _, status) = resolve_operation(&Method::DELETE, "/2015-03-31/tags/some-arn")
            .expect("should resolve");
        assert_eq!(op, LambdaOperation::UntagResource);
        assert_eq!(status, 204);
    }

    #[test]
    fn test_should_resolve_get_account_settings() {
        let (op, _, _) = resolve_operation(&Method::GET, "/2015-03-31/account-settings")
            .expect("should resolve");
        assert_eq!(op, LambdaOperation::GetAccountSettings);
    }

    #[test]
    fn test_should_resolve_create_function_url_config() {
        let (op, params, status) =
            resolve_operation(&Method::POST, "/2021-10-31/functions/my-func/url")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::CreateFunctionUrlConfig);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
        assert_eq!(status, 201);
    }

    #[test]
    fn test_should_resolve_get_function_url_config() {
        let (op, _, _) = resolve_operation(&Method::GET, "/2021-10-31/functions/my-func/url")
            .expect("should resolve");
        assert_eq!(op, LambdaOperation::GetFunctionUrlConfig);
    }

    #[test]
    fn test_should_resolve_update_function_url_config() {
        let (op, _, _) = resolve_operation(&Method::PUT, "/2021-10-31/functions/my-func/url")
            .expect("should resolve");
        assert_eq!(op, LambdaOperation::UpdateFunctionUrlConfig);
    }

    #[test]
    fn test_should_resolve_delete_function_url_config() {
        let (op, _, status) =
            resolve_operation(&Method::DELETE, "/2021-10-31/functions/my-func/url")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::DeleteFunctionUrlConfig);
        assert_eq!(status, 204);
    }

    #[test]
    fn test_should_resolve_list_function_url_configs() {
        let (op, params, status) =
            resolve_operation(&Method::GET, "/2021-10-31/functions/my-func/urls")
                .expect("should resolve");
        assert_eq!(op, LambdaOperation::ListFunctionUrlConfigs);
        assert_eq!(params.get("FunctionName"), Some("my-func"));
        assert_eq!(status, 200);
    }

    #[test]
    fn test_should_error_on_unknown_route() {
        let err =
            resolve_operation(&Method::GET, "/2015-03-31/nonexistent").expect_err("should error");
        assert_eq!(
            err.code,
            ruststack_lambda_model::error::LambdaErrorCode::UnknownOperation
        );
    }

    #[test]
    fn test_should_error_on_wrong_method() {
        let err =
            resolve_operation(&Method::PATCH, "/2015-03-31/functions").expect_err("should error");
        assert_eq!(
            err.code,
            ruststack_lambda_model::error::LambdaErrorCode::UnknownOperation
        );
    }

    #[test]
    fn test_should_handle_trailing_slash() {
        // Trailing slash should NOT match (different segment count after filtering).
        // "/2015-03-31/functions/" splits into ["2015-03-31", "functions", ""]
        // but filter(|s| !s.is_empty()) removes empty, so it matches.
        let result = resolve_operation(&Method::GET, "/2015-03-31/functions/");
        assert!(result.is_ok());
    }

    // ---- percent_decode tests ----

    #[test]
    fn test_should_decode_percent_encoded_colons() {
        let decoded = super::percent_decode("arn%3Aaws%3Alambda");
        assert_eq!(decoded, "arn:aws:lambda");
    }

    #[test]
    fn test_should_pass_through_plain_text() {
        let decoded = super::percent_decode("my-function");
        assert_eq!(decoded, "my-function");
    }

    #[test]
    fn test_should_handle_malformed_percent_encoding() {
        let decoded = super::percent_decode("bad%ZZstuff");
        assert_eq!(decoded, "bad%ZZstuff");
    }
}
