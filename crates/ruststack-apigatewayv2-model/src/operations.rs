//! Auto-generated from AWS ApiGatewayV2 Smithy model. DO NOT EDIT.

/// All supported ApiGatewayV2 operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiGatewayV2Operation {
    /// The CreateApi operation.
    CreateApi,
    /// The GetApi operation.
    GetApi,
    /// The UpdateApi operation.
    UpdateApi,
    /// The DeleteApi operation.
    DeleteApi,
    /// The GetApis operation.
    GetApis,
    /// The CreateRoute operation.
    CreateRoute,
    /// The GetRoute operation.
    GetRoute,
    /// The UpdateRoute operation.
    UpdateRoute,
    /// The DeleteRoute operation.
    DeleteRoute,
    /// The GetRoutes operation.
    GetRoutes,
    /// The CreateIntegration operation.
    CreateIntegration,
    /// The GetIntegration operation.
    GetIntegration,
    /// The UpdateIntegration operation.
    UpdateIntegration,
    /// The DeleteIntegration operation.
    DeleteIntegration,
    /// The GetIntegrations operation.
    GetIntegrations,
    /// The CreateStage operation.
    CreateStage,
    /// The GetStage operation.
    GetStage,
    /// The UpdateStage operation.
    UpdateStage,
    /// The DeleteStage operation.
    DeleteStage,
    /// The GetStages operation.
    GetStages,
    /// The CreateDeployment operation.
    CreateDeployment,
    /// The GetDeployment operation.
    GetDeployment,
    /// The DeleteDeployment operation.
    DeleteDeployment,
    /// The GetDeployments operation.
    GetDeployments,
    /// The CreateRouteResponse operation.
    CreateRouteResponse,
    /// The GetRouteResponse operation.
    GetRouteResponse,
    /// The DeleteRouteResponse operation.
    DeleteRouteResponse,
    /// The GetRouteResponses operation.
    GetRouteResponses,
    /// The CreateAuthorizer operation.
    CreateAuthorizer,
    /// The GetAuthorizer operation.
    GetAuthorizer,
    /// The UpdateAuthorizer operation.
    UpdateAuthorizer,
    /// The DeleteAuthorizer operation.
    DeleteAuthorizer,
    /// The GetAuthorizers operation.
    GetAuthorizers,
    /// The CreateModel operation.
    CreateModel,
    /// The GetModel operation.
    GetModel,
    /// The UpdateModel operation.
    UpdateModel,
    /// The DeleteModel operation.
    DeleteModel,
    /// The GetModels operation.
    GetModels,
    /// The GetModelTemplate operation.
    GetModelTemplate,
    /// The CreateDomainName operation.
    CreateDomainName,
    /// The GetDomainName operation.
    GetDomainName,
    /// The UpdateDomainName operation.
    UpdateDomainName,
    /// The DeleteDomainName operation.
    DeleteDomainName,
    /// The GetDomainNames operation.
    GetDomainNames,
    /// The CreateVpcLink operation.
    CreateVpcLink,
    /// The GetVpcLink operation.
    GetVpcLink,
    /// The UpdateVpcLink operation.
    UpdateVpcLink,
    /// The DeleteVpcLink operation.
    DeleteVpcLink,
    /// The GetVpcLinks operation.
    GetVpcLinks,
    /// The TagResource operation.
    TagResource,
    /// The UntagResource operation.
    UntagResource,
    /// The GetTags operation.
    GetTags,
    /// The CreateApiMapping operation.
    CreateApiMapping,
    /// The GetApiMapping operation.
    GetApiMapping,
    /// The UpdateApiMapping operation.
    UpdateApiMapping,
    /// The DeleteApiMapping operation.
    DeleteApiMapping,
    /// The GetApiMappings operation.
    GetApiMappings,
}

