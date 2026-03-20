//! Auto-generated from AWS ApiGatewayV2 Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    AccessLogSettings, Api, ApiMapping, AuthorizationType, Authorizer, AuthorizerType,
    ConnectionType, ContentHandlingStrategy, Cors, Deployment, DeploymentStatus, DomainName,
    DomainNameConfiguration, Integration, IntegrationType, IpAddressType, JWTConfiguration, Model,
    MutualTlsAuthentication, ParameterConstraints, PassthroughBehavior, ProtocolType, Route,
    RouteResponse, RouteSettings, RoutingMode, Stage, TlsConfig, VpcLink, VpcLinkStatus,
    VpcLinkVersion,
};

/// ApiGatewayV2 CreateApiMappingResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiMappingResponse {
    #[serde(rename = "ApiId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_id: Option<String>,
    #[serde(rename = "ApiMappingId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_id: Option<String>,
    #[serde(rename = "ApiMappingKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_key: Option<String>,
    #[serde(rename = "Stage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

/// ApiGatewayV2 CreateApiResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiResponse {
    #[serde(rename = "ApiEndpoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_endpoint: Option<String>,
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "ApiId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_id: Option<String>,
    #[serde(rename = "ApiKeySelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_selection_expression: Option<String>,
    #[serde(rename = "CorsConfiguration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors_configuration: Option<Cors>,
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "DisableExecuteApiEndpoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_execute_api_endpoint: Option<bool>,
    #[serde(rename = "DisableSchemaValidation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_schema_validation: Option<bool>,
    #[serde(rename = "ImportInfo")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import_info: Vec<String>,
    #[serde(rename = "IpAddressType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address_type: Option<IpAddressType>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "ProtocolType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_type: Option<ProtocolType>,
    #[serde(rename = "RouteSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_selection_expression: Option<String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    #[serde(rename = "Version")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(rename = "Warnings")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// ApiGatewayV2 CreateAuthorizerResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAuthorizerResponse {
    #[serde(rename = "AuthorizerCredentialsArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_credentials_arn: Option<String>,
    #[serde(rename = "AuthorizerId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_id: Option<String>,
    #[serde(rename = "AuthorizerPayloadFormatVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_payload_format_version: Option<String>,
    #[serde(rename = "AuthorizerResultTtlInSeconds")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_result_ttl_in_seconds: Option<i32>,
    #[serde(rename = "AuthorizerType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_type: Option<AuthorizerType>,
    #[serde(rename = "AuthorizerUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_uri: Option<String>,
    #[serde(rename = "EnableSimpleResponses")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_simple_responses: Option<bool>,
    #[serde(rename = "IdentitySource")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_source: Vec<String>,
    #[serde(rename = "IdentityValidationExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_validation_expression: Option<String>,
    #[serde(rename = "JwtConfiguration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwt_configuration: Option<JWTConfiguration>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// ApiGatewayV2 CreateDeploymentResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDeploymentResponse {
    #[serde(rename = "AutoDeployed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_deployed: Option<bool>,
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "DeploymentId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(rename = "DeploymentStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_status: Option<DeploymentStatus>,
    #[serde(rename = "DeploymentStatusMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_status_message: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// ApiGatewayV2 CreateDomainNameResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDomainNameResponse {
    #[serde(rename = "ApiMappingSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_selection_expression: Option<String>,
    #[serde(rename = "DomainName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_name: Option<String>,
    #[serde(rename = "DomainNameArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_name_arn: Option<String>,
    #[serde(rename = "DomainNameConfigurations")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_name_configurations: Vec<DomainNameConfiguration>,
    #[serde(rename = "MutualTlsAuthentication")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutual_tls_authentication: Option<MutualTlsAuthentication>,
    #[serde(rename = "RoutingMode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_mode: Option<RoutingMode>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 CreateIntegrationResult.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIntegrationResult {
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "ConnectionId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(rename = "ConnectionType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<ConnectionType>,
    #[serde(rename = "ContentHandlingStrategy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_handling_strategy: Option<ContentHandlingStrategy>,
    #[serde(rename = "CredentialsArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_arn: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "IntegrationId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_id: Option<String>,
    #[serde(rename = "IntegrationMethod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_method: Option<String>,
    #[serde(rename = "IntegrationResponseSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_response_selection_expression: Option<String>,
    #[serde(rename = "IntegrationSubtype")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_subtype: Option<String>,
    #[serde(rename = "IntegrationType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_type: Option<IntegrationType>,
    #[serde(rename = "IntegrationUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_uri: Option<String>,
    #[serde(rename = "PassthroughBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passthrough_behavior: Option<PassthroughBehavior>,
    #[serde(rename = "PayloadFormatVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_format_version: Option<String>,
    #[serde(rename = "RequestParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, String>,
    #[serde(rename = "RequestTemplates")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_templates: HashMap<String, String>,
    #[serde(rename = "ResponseParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_parameters: HashMap<String, HashMap<String, String>>,
    #[serde(rename = "TemplateSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_selection_expression: Option<String>,
    #[serde(rename = "TimeoutInMillis")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_in_millis: Option<i32>,
    #[serde(rename = "TlsConfig")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_config: Option<TlsConfig>,
}

