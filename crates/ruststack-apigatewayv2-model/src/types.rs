//! Auto-generated from AWS ApiGatewayV2 Smithy model. DO NOT EDIT.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// ApiGatewayV2 AuthorizationType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AuthorizationType {
    /// Default variant.
    #[default]
    #[serde(rename = "AWS_IAM")]
    AwsIam,
    #[serde(rename = "CUSTOM")]
    Custom,
    #[serde(rename = "JWT")]
    Jwt,
    #[serde(rename = "NONE")]
    None,
}

impl AuthorizationType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AwsIam => "AWS_IAM",
            Self::Custom => "CUSTOM",
            Self::Jwt => "JWT",
            Self::None => "NONE",
        }
    }
}

impl std::fmt::Display for AuthorizationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for AuthorizationType {
    fn from(s: &str) -> Self {
        match s {
            "AWS_IAM" => Self::AwsIam,
            "CUSTOM" => Self::Custom,
            "JWT" => Self::Jwt,
            "NONE" => Self::None,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 AuthorizerType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AuthorizerType {
    /// Default variant.
    #[default]
    #[serde(rename = "JWT")]
    Jwt,
    #[serde(rename = "REQUEST")]
    Request,
}

impl AuthorizerType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jwt => "JWT",
            Self::Request => "REQUEST",
        }
    }
}

impl std::fmt::Display for AuthorizerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for AuthorizerType {
    fn from(s: &str) -> Self {
        match s {
            "JWT" => Self::Jwt,
            "REQUEST" => Self::Request,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 ConnectionType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ConnectionType {
    /// Default variant.
    #[default]
    #[serde(rename = "INTERNET")]
    Internet,
    #[serde(rename = "VPC_LINK")]
    VpcLink,
}

impl ConnectionType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Internet => "INTERNET",
            Self::VpcLink => "VPC_LINK",
        }
    }
}

impl std::fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ConnectionType {
    fn from(s: &str) -> Self {
        match s {
            "INTERNET" => Self::Internet,
            "VPC_LINK" => Self::VpcLink,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 ContentHandlingStrategy enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ContentHandlingStrategy {
    /// Default variant.
    #[default]
    #[serde(rename = "CONVERT_TO_BINARY")]
    ConvertToBinary,
    #[serde(rename = "CONVERT_TO_TEXT")]
    ConvertToText,
}

impl ContentHandlingStrategy {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConvertToBinary => "CONVERT_TO_BINARY",
            Self::ConvertToText => "CONVERT_TO_TEXT",
        }
    }
}

impl std::fmt::Display for ContentHandlingStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ContentHandlingStrategy {
    fn from(s: &str) -> Self {
        match s {
            "CONVERT_TO_BINARY" => Self::ConvertToBinary,
            "CONVERT_TO_TEXT" => Self::ConvertToText,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 DeploymentStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DeploymentStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "DEPLOYED")]
    Deployed,
    #[serde(rename = "FAILED")]
    Failed,
    #[serde(rename = "PENDING")]
    Pending,
}

impl DeploymentStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Deployed => "DEPLOYED",
            Self::Failed => "FAILED",
            Self::Pending => "PENDING",
        }
    }
}

impl std::fmt::Display for DeploymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DeploymentStatus {
    fn from(s: &str) -> Self {
        match s {
            "DEPLOYED" => Self::Deployed,
            "FAILED" => Self::Failed,
            "PENDING" => Self::Pending,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 DomainNameStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DomainNameStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "AVAILABLE")]
    Available,
    #[serde(rename = "PENDING_CERTIFICATE_REIMPORT")]
    PendingCertificateReimport,
    #[serde(rename = "PENDING_OWNERSHIP_VERIFICATION")]
    PendingOwnershipVerification,
    #[serde(rename = "UPDATING")]
    Updating,
}

impl DomainNameStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "AVAILABLE",
            Self::PendingCertificateReimport => "PENDING_CERTIFICATE_REIMPORT",
            Self::PendingOwnershipVerification => "PENDING_OWNERSHIP_VERIFICATION",
            Self::Updating => "UPDATING",
        }
    }
}

