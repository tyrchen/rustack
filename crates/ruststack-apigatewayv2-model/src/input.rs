//! Auto-generated from AWS ApiGatewayV2 Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    AccessLogSettings, AuthorizationType, AuthorizerType, ConnectionType, ContentHandlingStrategy,
    Cors, DomainNameConfiguration, IntegrationType, IpAddressType, JWTConfiguration,
    MutualTlsAuthenticationInput, ParameterConstraints, PassthroughBehavior, ProtocolType,
    RouteSettings, RoutingMode, TlsConfigInput,
};

/// ApiGatewayV2 CreateApiInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_selection_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors_configuration: Option<Cors>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_execute_api_endpoint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_schema_validation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address_type: Option<IpAddressType>,
    pub name: String,
    pub protocol_type: ProtocolType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_selection_expression: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// ApiGatewayV2 CreateApiMappingInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiMappingInput {
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_key: Option<String>,
    /// HTTP label (URI path).
    pub domain_name: String,
    pub stage: String,
}

/// ApiGatewayV2 CreateAuthorizerInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAuthorizerInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_credentials_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_payload_format_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_result_ttl_in_seconds: Option<i32>,
    pub authorizer_type: AuthorizerType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_simple_responses: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_source: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_validation_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwt_configuration: Option<JWTConfiguration>,
    pub name: String,
}

/// ApiGatewayV2 CreateDeploymentInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDeploymentInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_name: Option<String>,
}

/// ApiGatewayV2 CreateDomainNameInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDomainNameInput {
    pub domain_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_name_configurations: Vec<DomainNameConfiguration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutual_tls_authentication: Option<MutualTlsAuthenticationInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_mode: Option<RoutingMode>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 CreateIntegrationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIntegrationInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<ConnectionType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_handling_strategy: Option<ContentHandlingStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_subtype: Option<String>,
    pub integration_type: IntegrationType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passthrough_behavior: Option<PassthroughBehavior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_format_version: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_templates: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_parameters: HashMap<String, HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_selection_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_in_millis: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_config: Option<TlsConfigInput>,
}

/// ApiGatewayV2 CreateModelInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateModelInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub name: String,
    pub schema: String,
}

/// ApiGatewayV2 CreateRouteInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRouteInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorization_scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_type: Option<AuthorizationType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_models: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, ParameterConstraints>,
    pub route_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_selection_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// ApiGatewayV2 CreateRouteResponseInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRouteResponseInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection_expression: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_models: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_parameters: HashMap<String, ParameterConstraints>,
    /// HTTP label (URI path).
    pub route_id: String,
    pub route_response_key: String,
}

/// ApiGatewayV2 CreateStageInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateStageInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_log_settings: Option<AccessLogSettings>,
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_deploy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_certificate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_route_settings: Option<RouteSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub route_settings: HashMap<String, RouteSettings>,
    pub stage_name: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub stage_variables: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 CreateVpcLinkInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVpcLinkInput {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_group_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subnet_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 DeleteApiInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteApiInput {
    /// HTTP label (URI path).
    pub api_id: String,
}

/// ApiGatewayV2 DeleteApiMappingInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteApiMappingInput {
    /// HTTP label (URI path).
    pub api_mapping_id: String,
    /// HTTP label (URI path).
    pub domain_name: String,
}

/// ApiGatewayV2 DeleteAuthorizerInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAuthorizerInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub authorizer_id: String,
}

/// ApiGatewayV2 DeleteDeploymentInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteDeploymentInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub deployment_id: String,
}

/// ApiGatewayV2 DeleteDomainNameInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteDomainNameInput {
    /// HTTP label (URI path).
    pub domain_name: String,
}

/// ApiGatewayV2 DeleteIntegrationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteIntegrationInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub integration_id: String,
}

/// ApiGatewayV2 DeleteModelInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteModelInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub model_id: String,
}

/// ApiGatewayV2 DeleteRouteInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRouteInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub route_id: String,
}

/// ApiGatewayV2 DeleteRouteResponseInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRouteResponseInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub route_id: String,
    /// HTTP label (URI path).
    pub route_response_id: String,
}

/// ApiGatewayV2 DeleteStageInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteStageInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub stage_name: String,
}

/// ApiGatewayV2 DeleteVpcLinkInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteVpcLinkInput {
    /// HTTP label (URI path).
    pub vpc_link_id: String,
}

/// ApiGatewayV2 GetApiInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetApiInput {
    /// HTTP label (URI path).
    pub api_id: String,
}

/// ApiGatewayV2 GetApiMappingInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetApiMappingInput {
    /// HTTP label (URI path).
    pub api_mapping_id: String,
    /// HTTP label (URI path).
    pub domain_name: String,
}