/// ApiGatewayV2 CreateModelResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateModelResponse {
    #[serde(rename = "ContentType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "ModelId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "Schema")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

/// ApiGatewayV2 CreateRouteResponseResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRouteResponseResponse {
    #[serde(rename = "ModelSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection_expression: Option<String>,
    #[serde(rename = "ResponseModels")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_models: HashMap<String, String>,
    #[serde(rename = "ResponseParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_parameters: HashMap<String, ParameterConstraints>,
    #[serde(rename = "RouteResponseId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_id: Option<String>,
    #[serde(rename = "RouteResponseKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_key: Option<String>,
}

/// ApiGatewayV2 CreateRouteResult.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRouteResult {
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "ApiKeyRequired")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_required: Option<bool>,
    #[serde(rename = "AuthorizationScopes")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorization_scopes: Vec<String>,
    #[serde(rename = "AuthorizationType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_type: Option<AuthorizationType>,
    #[serde(rename = "AuthorizerId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_id: Option<String>,
    #[serde(rename = "ModelSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection_expression: Option<String>,
    #[serde(rename = "OperationName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    #[serde(rename = "RequestModels")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_models: HashMap<String, String>,
    #[serde(rename = "RequestParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, ParameterConstraints>,
    #[serde(rename = "RouteId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_id: Option<String>,
    #[serde(rename = "RouteKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(rename = "RouteResponseSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_selection_expression: Option<String>,
    #[serde(rename = "Target")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// ApiGatewayV2 CreateStageResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateStageResponse {
    #[serde(rename = "AccessLogSettings")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_log_settings: Option<AccessLogSettings>,
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "AutoDeploy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_deploy: Option<bool>,
    #[serde(rename = "ClientCertificateId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_certificate_id: Option<String>,
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "DefaultRouteSettings")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_route_settings: Option<RouteSettings>,
    #[serde(rename = "DeploymentId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "LastDeploymentStatusMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_deployment_status_message: Option<String>,
    #[serde(rename = "LastUpdatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "RouteSettings")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub route_settings: HashMap<String, RouteSettings>,
    #[serde(rename = "StageName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_name: Option<String>,
    #[serde(rename = "StageVariables")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub stage_variables: HashMap<String, String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 CreateVpcLinkResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVpcLinkResponse {
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "SecurityGroupIds")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_group_ids: Vec<String>,
    #[serde(rename = "SubnetIds")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subnet_ids: Vec<String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    #[serde(rename = "VpcLinkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_id: Option<String>,
    #[serde(rename = "VpcLinkStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_status: Option<VpcLinkStatus>,
    #[serde(rename = "VpcLinkStatusMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_status_message: Option<String>,
    #[serde(rename = "VpcLinkVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_version: Option<VpcLinkVersion>,
}

/// ApiGatewayV2 DeleteVpcLinkResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteVpcLinkResponse {}

/// ApiGatewayV2 GetApiMappingResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetApiMappingResponse {
    #[serde(rename = "ApiId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_id: Option<String>,
    #[serde(rename = "ApiMappingId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_id: Option<String>,
    #[serde(rename = "ApiMappingKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_key: Option<String>,
    #[serde(rename = "Stage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