impl std::fmt::Display for DomainNameStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for DomainNameStatus {
    fn from(s: &str) -> Self {
        match s {
            "AVAILABLE" => Self::Available,
            "PENDING_CERTIFICATE_REIMPORT" => Self::PendingCertificateReimport,
            "PENDING_OWNERSHIP_VERIFICATION" => Self::PendingOwnershipVerification,
            "UPDATING" => Self::Updating,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 EndpointType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EndpointType {
    /// Default variant.
    #[default]
    #[serde(rename = "EDGE")]
    Edge,
    #[serde(rename = "REGIONAL")]
    Regional,
}

impl EndpointType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Edge => "EDGE",
            Self::Regional => "REGIONAL",
        }
    }
}

impl std::fmt::Display for EndpointType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for EndpointType {
    fn from(s: &str) -> Self {
        match s {
            "EDGE" => Self::Edge,
            "REGIONAL" => Self::Regional,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 IntegrationType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum IntegrationType {
    /// Default variant.
    #[default]
    #[serde(rename = "AWS")]
    Aws,
    #[serde(rename = "AWS_PROXY")]
    AwsProxy,
    #[serde(rename = "HTTP")]
    Http,
    #[serde(rename = "HTTP_PROXY")]
    HttpProxy,
    #[serde(rename = "MOCK")]
    Mock,
}

impl IntegrationType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aws => "AWS",
            Self::AwsProxy => "AWS_PROXY",
            Self::Http => "HTTP",
            Self::HttpProxy => "HTTP_PROXY",
            Self::Mock => "MOCK",
        }
    }
}

impl std::fmt::Display for IntegrationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for IntegrationType {
    fn from(s: &str) -> Self {
        match s {
            "AWS" => Self::Aws,
            "AWS_PROXY" => Self::AwsProxy,
            "HTTP" => Self::Http,
            "HTTP_PROXY" => Self::HttpProxy,
            "MOCK" => Self::Mock,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 IpAddressType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum IpAddressType {
    /// Default variant.
    #[default]
    #[serde(rename = "dualstack")]
    Dualstack,
    #[serde(rename = "ipv4")]
    Ipv4,
}

impl IpAddressType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dualstack => "dualstack",
            Self::Ipv4 => "ipv4",
        }
    }
}