impl ApiGatewayV2Operation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateApi => "CreateApi",
            Self::GetApi => "GetApi",
            Self::UpdateApi => "UpdateApi",
            Self::DeleteApi => "DeleteApi",
            Self::GetApis => "GetApis",
            Self::CreateRoute => "CreateRoute",
            Self::GetRoute => "GetRoute",
            Self::UpdateRoute => "UpdateRoute",
            Self::DeleteRoute => "DeleteRoute",
            Self::GetRoutes => "GetRoutes",
            Self::CreateIntegration => "CreateIntegration",
            Self::GetIntegration => "GetIntegration",
            Self::UpdateIntegration => "UpdateIntegration",
            Self::DeleteIntegration => "DeleteIntegration",
            Self::GetIntegrations => "GetIntegrations",
            Self::CreateStage => "CreateStage",
            Self::GetStage => "GetStage",
            Self::UpdateStage => "UpdateStage",
            Self::DeleteStage => "DeleteStage",
            Self::GetStages => "GetStages",
            Self::CreateDeployment => "CreateDeployment",
            Self::GetDeployment => "GetDeployment",
            Self::DeleteDeployment => "DeleteDeployment",
            Self::GetDeployments => "GetDeployments",
            Self::CreateRouteResponse => "CreateRouteResponse",
            Self::GetRouteResponse => "GetRouteResponse",
            Self::DeleteRouteResponse => "DeleteRouteResponse",
            Self::GetRouteResponses => "GetRouteResponses",
            Self::CreateAuthorizer => "CreateAuthorizer",
            Self::GetAuthorizer => "GetAuthorizer",
            Self::UpdateAuthorizer => "UpdateAuthorizer",
            Self::DeleteAuthorizer => "DeleteAuthorizer",
            Self::GetAuthorizers => "GetAuthorizers",
            Self::CreateModel => "CreateModel",
            Self::GetModel => "GetModel",
            Self::UpdateModel => "UpdateModel",
            Self::DeleteModel => "DeleteModel",
            Self::GetModels => "GetModels",
            Self::GetModelTemplate => "GetModelTemplate",
            Self::CreateDomainName => "CreateDomainName",
            Self::GetDomainName => "GetDomainName",
            Self::UpdateDomainName => "UpdateDomainName",
            Self::DeleteDomainName => "DeleteDomainName",
            Self::GetDomainNames => "GetDomainNames",
            Self::CreateVpcLink => "CreateVpcLink",
            Self::GetVpcLink => "GetVpcLink",
            Self::UpdateVpcLink => "UpdateVpcLink",
            Self::DeleteVpcLink => "DeleteVpcLink",
            Self::GetVpcLinks => "GetVpcLinks",
            Self::TagResource => "TagResource",
            Self::UntagResource => "UntagResource",
            Self::GetTags => "GetTags",
            Self::CreateApiMapping => "CreateApiMapping",
            Self::GetApiMapping => "GetApiMapping",
            Self::UpdateApiMapping => "UpdateApiMapping",
            Self::DeleteApiMapping => "DeleteApiMapping",
            Self::GetApiMappings => "GetApiMappings",
        }
    }

    /// Parse an operation name string into an ApiGatewayV2Operation.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateApi" => Some(Self::CreateApi),
            "GetApi" => Some(Self::GetApi),
            "UpdateApi" => Some(Self::UpdateApi),
            "DeleteApi" => Some(Self::DeleteApi),
            "GetApis" => Some(Self::GetApis),
            "CreateRoute" => Some(Self::CreateRoute),
            "GetRoute" => Some(Self::GetRoute),
            "UpdateRoute" => Some(Self::UpdateRoute),
            "DeleteRoute" => Some(Self::DeleteRoute),
            "GetRoutes" => Some(Self::GetRoutes),
            "CreateIntegration" => Some(Self::CreateIntegration),
            "GetIntegration" => Some(Self::GetIntegration),
            "UpdateIntegration" => Some(Self::UpdateIntegration),
            "DeleteIntegration" => Some(Self::DeleteIntegration),
            "GetIntegrations" => Some(Self::GetIntegrations),
            "CreateStage" => Some(Self::CreateStage),
            "GetStage" => Some(Self::GetStage),
            "UpdateStage" => Some(Self::UpdateStage),
            "DeleteStage" => Some(Self::DeleteStage),
            "GetStages" => Some(Self::GetStages),
            "CreateDeployment" => Some(Self::CreateDeployment),
            "GetDeployment" => Some(Self::GetDeployment),
            "DeleteDeployment" => Some(Self::DeleteDeployment),
            "GetDeployments" => Some(Self::GetDeployments),
            "CreateRouteResponse" => Some(Self::CreateRouteResponse),
            "GetRouteResponse" => Some(Self::GetRouteResponse),
            "DeleteRouteResponse" => Some(Self::DeleteRouteResponse),
            "GetRouteResponses" => Some(Self::GetRouteResponses),
            "CreateAuthorizer" => Some(Self::CreateAuthorizer),
            "GetAuthorizer" => Some(Self::GetAuthorizer),
            "UpdateAuthorizer" => Some(Self::UpdateAuthorizer),
            "DeleteAuthorizer" => Some(Self::DeleteAuthorizer),
            "GetAuthorizers" => Some(Self::GetAuthorizers),
            "CreateModel" => Some(Self::CreateModel),
            "GetModel" => Some(Self::GetModel),
            "UpdateModel" => Some(Self::UpdateModel),
            "DeleteModel" => Some(Self::DeleteModel),
            "GetModels" => Some(Self::GetModels),
            "GetModelTemplate" => Some(Self::GetModelTemplate),
            "CreateDomainName" => Some(Self::CreateDomainName),
            "GetDomainName" => Some(Self::GetDomainName),
            "UpdateDomainName" => Some(Self::UpdateDomainName),
            "DeleteDomainName" => Some(Self::DeleteDomainName),
            "GetDomainNames" => Some(Self::GetDomainNames),
            "CreateVpcLink" => Some(Self::CreateVpcLink),
            "GetVpcLink" => Some(Self::GetVpcLink),
            "UpdateVpcLink" => Some(Self::UpdateVpcLink),
            "DeleteVpcLink" => Some(Self::DeleteVpcLink),
            "GetVpcLinks" => Some(Self::GetVpcLinks),
            "TagResource" => Some(Self::TagResource),
            "UntagResource" => Some(Self::UntagResource),
            "GetTags" => Some(Self::GetTags),
            "CreateApiMapping" => Some(Self::CreateApiMapping),
            "GetApiMapping" => Some(Self::GetApiMapping),
            "UpdateApiMapping" => Some(Self::UpdateApiMapping),
            "DeleteApiMapping" => Some(Self::DeleteApiMapping),
            "GetApiMappings" => Some(Self::GetApiMappings),
            _ => None,
        }
    }
}