/// ApiGatewayV2 GetApiMappingsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetApiMappingsResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ApiMapping>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetApiResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetApiResponse {
    #[serde(rename = "ApiEndpoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_endpoint: Option<String>,
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "ApiId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_id: Option<String>,
    #[serde(rename = "ApiKeySelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_selection_expression: Option<String>,
    #[serde(rename = "CorsConfiguration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors_configuration: Option<Cors>,
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "DisableExecuteApiEndpoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_execute_api_endpoint: Option<bool>,
    #[serde(rename = "DisableSchemaValidation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_schema_validation: Option<bool>,
    #[serde(rename = "ImportInfo")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import_info: Vec<String>,
    #[serde(rename = "IpAddressType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address_type: Option<IpAddressType>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "ProtocolType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_type: Option<ProtocolType>,
    #[serde(rename = "RouteSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_selection_expression: Option<String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    #[serde(rename = "Version")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(rename = "Warnings")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// ApiGatewayV2 GetApisResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetApisResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Api>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetAuthorizerResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAuthorizerResponse {
    #[serde(rename = "AuthorizerCredentialsArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_credentials_arn: Option<String>,
    #[serde(rename = "AuthorizerId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_id: Option<String>,
    #[serde(rename = "AuthorizerPayloadFormatVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_payload_format_version: Option<String>,
    #[serde(rename = "AuthorizerResultTtlInSeconds")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_result_ttl_in_seconds: Option<i32>,
    #[serde(rename = "AuthorizerType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_type: Option<AuthorizerType>,
    #[serde(rename = "AuthorizerUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_uri: Option<String>,
    #[serde(rename = "EnableSimpleResponses")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_simple_responses: Option<bool>,
    #[serde(rename = "IdentitySource")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_source: Vec<String>,
    #[serde(rename = "IdentityValidationExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_validation_expression: Option<String>,
    #[serde(rename = "JwtConfiguration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwt_configuration: Option<JWTConfiguration>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// ApiGatewayV2 GetAuthorizersResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAuthorizersResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Authorizer>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetDeploymentResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDeploymentResponse {
    #[serde(rename = "AutoDeployed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_deployed: Option<bool>,
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "DeploymentId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(rename = "DeploymentStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_status: Option<DeploymentStatus>,
    #[serde(rename = "DeploymentStatusMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_status_message: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// ApiGatewayV2 GetDeploymentsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDeploymentsResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Deployment>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetDomainNameResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDomainNameResponse {
    #[serde(rename = "ApiMappingSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_selection_expression: Option<String>,
    #[serde(rename = "DomainName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_name: Option<String>,
    #[serde(rename = "DomainNameArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_name_arn: Option<String>,
    #[serde(rename = "DomainNameConfigurations")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_name_configurations: Vec<DomainNameConfiguration>,
    #[serde(rename = "MutualTlsAuthentication")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutual_tls_authentication: Option<MutualTlsAuthentication>,
    #[serde(rename = "RoutingMode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_mode: Option<RoutingMode>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 GetDomainNamesResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDomainNamesResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DomainName>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetIntegrationResult.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetIntegrationResult {
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "ConnectionId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(rename = "ConnectionType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<ConnectionType>,
    #[serde(rename = "ContentHandlingStrategy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_handling_strategy: Option<ContentHandlingStrategy>,
    #[serde(rename = "CredentialsArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_arn: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "IntegrationId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_id: Option<String>,
    #[serde(rename = "IntegrationMethod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_method: Option<String>,
    #[serde(rename = "IntegrationResponseSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_response_selection_expression: Option<String>,
    #[serde(rename = "IntegrationSubtype")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_subtype: Option<String>,
    #[serde(rename = "IntegrationType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_type: Option<IntegrationType>,
    #[serde(rename = "IntegrationUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_uri: Option<String>,
    #[serde(rename = "PassthroughBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passthrough_behavior: Option<PassthroughBehavior>,
    #[serde(rename = "PayloadFormatVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_format_version: Option<String>,
    #[serde(rename = "RequestParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, String>,
    #[serde(rename = "RequestTemplates")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_templates: HashMap<String, String>,
    #[serde(rename = "ResponseParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_parameters: HashMap<String, HashMap<String, String>>,
    #[serde(rename = "TemplateSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_selection_expression: Option<String>,
    #[serde(rename = "TimeoutInMillis")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_in_millis: Option<i32>,
    #[serde(rename = "TlsConfig")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_config: Option<TlsConfig>,
}

/// ApiGatewayV2 GetIntegrationsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetIntegrationsResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Integration>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetModelResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetModelResponse {
    #[serde(rename = "ContentType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "ModelId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "Schema")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

/// ApiGatewayV2 GetModelTemplateResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetModelTemplateResponse {
    #[serde(rename = "Value")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// ApiGatewayV2 GetModelsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetModelsResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Model>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetRouteResponseResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRouteResponseResponse {
    #[serde(rename = "ModelSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection_expression: Option<String>,
    #[serde(rename = "ResponseModels")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_models: HashMap<String, String>,
    #[serde(rename = "ResponseParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_parameters: HashMap<String, ParameterConstraints>,
    #[serde(rename = "RouteResponseId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_id: Option<String>,
    #[serde(rename = "RouteResponseKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_key: Option<String>,
}