impl std::fmt::Display for IpAddressType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for IpAddressType {
    fn from(s: &str) -> Self {
        match s {
            "dualstack" => Self::Dualstack,
            "ipv4" => Self::Ipv4,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 LoggingLevel enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LoggingLevel {
    /// Default variant.
    #[default]
    #[serde(rename = "ERROR")]
    Error,
    #[serde(rename = "INFO")]
    Info,
    #[serde(rename = "OFF")]
    Off,
}

impl LoggingLevel {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Info => "INFO",
            Self::Off => "OFF",
        }
    }
}

impl std::fmt::Display for LoggingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for LoggingLevel {
    fn from(s: &str) -> Self {
        match s {
            "ERROR" => Self::Error,
            "INFO" => Self::Info,
            "OFF" => Self::Off,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 PassthroughBehavior enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PassthroughBehavior {
    /// Default variant.
    #[default]
    #[serde(rename = "NEVER")]
    Never,
    #[serde(rename = "WHEN_NO_MATCH")]
    WhenNoMatch,
    #[serde(rename = "WHEN_NO_TEMPLATES")]
    WhenNoTemplates,
}

impl PassthroughBehavior {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Never => "NEVER",
            Self::WhenNoMatch => "WHEN_NO_MATCH",
            Self::WhenNoTemplates => "WHEN_NO_TEMPLATES",
        }
    }
}

impl std::fmt::Display for PassthroughBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for PassthroughBehavior {
    fn from(s: &str) -> Self {
        match s {
            "NEVER" => Self::Never,
            "WHEN_NO_MATCH" => Self::WhenNoMatch,
            "WHEN_NO_TEMPLATES" => Self::WhenNoTemplates,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 ProtocolType enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ProtocolType {
    /// Default variant.
    #[default]
    #[serde(rename = "HTTP")]
    Http,
    #[serde(rename = "WEBSOCKET")]
    Websocket,
}

impl ProtocolType {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Http => "HTTP",
            Self::Websocket => "WEBSOCKET",
        }
    }
}

impl std::fmt::Display for ProtocolType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ProtocolType {
    fn from(s: &str) -> Self {
        match s {
            "HTTP" => Self::Http,
            "WEBSOCKET" => Self::Websocket,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 RoutingMode enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RoutingMode {
    /// Default variant.
    #[default]
    #[serde(rename = "API_MAPPING_ONLY")]
    ApiMappingOnly,
    #[serde(rename = "ROUTING_RULE_ONLY")]
    RoutingRuleOnly,
    #[serde(rename = "ROUTING_RULE_THEN_API_MAPPING")]
    RoutingRuleThenApiMapping,
}

impl RoutingMode {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ApiMappingOnly => "API_MAPPING_ONLY",
            Self::RoutingRuleOnly => "ROUTING_RULE_ONLY",
            Self::RoutingRuleThenApiMapping => "ROUTING_RULE_THEN_API_MAPPING",
        }
    }
}

impl std::fmt::Display for RoutingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for RoutingMode {
    fn from(s: &str) -> Self {
        match s {
            "API_MAPPING_ONLY" => Self::ApiMappingOnly,
            "ROUTING_RULE_ONLY" => Self::RoutingRuleOnly,
            "ROUTING_RULE_THEN_API_MAPPING" => Self::RoutingRuleThenApiMapping,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 SecurityPolicy enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SecurityPolicy {
    /// Default variant.
    #[default]
    #[serde(rename = "TLS_1_0")]
    Tls10,
    #[serde(rename = "TLS_1_2")]
    Tls12,
}

impl SecurityPolicy {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tls10 => "TLS_1_0",
            Self::Tls12 => "TLS_1_2",
        }
    }
}

impl std::fmt::Display for SecurityPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for SecurityPolicy {
    fn from(s: &str) -> Self {
        match s {
            "TLS_1_0" => Self::Tls10,
            "TLS_1_2" => Self::Tls12,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 VpcLinkStatus enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum VpcLinkStatus {
    /// Default variant.
    #[default]
    #[serde(rename = "AVAILABLE")]
    Available,
    #[serde(rename = "DELETING")]
    Deleting,
    #[serde(rename = "FAILED")]
    Failed,
    #[serde(rename = "INACTIVE")]
    Inactive,
    #[serde(rename = "PENDING")]
    Pending,
}

impl VpcLinkStatus {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "AVAILABLE",
            Self::Deleting => "DELETING",
            Self::Failed => "FAILED",
            Self::Inactive => "INACTIVE",
            Self::Pending => "PENDING",
        }
    }
}

impl std::fmt::Display for VpcLinkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for VpcLinkStatus {
    fn from(s: &str) -> Self {
        match s {
            "AVAILABLE" => Self::Available,
            "DELETING" => Self::Deleting,
            "FAILED" => Self::Failed,
            "INACTIVE" => Self::Inactive,
            "PENDING" => Self::Pending,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 VpcLinkVersion enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum VpcLinkVersion {
    /// Default variant.
    #[default]
    V2,
}

impl VpcLinkVersion {
    /// Returns the string value of this enum variant.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V2 => "V2",
        }
    }
}

impl std::fmt::Display for VpcLinkVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for VpcLinkVersion {
    fn from(s: &str) -> Self {
        match s {
            "V2" => Self::V2,
            _ => Self::default(),
        }
    }
}

/// ApiGatewayV2 AccessLogSettings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessLogSettings {
    #[serde(rename = "DestinationArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_arn: Option<String>,
    #[serde(rename = "Format")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

/// ApiGatewayV2 Api.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Api {
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
    pub name: String,
    #[serde(rename = "ProtocolType")]
    pub protocol_type: ProtocolType,
    #[serde(rename = "RouteSelectionExpression")]
    pub route_selection_expression: String,
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

/// ApiGatewayV2 ApiMapping.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiMapping {
    #[serde(rename = "ApiId")]
    pub api_id: String,
    #[serde(rename = "ApiMappingId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_id: Option<String>,
    #[serde(rename = "ApiMappingKey")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_key: Option<String>,
    #[serde(rename = "Stage")]
    pub stage: String,
}

/// ApiGatewayV2 Authorizer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Authorizer {
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
    pub name: String,
}

/// ApiGatewayV2 Cors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cors {
    #[serde(rename = "AllowCredentials")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_credentials: Option<bool>,
    #[serde(rename = "AllowHeaders")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_headers: Vec<String>,
    #[serde(rename = "AllowMethods")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_methods: Vec<String>,
    #[serde(rename = "AllowOrigins")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_origins: Vec<String>,
    #[serde(rename = "ExposeHeaders")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expose_headers: Vec<String>,
    #[serde(rename = "MaxAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age: Option<i32>,
}

/// ApiGatewayV2 Deployment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
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

/// ApiGatewayV2 DomainName.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainName {
    #[serde(rename = "ApiMappingSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_mapping_selection_expression: Option<String>,
    #[serde(rename = "DomainName")]
    pub domain_name: String,
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

/// ApiGatewayV2 DomainNameConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainNameConfiguration {
    #[serde(rename = "ApiGatewayDomainName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_gateway_domain_name: Option<String>,
    #[serde(rename = "CertificateArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_arn: Option<String>,
    #[serde(rename = "CertificateName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_name: Option<String>,
    #[serde(rename = "CertificateUploadDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_upload_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "DomainNameStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_name_status: Option<DomainNameStatus>,
    #[serde(rename = "DomainNameStatusMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_name_status_message: Option<String>,
    #[serde(rename = "EndpointType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_type: Option<EndpointType>,
    #[serde(rename = "HostedZoneId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosted_zone_id: Option<String>,
    #[serde(rename = "IpAddressType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address_type: Option<IpAddressType>,
    #[serde(rename = "OwnershipVerificationCertificateArn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ownership_verification_certificate_arn: Option<String>,
    #[serde(rename = "SecurityPolicy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_policy: Option<SecurityPolicy>,
}

/// ApiGatewayV2 Integration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
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

/// ApiGatewayV2 JWTConfiguration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JWTConfiguration {
    #[serde(rename = "Audience")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audience: Vec<String>,
    #[serde(rename = "Issuer")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
}

/// ApiGatewayV2 Model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
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
    pub name: String,
    #[serde(rename = "Schema")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

/// ApiGatewayV2 MutualTlsAuthentication.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutualTlsAuthentication {
    #[serde(rename = "TruststoreUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truststore_uri: Option<String>,
    #[serde(rename = "TruststoreVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truststore_version: Option<String>,
    #[serde(rename = "TruststoreWarnings")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub truststore_warnings: Vec<String>,
}

/// ApiGatewayV2 MutualTlsAuthenticationInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutualTlsAuthenticationInput {
    #[serde(rename = "TruststoreUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truststore_uri: Option<String>,
    #[serde(rename = "TruststoreVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truststore_version: Option<String>,
}