/// ApiGatewayV2 GetApiMappingsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetApiMappingsInput {
    /// HTTP label (URI path).
    pub domain_name: String,
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetApisInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetApisInput {
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetAuthorizerInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAuthorizerInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub authorizer_id: String,
}

/// ApiGatewayV2 GetAuthorizersInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAuthorizersInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetDeploymentInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDeploymentInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub deployment_id: String,
}

/// ApiGatewayV2 GetDeploymentsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDeploymentsInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetDomainNameInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDomainNameInput {
    /// HTTP label (URI path).
    pub domain_name: String,
}

/// ApiGatewayV2 GetDomainNamesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDomainNamesInput {
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetIntegrationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetIntegrationInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub integration_id: String,
}

/// ApiGatewayV2 GetIntegrationsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetIntegrationsInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetModelInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetModelInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub model_id: String,
}

/// ApiGatewayV2 GetModelTemplateInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetModelTemplateInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub model_id: String,
}

/// ApiGatewayV2 GetModelsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetModelsInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetRouteInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRouteInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub route_id: String,
}

/// ApiGatewayV2 GetRouteResponseInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRouteResponseInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub route_id: String,
    /// HTTP label (URI path).
    pub route_response_id: String,
}

/// ApiGatewayV2 GetRouteResponsesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRouteResponsesInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    /// HTTP label (URI path).
    pub route_id: String,
}

/// ApiGatewayV2 GetRoutesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRoutesInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetStageInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetStageInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP label (URI path).
    pub stage_name: String,
}

/// ApiGatewayV2 GetStagesInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetStagesInput {
    /// HTTP label (URI path).
    pub api_id: String,
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetTagsInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTagsInput {
    /// HTTP label (URI path).
    pub resource_arn: String,
}

/// ApiGatewayV2 GetVpcLinkInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetVpcLinkInput {
    /// HTTP label (URI path).
    pub vpc_link_id: String,
}

/// ApiGatewayV2 GetVpcLinksInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetVpcLinksInput {
    /// HTTP query: `maxResults`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<String>,
    /// HTTP query: `nextToken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 TagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagResourceInput {
    /// HTTP label (URI path).
    pub resource_arn: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 UntagResourceInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UntagResourceInput {
    /// HTTP label (URI path).
    pub resource_arn: String,
    /// HTTP query: `tagKeys`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_keys: Vec<String>,
}

/// ApiGatewayV2 UpdateApiInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApiInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_selection_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors_configuration: Option<Cors>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_execute_api_endpoint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_schema_validation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address_type: Option<IpAddressType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_selection_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// ApiGatewayV2 UpdateApiMappingInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApiMappingInput {
    pub api_id: String,
    /// HTTP label (URI path).
    pub api_mapping_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_key: Option<String>,
    /// HTTP label (URI path).
    pub domain_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

/// ApiGatewayV2 UpdateAuthorizerInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAuthorizerInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_credentials_arn: Option<String>,
    /// HTTP label (URI path).
    pub authorizer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_payload_format_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_result_ttl_in_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_type: Option<AuthorizerType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_simple_responses: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_source: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_validation_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwt_configuration: Option<JWTConfiguration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// ApiGatewayV2 UpdateDomainNameInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDomainNameInput {
    /// HTTP label (URI path).
    pub domain_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_name_configurations: Vec<DomainNameConfiguration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutual_tls_authentication: Option<MutualTlsAuthenticationInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_mode: Option<RoutingMode>,
}

/// ApiGatewayV2 UpdateIntegrationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIntegrationInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<ConnectionType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_handling_strategy: Option<ContentHandlingStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// HTTP label (URI path).
    pub integration_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_subtype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_type: Option<IntegrationType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passthrough_behavior: Option<PassthroughBehavior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_format_version: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_templates: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_parameters: HashMap<String, HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_selection_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_in_millis: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_config: Option<TlsConfigInput>,
}

/// ApiGatewayV2 UpdateModelInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateModelInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// HTTP label (URI path).
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

/// ApiGatewayV2 UpdateRouteInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRouteInput {
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorization_scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_type: Option<AuthorizationType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_models: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, ParameterConstraints>,
    /// HTTP label (URI path).
    pub route_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_selection_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// ApiGatewayV2 UpdateStageInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStageInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_log_settings: Option<AccessLogSettings>,
    /// HTTP label (URI path).
    pub api_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_deploy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_certificate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_route_settings: Option<RouteSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub route_settings: HashMap<String, RouteSettings>,
    /// HTTP label (URI path).
    pub stage_name: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub stage_variables: HashMap<String, String>,
}

/// ApiGatewayV2 UpdateVpcLinkInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVpcLinkInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// HTTP label (URI path).
    pub vpc_link_id: String,
}