/// ApiGatewayV2 GetRouteResponsesResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRouteResponsesResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<RouteResponse>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetRouteResult.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRouteResult {
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "ApiKeyRequired")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_required: Option<bool>,
    #[serde(rename = "AuthorizationScopes")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorization_scopes: Vec<String>,
    #[serde(rename = "AuthorizationType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_type: Option<AuthorizationType>,
    #[serde(rename = "AuthorizerId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_id: Option<String>,
    #[serde(rename = "ModelSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection_expression: Option<String>,
    #[serde(rename = "OperationName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    #[serde(rename = "RequestModels")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_models: HashMap<String, String>,
    #[serde(rename = "RequestParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, ParameterConstraints>,
    #[serde(rename = "RouteId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_id: Option<String>,
    #[serde(rename = "RouteKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(rename = "RouteResponseSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_selection_expression: Option<String>,
    #[serde(rename = "Target")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// ApiGatewayV2 GetRoutesResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRoutesResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Route>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetStageResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetStageResponse {
    #[serde(rename = "AccessLogSettings")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_log_settings: Option<AccessLogSettings>,
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "AutoDeploy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_deploy: Option<bool>,
    #[serde(rename = "ClientCertificateId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_certificate_id: Option<String>,
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "DefaultRouteSettings")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_route_settings: Option<RouteSettings>,
    #[serde(rename = "DeploymentId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "LastDeploymentStatusMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_deployment_status_message: Option<String>,
    #[serde(rename = "LastUpdatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "RouteSettings")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub route_settings: HashMap<String, RouteSettings>,
    #[serde(rename = "StageName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_name: Option<String>,
    #[serde(rename = "StageVariables")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub stage_variables: HashMap<String, String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 GetStagesResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetStagesResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Stage>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 GetTagsResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTagsResponse {
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 GetVpcLinkResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetVpcLinkResponse {
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "SecurityGroupIds")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_group_ids: Vec<String>,
    #[serde(rename = "SubnetIds")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subnet_ids: Vec<String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    #[serde(rename = "VpcLinkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_id: Option<String>,
    #[serde(rename = "VpcLinkStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_status: Option<VpcLinkStatus>,
    #[serde(rename = "VpcLinkStatusMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_status_message: Option<String>,
    #[serde(rename = "VpcLinkVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_version: Option<VpcLinkVersion>,
}

/// ApiGatewayV2 GetVpcLinksResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetVpcLinksResponse {
    #[serde(rename = "Items")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<VpcLink>,
    #[serde(rename = "NextToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// ApiGatewayV2 TagResourceResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagResourceResponse {}

/// ApiGatewayV2 UpdateApiMappingResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApiMappingResponse {
    #[serde(rename = "ApiId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_id: Option<String>,
    #[serde(rename = "ApiMappingId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_id: Option<String>,
    #[serde(rename = "ApiMappingKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_key: Option<String>,
    #[serde(rename = "Stage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