/// ApiGatewayV2 ParameterConstraints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterConstraints {
    #[serde(rename = "Required")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// ApiGatewayV2 Route.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
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
    pub route_key: String,
    #[serde(rename = "RouteResponseSelectionExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_response_selection_expression: Option<String>,
    #[serde(rename = "Target")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// ApiGatewayV2 RouteResponse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteResponse {
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
    pub route_response_key: String,
}

/// ApiGatewayV2 RouteSettings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteSettings {
    #[serde(rename = "DataTraceEnabled")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_trace_enabled: Option<bool>,
    #[serde(rename = "DetailedMetricsEnabled")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detailed_metrics_enabled: Option<bool>,
    #[serde(rename = "LoggingLevel")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging_level: Option<LoggingLevel>,
    #[serde(rename = "ThrottlingBurstLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throttling_burst_limit: Option<i32>,
    #[serde(rename = "ThrottlingRateLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throttling_rate_limit: Option<f64>,
}

/// ApiGatewayV2 Stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
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
    pub stage_name: String,
    #[serde(rename = "StageVariables")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub stage_variables: HashMap<String, String>,
    #[serde(rename = "Tags")]
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
}

/// ApiGatewayV2 TlsConfig.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsConfig {
    #[serde(rename = "ServerNameToVerify")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name_to_verify: Option<String>,
}

/// ApiGatewayV2 TlsConfigInput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsConfigInput {
    #[serde(rename = "ServerNameToVerify")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name_to_verify: Option<String>,
}

/// ApiGatewayV2 VpcLink.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VpcLink {
    #[serde(rename = "CreatedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(rename = "Name")]
    pub name: String,
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
    pub vpc_link_id: String,
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
