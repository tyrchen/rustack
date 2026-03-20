//! Lambda event v2 construction for API Gateway v2 proxy integrations.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// API Gateway v2 Lambda event (payload format version 2.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiGatewayV2Event {
    /// The version of the event format.
    pub version: String,
    /// Route key that matched.
    pub route_key: String,
    /// Raw path.
    pub raw_path: String,
    /// Raw query string.
    pub raw_query_string: String,
    /// Headers.
    pub headers: HashMap<String, String>,
    /// Query string parameters.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub query_string_parameters: HashMap<String, String>,
    /// Path parameters.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub path_parameters: HashMap<String, String>,
    /// Stage variables.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub stage_variables: HashMap<String, String>,
    /// Request context.
    pub request_context: RequestContext,
    /// Body (base64 encoded if binary).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Whether the body is base64 encoded.
    #[serde(default)]
    pub is_base64_encoded: bool,
}

/// Request context for the Lambda event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestContext {
    /// Account ID.
    pub account_id: String,
    /// API ID.
    pub api_id: String,
    /// Domain name.
    pub domain_name: String,
    /// Domain prefix.
    pub domain_prefix: String,
    /// HTTP method and path info.
    pub http: HttpInfo,
    /// Request ID.
    pub request_id: String,
    /// Route key.
    pub route_key: String,
    /// Stage name.
    pub stage: String,
    /// Timestamp in epoch milliseconds.
    pub time_epoch: i64,
}

/// HTTP information in the request context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpInfo {
    /// HTTP method.
    pub method: String,
    /// Path.
    pub path: String,
    /// Protocol (e.g., "HTTP/1.1").
    pub protocol: String,
    /// Source IP.
    pub source_ip: String,
    /// User agent.
    pub user_agent: String,
}

/// Parameters for building a Lambda event v2.
#[derive(Debug)]
pub struct LambdaEventParams<'a> {
    /// API ID.
    pub api_id: &'a str,
    /// Stage name.
    pub stage_name: &'a str,
    /// Route key.
    pub route_key: &'a str,
    /// HTTP method.
    pub method: &'a str,
    /// Request path.
    pub path: &'a str,
    /// Request headers.
    pub headers: &'a http::HeaderMap,
    /// Request body.
    pub body: &'a [u8],
    /// Extracted path parameters.
    pub path_params: &'a HashMap<String, String>,
    /// Stage variables.
    pub stage_variables: &'a HashMap<String, String>,
    /// AWS account ID.
    pub account_id: &'a str,
    /// AWS region.
    pub region: &'a str,
}

/// Build a Lambda event v2 from request components.
#[must_use]
pub fn build_lambda_event(params: &LambdaEventParams<'_>) -> ApiGatewayV2Event {
    let header_map: HashMap<String, String> = params
        .headers
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
        .collect();

    let domain_name = format!(
        "{}.execute-api.{}.amazonaws.com",
        params.api_id, params.region
    );
    let user_agent = header_map.get("user-agent").cloned().unwrap_or_default();

    let (event_body, is_base64) = if params.body.is_empty() {
        (None, false)
    } else if is_text_content(params.headers) {
        (
            Some(String::from_utf8_lossy(params.body).into_owned()),
            false,
        )
    } else {
        use base64::Engine;
        (
            Some(base64::engine::general_purpose::STANDARD.encode(params.body)),
            true,
        )
    };

    ApiGatewayV2Event {
        version: "2.0".to_owned(),
        route_key: params.route_key.to_owned(),
        raw_path: params.path.to_owned(),
        raw_query_string: String::new(),
        headers: header_map,
        query_string_parameters: HashMap::new(),
        path_parameters: params.path_params.clone(),
        stage_variables: params.stage_variables.clone(),
        request_context: RequestContext {
            account_id: params.account_id.to_owned(),
            api_id: params.api_id.to_owned(),
            domain_name: domain_name.clone(),
            domain_prefix: params.api_id.to_owned(),
            http: HttpInfo {
                method: params.method.to_owned(),
                path: params.path.to_owned(),
                protocol: "HTTP/1.1".to_owned(),
                source_ip: "127.0.0.1".to_owned(),
                user_agent,
            },
            request_id: uuid::Uuid::new_v4().to_string(),
            route_key: params.route_key.to_owned(),
            stage: params.stage_name.to_owned(),
            time_epoch: chrono::Utc::now().timestamp_millis(),
        },
        body: event_body,
        is_base64_encoded: is_base64,
    }
}

/// Check if the content type indicates text.
fn is_text_content(headers: &http::HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_none_or(|ct| {
            ct.starts_with("text/")
                || ct.contains("json")
                || ct.contains("xml")
                || ct.contains("javascript")
                || ct.contains("html")
                || ct.contains("form-urlencoded")
        })
}