/// ApiGatewayV2 UpdateApiResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApiResponse {
    #[serde(rename = "ApiEndpoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_endpoint: Option<String>,
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "ApiId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_id: Option<String>,
    #[serde(rename = "ApiKeySelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_selection_expression: Option<String>,
    #[serde(rename = "CorsConfiguration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors_configuration: Option<Cors>,
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "DisableExecuteApiEndpoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_execute_api_endpoint: Option<bool>,
    #[serde(rename = "DisableSchemaValidation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_schema_validation: Option<bool>,
    #[serde(rename = "ImportInfo")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import_info: Vec<String>,
    #[serde(rename = "IpAddressType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address_type: Option<IpAddressType>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "ProtocolType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_type: Option<ProtocolType>,
    #[serde(rename = "RouteSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_selection_expression: Option<String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    #[serde(rename = "Version")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(rename = "Warnings")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// ApiGatewayV2 UpdateAuthorizerResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAuthorizerResponse {
    #[serde(rename = "AuthorizerCredentialsArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_credentials_arn: Option<String>,
    #[serde(rename = "AuthorizerId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_id: Option<String>,
    #[serde(rename = "AuthorizerPayloadFormatVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_payload_format_version: Option<String>,
    #[serde(rename = "AuthorizerResultTtlInSeconds")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_result_ttl_in_seconds: Option<i32>,
    #[serde(rename = "AuthorizerType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_type: Option<AuthorizerType>,
    #[serde(rename = "AuthorizerUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_uri: Option<String>,
    #[serde(rename = "EnableSimpleResponses")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_simple_responses: Option<bool>,
    #[serde(rename = "IdentitySource")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_source: Vec<String>,
    #[serde(rename = "IdentityValidationExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_validation_expression: Option<String>,
    #[serde(rename = "JwtConfiguration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwt_configuration: Option<JWTConfiguration>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// ApiGatewayV2 UpdateDomainNameResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDomainNameResponse {
    #[serde(rename = "ApiMappingSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_selection_expression: Option<String>,
    #[serde(rename = "DomainName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_name: Option<String>,
    #[serde(rename = "DomainNameArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_name_arn: Option<String>,
    #[serde(rename = "DomainNameConfigurations")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_name_configurations: Vec<DomainNameConfiguration>,
    #[serde(rename = "MutualTlsAuthentication")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutual_tls_authentication: Option<MutualTlsAuthentication>,
    #[serde(rename = "RoutingMode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_mode: Option<RoutingMode>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 UpdateIntegrationResult.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIntegrationResult {
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "ConnectionId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(rename = "ConnectionType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<ConnectionType>,
    #[serde(rename = "ContentHandlingStrategy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_handling_strategy: Option<ContentHandlingStrategy>,
    #[serde(rename = "CredentialsArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_arn: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "IntegrationId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_id: Option<String>,
    #[serde(rename = "IntegrationMethod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_method: Option<String>,
    #[serde(rename = "IntegrationResponseSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_response_selection_expression: Option<String>,
    #[serde(rename = "IntegrationSubtype")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_subtype: Option<String>,
    #[serde(rename = "IntegrationType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_type: Option<IntegrationType>,
    #[serde(rename = "IntegrationUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_uri: Option<String>,
    #[serde(rename = "PassthroughBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passthrough_behavior: Option<PassthroughBehavior>,
    #[serde(rename = "PayloadFormatVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_format_version: Option<String>,
    #[serde(rename = "RequestParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, String>,
    #[serde(rename = "RequestTemplates")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_templates: HashMap<String, String>,
    #[serde(rename = "ResponseParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub response_parameters: HashMap<String, HashMap<String, String>>,
    #[serde(rename = "TemplateSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_selection_expression: Option<String>,
    #[serde(rename = "TimeoutInMillis")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_in_millis: Option<i32>,
    #[serde(rename = "TlsConfig")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_config: Option<TlsConfig>,
}

/// ApiGatewayV2 UpdateModelResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateModelResponse {
    #[serde(rename = "ContentType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "ModelId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "Schema")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

/// ApiGatewayV2 UpdateRouteResult.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRouteResult {
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "ApiKeyRequired")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_required: Option<bool>,
    #[serde(rename = "AuthorizationScopes")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorization_scopes: Vec<String>,
    #[serde(rename = "AuthorizationType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_type: Option<AuthorizationType>,
    #[serde(rename = "AuthorizerId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizer_id: Option<String>,
    #[serde(rename = "ModelSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection_expression: Option<String>,
    #[serde(rename = "OperationName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    #[serde(rename = "RequestModels")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_models: HashMap<String, String>,
    #[serde(rename = "RequestParameters")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub request_parameters: HashMap<String, ParameterConstraints>,
    #[serde(rename = "RouteId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_id: Option<String>,
    #[serde(rename = "RouteKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(rename = "RouteResponseSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_selection_expression: Option<String>,
    #[serde(rename = "Target")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// ApiGatewayV2 UpdateStageResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStageResponse {
    #[serde(rename = "AccessLogSettings")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_log_settings: Option<AccessLogSettings>,
    #[serde(rename = "ApiGatewayManaged")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_managed: Option<bool>,
    #[serde(rename = "AutoDeploy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_deploy: Option<bool>,
    #[serde(rename = "ClientCertificateId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_certificate_id: Option<String>,
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "DefaultRouteSettings")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_route_settings: Option<RouteSettings>,
    #[serde(rename = "DeploymentId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(rename = "Description")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "LastDeploymentStatusMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_deployment_status_message: Option<String>,
    #[serde(rename = "LastUpdatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "RouteSettings")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub route_settings: HashMap<String, RouteSettings>,
    #[serde(rename = "StageName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_name: Option<String>,
    #[serde(rename = "StageVariables")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub stage_variables: HashMap<String, String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 UpdateVpcLinkResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVpcLinkResponse {
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "Name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "SecurityGroupIds")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_group_ids: Vec<String>,
    #[serde(rename = "SubnetIds")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subnet_ids: Vec<String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    #[serde(rename = "VpcLinkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_id: Option<String>,
    #[serde(rename = "VpcLinkStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_status: Option<VpcLinkStatus>,
    #[serde(rename = "VpcLinkStatusMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_status_message: Option<String>,
    #[serde(rename = "VpcLinkVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpc_link_version: Option<VpcLinkVersion>,
}