impl std::fmt::Display for ApiGatewayV2Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Route descriptor for an API Gateway v2 operation.
#[derive(Debug, Clone)]
pub struct ApiGatewayV2Route {
    /// HTTP method for this route.
    pub method: http::Method,
    /// URL path pattern with `{param}` placeholders.
    pub path_pattern: &'static str,
    /// Operation to dispatch to.
    pub operation: ApiGatewayV2Operation,
    /// HTTP status code on success.
    pub success_status: u16,
}

/// Route table for all API Gateway v2 operations.
///
/// Routes are ordered by specificity: longer/more-specific paths first.
/// When multiple operations share the same path, they are disambiguated by
/// HTTP method.
pub const APIGATEWAYV2_ROUTES: &[ApiGatewayV2Route] = &[
    // --- Route responses: /v2/apis/{apiId}/routes/{routeId}/routeresponses/{routeResponseId} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/routes/{routeId}/routeresponses/{routeResponseId}",
        operation: ApiGatewayV2Operation::GetRouteResponse,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/apis/{apiId}/routes/{routeId}/routeresponses/{routeResponseId}",
        operation: ApiGatewayV2Operation::DeleteRouteResponse,
        success_status: 204,
    },
    // --- Route responses collection: /v2/apis/{apiId}/routes/{routeId}/routeresponses ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis/{apiId}/routes/{routeId}/routeresponses",
        operation: ApiGatewayV2Operation::CreateRouteResponse,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/routes/{routeId}/routeresponses",
        operation: ApiGatewayV2Operation::GetRouteResponses,
        success_status: 200,
    },
    // --- Model template: /v2/apis/{apiId}/models/{modelId}/template ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/models/{modelId}/template",
        operation: ApiGatewayV2Operation::GetModelTemplate,
        success_status: 200,
    },
    // --- Individual routes: /v2/apis/{apiId}/routes/{routeId} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/routes/{routeId}",
        operation: ApiGatewayV2Operation::GetRoute,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/apis/{apiId}/routes/{routeId}",
        operation: ApiGatewayV2Operation::UpdateRoute,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/apis/{apiId}/routes/{routeId}",
        operation: ApiGatewayV2Operation::DeleteRoute,
        success_status: 204,
    },
    // --- Routes collection: /v2/apis/{apiId}/routes ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis/{apiId}/routes",
        operation: ApiGatewayV2Operation::CreateRoute,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/routes",
        operation: ApiGatewayV2Operation::GetRoutes,
        success_status: 200,
    },
    // --- Individual integrations: /v2/apis/{apiId}/integrations/{integrationId} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/integrations/{integrationId}",
        operation: ApiGatewayV2Operation::GetIntegration,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/apis/{apiId}/integrations/{integrationId}",
        operation: ApiGatewayV2Operation::UpdateIntegration,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/apis/{apiId}/integrations/{integrationId}",
        operation: ApiGatewayV2Operation::DeleteIntegration,
        success_status: 204,
    },
    // --- Integrations collection: /v2/apis/{apiId}/integrations ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis/{apiId}/integrations",
        operation: ApiGatewayV2Operation::CreateIntegration,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/integrations",
        operation: ApiGatewayV2Operation::GetIntegrations,
        success_status: 200,
    },
    // --- Individual stages: /v2/apis/{apiId}/stages/{stageName} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/stages/{stageName}",
        operation: ApiGatewayV2Operation::GetStage,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/apis/{apiId}/stages/{stageName}",
        operation: ApiGatewayV2Operation::UpdateStage,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/apis/{apiId}/stages/{stageName}",
        operation: ApiGatewayV2Operation::DeleteStage,
        success_status: 204,
    },
    // --- Stages collection: /v2/apis/{apiId}/stages ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis/{apiId}/stages",
        operation: ApiGatewayV2Operation::CreateStage,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/stages",
        operation: ApiGatewayV2Operation::GetStages,
        success_status: 200,
    },
    // --- Individual deployments: /v2/apis/{apiId}/deployments/{deploymentId} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/deployments/{deploymentId}",
        operation: ApiGatewayV2Operation::GetDeployment,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/apis/{apiId}/deployments/{deploymentId}",
        operation: ApiGatewayV2Operation::DeleteDeployment,
        success_status: 204,
    },
    // --- Deployments collection: /v2/apis/{apiId}/deployments ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis/{apiId}/deployments",
        operation: ApiGatewayV2Operation::CreateDeployment,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/deployments",
        operation: ApiGatewayV2Operation::GetDeployments,
        success_status: 200,
    },
    // --- Individual authorizers: /v2/apis/{apiId}/authorizers/{authorizerId} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/authorizers/{authorizerId}",
        operation: ApiGatewayV2Operation::GetAuthorizer,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/apis/{apiId}/authorizers/{authorizerId}",
        operation: ApiGatewayV2Operation::UpdateAuthorizer,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/apis/{apiId}/authorizers/{authorizerId}",
        operation: ApiGatewayV2Operation::DeleteAuthorizer,
        success_status: 204,
    },
    // --- Authorizers collection: /v2/apis/{apiId}/authorizers ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis/{apiId}/authorizers",
        operation: ApiGatewayV2Operation::CreateAuthorizer,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/authorizers",
        operation: ApiGatewayV2Operation::GetAuthorizers,
        success_status: 200,
    },
    // --- Individual models: /v2/apis/{apiId}/models/{modelId} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/models/{modelId}",
        operation: ApiGatewayV2Operation::GetModel,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/apis/{apiId}/models/{modelId}",
        operation: ApiGatewayV2Operation::UpdateModel,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/apis/{apiId}/models/{modelId}",
        operation: ApiGatewayV2Operation::DeleteModel,
        success_status: 204,
    },
    // --- Models collection: /v2/apis/{apiId}/models ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis/{apiId}/models",
        operation: ApiGatewayV2Operation::CreateModel,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}/models",
        operation: ApiGatewayV2Operation::GetModels,
        success_status: 200,
    },
    // --- Individual API: /v2/apis/{apiId} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}",
        operation: ApiGatewayV2Operation::GetApi,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/apis/{apiId}",
        operation: ApiGatewayV2Operation::UpdateApi,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/apis/{apiId}",
        operation: ApiGatewayV2Operation::DeleteApi,
        success_status: 204,
    },
    // --- APIs collection: /v2/apis ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis",
        operation: ApiGatewayV2Operation::CreateApi,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis",
        operation: ApiGatewayV2Operation::GetApis,
        success_status: 200,
    },
    // --- Individual domain names: /v2/domainnames/{domainName} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/domainnames/{domainName}",
        operation: ApiGatewayV2Operation::GetDomainName,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/domainnames/{domainName}",
        operation: ApiGatewayV2Operation::UpdateDomainName,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/domainnames/{domainName}",
        operation: ApiGatewayV2Operation::DeleteDomainName,
        success_status: 204,
    },
    // --- Domain names collection: /v2/domainnames ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/domainnames",
        operation: ApiGatewayV2Operation::CreateDomainName,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/domainnames",
        operation: ApiGatewayV2Operation::GetDomainNames,
        success_status: 200,
    },
    // --- Individual API mappings: /v2/domainnames/{domainName}/apimappings/{apiMappingId} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/domainnames/{domainName}/apimappings/{apiMappingId}",
        operation: ApiGatewayV2Operation::GetApiMapping,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/domainnames/{domainName}/apimappings/{apiMappingId}",
        operation: ApiGatewayV2Operation::UpdateApiMapping,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/domainnames/{domainName}/apimappings/{apiMappingId}",
        operation: ApiGatewayV2Operation::DeleteApiMapping,
        success_status: 204,
    },
    // --- API mappings collection: /v2/domainnames/{domainName}/apimappings ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/domainnames/{domainName}/apimappings",
        operation: ApiGatewayV2Operation::CreateApiMapping,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/domainnames/{domainName}/apimappings",
        operation: ApiGatewayV2Operation::GetApiMappings,
        success_status: 200,
    },
    // --- Individual VPC links: /v2/vpclinks/{vpcLinkId} ---
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/vpclinks/{vpcLinkId}",
        operation: ApiGatewayV2Operation::GetVpcLink,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/vpclinks/{vpcLinkId}",
        operation: ApiGatewayV2Operation::UpdateVpcLink,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/vpclinks/{vpcLinkId}",
        operation: ApiGatewayV2Operation::DeleteVpcLink,
        success_status: 202,
    },
    // --- VPC links collection: /v2/vpclinks ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/vpclinks",
        operation: ApiGatewayV2Operation::CreateVpcLink,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/vpclinks",
        operation: ApiGatewayV2Operation::GetVpcLinks,
        success_status: 200,
    },
    // --- Tags: /v2/tags/{resource-arn} ---
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/tags/{resource-arn}",
        operation: ApiGatewayV2Operation::TagResource,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/tags/{resource-arn}",
        operation: ApiGatewayV2Operation::GetTags,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::DELETE,
        path_pattern: "/v2/tags/{resource-arn}",
        operation: ApiGatewayV2Operation::UntagResource,
        success_status: 204,
    },
];
