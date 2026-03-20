//! In-memory storage for API Gateway v2 resources.
//!
//! Uses `DashMap` for concurrent access to top-level resources (APIs, domain names,
//! VPC links). API-scoped resources (routes, integrations, stages, deployments,
//! authorizers, models) are stored within `ApiRecord` as `HashMap`s.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use rand::Rng;

use ruststack_apigatewayv2_model::types::{
    AccessLogSettings, AuthorizationType, AuthorizerType, ConnectionType, ContentHandlingStrategy,
    Cors, DeploymentStatus, DomainNameConfiguration, IntegrationType, IpAddressType,
    JWTConfiguration, MutualTlsAuthentication, ParameterConstraints, PassthroughBehavior,
    ProtocolType, RouteSettings, RoutingMode, TlsConfig,
};

/// Generate a random 10-character alphanumeric ID.
///
/// Matches the format used by AWS API Gateway v2 (e.g., "abc1234def").
#[must_use]
pub fn generate_id() -> String {
    let mut rng = rand::rng();
    (0..10)
        .map(|_| {
            let idx = rng.random_range(0..36u8);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect()
}

/// Central storage for all API Gateway v2 resources.
#[derive(Debug, Default)]
pub struct ApiStore {
    /// APIs keyed by API ID.
    pub apis: DashMap<String, ApiRecord>,
    /// Domain names keyed by domain name.
    pub domain_names: DashMap<String, DomainNameRecord>,
    /// VPC links keyed by VPC link ID.
    pub vpc_links: DashMap<String, VpcLinkRecord>,
    /// Tags keyed by resource ARN.
    pub tags: DashMap<String, HashMap<String, String>>,
}

impl ApiStore {
    /// Create a new empty API store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// A single HTTP API record.
#[derive(Debug, Clone)]
pub struct ApiRecord {
    /// Unique API identifier (10-char alphanumeric).
    pub api_id: String,
    /// Human-readable API name.
    pub name: String,
    /// Protocol type: HTTP or WEBSOCKET.
    pub protocol_type: ProtocolType,
    /// Route selection expression.
    pub route_selection_expression: String,
    /// API key selection expression.
    pub api_key_selection_expression: Option<String>,
    /// CORS configuration.
    pub cors_configuration: Option<Cors>,
    /// Description.
    pub description: Option<String>,
    /// Whether the default execute-api endpoint is disabled.
    pub disable_execute_api_endpoint: bool,
    /// Whether schema validation is disabled.
    pub disable_schema_validation: bool,
    /// IP address type.
    pub ip_address_type: Option<IpAddressType>,
    /// API version.
    pub version: Option<String>,
    /// The generated API endpoint.
    pub api_endpoint: String,
    /// Tags.
    pub tags: HashMap<String, String>,
    /// Creation timestamp.
    pub created_date: DateTime<Utc>,
    /// Routes keyed by route ID.
    pub routes: HashMap<String, RouteRecord>,
    /// Integrations keyed by integration ID.
    pub integrations: HashMap<String, IntegrationRecord>,
    /// Stages keyed by stage name.
    pub stages: HashMap<String, StageRecord>,
    /// Deployments keyed by deployment ID.
    pub deployments: HashMap<String, DeploymentRecord>,
    /// Authorizers keyed by authorizer ID.
    pub authorizers: HashMap<String, AuthorizerRecord>,
    /// Models keyed by model ID.
    pub models: HashMap<String, ModelRecord>,
}

/// A route within an API.
#[derive(Debug, Clone)]
pub struct RouteRecord {
    /// Unique route identifier.
    pub route_id: String,
    /// Route key: "{METHOD} {path}" or "$default".
    pub route_key: String,
    /// Target integration: "integrations/{integrationId}".
    pub target: Option<String>,
    /// Authorization type.
    pub authorization_type: Option<AuthorizationType>,
    /// Authorizer ID.
    pub authorizer_id: Option<String>,
    /// Authorization scopes.
    pub authorization_scopes: Vec<String>,
    /// Whether API key is required.
    pub api_key_required: bool,
    /// Model selection expression.
    pub model_selection_expression: Option<String>,
    /// Operation name.
    pub operation_name: Option<String>,
    /// Request models.
    pub request_models: HashMap<String, String>,
    /// Request parameters.
    pub request_parameters: HashMap<String, ParameterConstraints>,
    /// Route response selection expression.
    pub route_response_selection_expression: Option<String>,
    /// Route responses keyed by route response ID.
    pub route_responses: HashMap<String, RouteResponseRecord>,
    /// Whether this route is API Gateway managed.
    pub api_gateway_managed: bool,
}

/// A route response record.
#[derive(Debug, Clone)]
pub struct RouteResponseRecord {
    /// Unique route response identifier.
    pub route_response_id: String,
    /// Route response key (e.g., "$default").
    pub route_response_key: String,
    /// Model selection expression.
    pub model_selection_expression: Option<String>,
    /// Response models.
    pub response_models: HashMap<String, String>,
    /// Response parameters.
    pub response_parameters: HashMap<String, ParameterConstraints>,
}

/// An integration connecting a route to a backend.
#[derive(Debug, Clone)]
pub struct IntegrationRecord {
    /// Unique integration identifier.
    pub integration_id: String,
    /// Integration type.
    pub integration_type: IntegrationType,
    /// Integration method.
    pub integration_method: Option<String>,
    /// Integration URI.
    pub integration_uri: Option<String>,
    /// Connection type.
    pub connection_type: Option<ConnectionType>,
    /// Connection ID.
    pub connection_id: Option<String>,
    /// Content handling strategy.
    pub content_handling_strategy: Option<ContentHandlingStrategy>,
    /// Credentials ARN.
    pub credentials_arn: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Passthrough behavior.
    pub passthrough_behavior: Option<PassthroughBehavior>,
    /// Payload format version.
    pub payload_format_version: Option<String>,
    /// Request parameters mapping.
    pub request_parameters: HashMap<String, String>,
    /// Request templates.
    pub request_templates: HashMap<String, String>,
    /// Response parameters mapping.
    pub response_parameters: HashMap<String, HashMap<String, String>>,
    /// Template selection expression.
    pub template_selection_expression: Option<String>,
    /// Timeout in milliseconds.
    pub timeout_in_millis: Option<i32>,
    /// TLS configuration.
    pub tls_config: Option<TlsConfig>,
    /// Integration subtype.
    pub integration_subtype: Option<String>,
    /// Whether this integration is API Gateway managed.
    pub api_gateway_managed: bool,
    /// Integration response selection expression.
    pub integration_response_selection_expression: Option<String>,
}

/// A stage configuration for an API.
#[derive(Debug, Clone)]
pub struct StageRecord {
    /// Stage name.
    pub stage_name: String,
    /// Associated deployment ID.
    pub deployment_id: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Whether auto-deploy is enabled.
    pub auto_deploy: bool,
    /// Stage variables.
    pub stage_variables: HashMap<String, String>,
    /// Default route settings.
    pub default_route_settings: Option<RouteSettings>,
    /// Per-route settings overrides.
    pub route_settings: HashMap<String, RouteSettings>,
    /// Access log settings.
    pub access_log_settings: Option<AccessLogSettings>,
    /// Client certificate ID.
    pub client_certificate_id: Option<String>,
    /// Tags.
    pub tags: HashMap<String, String>,
    /// Creation timestamp.
    pub created_date: DateTime<Utc>,
    /// Last updated timestamp.
    pub last_updated_date: DateTime<Utc>,
    /// Whether this stage is API Gateway managed.
    pub api_gateway_managed: bool,
}

/// A deployment snapshot.
#[derive(Debug, Clone)]
pub struct DeploymentRecord {
    /// Unique deployment identifier.
    pub deployment_id: String,
    /// Description.
    pub description: Option<String>,
    /// Whether this deployment was auto-deployed.
    pub auto_deployed: bool,
    /// Deployment status.
    pub deployment_status: DeploymentStatus,
    /// Deployment status message.
    pub deployment_status_message: Option<String>,
    /// Creation timestamp.
    pub created_date: DateTime<Utc>,
}

/// An authorizer for an API.
#[derive(Debug, Clone)]
pub struct AuthorizerRecord {
    /// Unique authorizer identifier.
    pub authorizer_id: String,
    /// Authorizer name.
    pub name: String,
    /// Authorizer type.
    pub authorizer_type: AuthorizerType,
    /// JWT configuration.
    pub jwt_configuration: Option<JWTConfiguration>,
    /// Authorizer credentials ARN.
    pub authorizer_credentials_arn: Option<String>,
    /// Authorizer URI.
    pub authorizer_uri: Option<String>,
    /// Identity source expressions.
    pub identity_source: Vec<String>,
    /// Identity validation expression.
    pub identity_validation_expression: Option<String>,
    /// Authorizer payload format version.
    pub authorizer_payload_format_version: Option<String>,
    /// Authorizer result TTL in seconds.
    pub authorizer_result_ttl_in_seconds: Option<i32>,
    /// Whether simple responses are enabled.
    pub enable_simple_responses: Option<bool>,
}

/// A model schema for an API.
#[derive(Debug, Clone)]
pub struct ModelRecord {
    /// Unique model identifier.
    pub model_id: String,
    /// Model name.
    pub name: String,
    /// Content type.
    pub content_type: Option<String>,
    /// JSON schema string.
    pub schema: String,
    /// Description.
    pub description: Option<String>,
}

/// A custom domain name record.
#[derive(Debug, Clone)]
pub struct DomainNameRecord {
    /// The domain name.
    pub domain_name: String,
    /// Domain name configurations.
    pub domain_name_configurations: Vec<DomainNameConfiguration>,
    /// Mutual TLS authentication configuration.
    pub mutual_tls_authentication: Option<MutualTlsAuthentication>,
    /// Routing mode.
    pub routing_mode: Option<RoutingMode>,
    /// Tags.
    pub tags: HashMap<String, String>,
    /// API mappings keyed by mapping ID.
    pub api_mappings: HashMap<String, ApiMappingRecord>,
}

/// An API mapping record.
#[derive(Debug, Clone)]
pub struct ApiMappingRecord {
    /// Unique API mapping identifier.
    pub api_mapping_id: String,
    /// API ID.
    pub api_id: String,
    /// API mapping key (path prefix).
    pub api_mapping_key: Option<String>,
    /// Stage name.
    pub stage: String,
}

/// A VPC link record.
#[derive(Debug, Clone)]
pub struct VpcLinkRecord {
    /// Unique VPC link identifier.
    pub vpc_link_id: String,
    /// VPC link name.
    pub name: String,
    /// Security group IDs.
    pub security_group_ids: Vec<String>,
    /// Subnet IDs.
    pub subnet_ids: Vec<String>,
    /// Tags.
    pub tags: HashMap<String, String>,
    /// Creation timestamp.
    pub created_date: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_generate_10_char_id() {
        let id = generate_id();
        assert_eq!(id.len(), 10);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_should_create_empty_store() {
        let store = ApiStore::new();
        assert!(store.apis.is_empty());
        assert!(store.domain_names.is_empty());
        assert!(store.vpc_links.is_empty());
    }
}
