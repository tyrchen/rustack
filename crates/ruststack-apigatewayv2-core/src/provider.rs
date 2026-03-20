//! API Gateway v2 business logic provider.
//!
//! Implements all 56 API Gateway v2 operations, maintaining internal storage
//! and converting between model input/output types and internal records.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tracing::info;

#[allow(clippy::wildcard_imports)]
use ruststack_apigatewayv2_model::input::*;
#[allow(clippy::wildcard_imports)]
use ruststack_apigatewayv2_model::output::*;
use ruststack_apigatewayv2_model::types::{
    Api, ApiMapping, Authorizer, ConnectionType as ModelConnectionType, Deployment,
    DeploymentStatus, DomainName, Integration, Model, MutualTlsAuthentication, Route,
    RouteResponse, Stage, TlsConfig, VpcLink, VpcLinkStatus, VpcLinkVersion,
};

use crate::config::ApiGatewayV2Config;
use crate::error::ApiGatewayV2ServiceError;
use crate::storage::{
    ApiMappingRecord, ApiRecord, ApiStore, AuthorizerRecord, DeploymentRecord, DomainNameRecord,
    IntegrationRecord, ModelRecord, RouteRecord, RouteResponseRecord, StageRecord, VpcLinkRecord,
    generate_id,
};

/// Main API Gateway v2 provider. Owns resource storage and configuration.
#[derive(Debug)]
pub struct RustStackApiGatewayV2 {
    /// Resource storage.
    store: ApiStore,
    /// Configuration.
    config: Arc<ApiGatewayV2Config>,
    /// HTTP client for HTTP proxy integrations.
    http_client: reqwest::Client,
}

#[allow(
    clippy::needless_pass_by_value,
    clippy::manual_let_else,
    clippy::assigning_clones
)]
impl RustStackApiGatewayV2 {
    /// Create a new provider with the given configuration.
    #[must_use]
    pub fn new(config: ApiGatewayV2Config) -> Self {
        Self {
            store: ApiStore::new(),
            config: Arc::new(config),
            http_client: reqwest::Client::new(),
        }
    }

    /// Returns a reference to the API store.
    #[must_use]
    pub fn store(&self) -> &ApiStore {
        &self.store
    }

    /// Returns a reference to the configuration.
    #[must_use]
    pub fn config(&self) -> &ApiGatewayV2Config {
        &self.config
    }

    /// Returns a reference to the HTTP client.
    #[must_use]
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    fn api_endpoint(&self, api_id: &str) -> String {
        format!(
            "https://{api_id}.execute-api.{}.amazonaws.com",
            self.config.default_region
        )
    }

    fn api_arn(&self, api_id: &str) -> String {
        format!(
            "arn:aws:apigateway:{}::/apis/{api_id}",
            self.config.default_region
        )
    }

    fn domain_arn(&self, domain_name: &str) -> String {
        format!(
            "arn:aws:apigateway:{}::/domainnames/{domain_name}",
            self.config.default_region
        )
    }

    fn vpc_link_arn(&self, vpc_link_id: &str) -> String {
        format!(
            "arn:aws:apigateway:{}::/vpclinks/{vpc_link_id}",
            self.config.default_region
        )
    }

    /// Perform auto-deploy for stages with auto_deploy enabled.
    fn auto_deploy(&self, api_id: &str) {
        let mut api_ref = match self.store.apis.get_mut(api_id) {
            Some(r) => r,
            None => return,
        };
        let api = api_ref.value_mut();
        let auto_deploy_stages: Vec<String> = api
            .stages
            .iter()
            .filter(|(_, s)| s.auto_deploy)
            .map(|(name, _)| name.clone())
            .collect();

        for stage_name in auto_deploy_stages {
            let deployment_id = generate_id();
            let now = Utc::now();
            let deployment = DeploymentRecord {
                deployment_id: deployment_id.clone(),
                description: Some("Auto-deployed".to_owned()),
                auto_deployed: true,
                deployment_status: DeploymentStatus::Deployed,
                deployment_status_message: None,
                created_date: now,
            };
            api.deployments.insert(deployment_id.clone(), deployment);
            if let Some(stage) = api.stages.get_mut(&stage_name) {
                stage.deployment_id = Some(deployment_id);
                stage.last_updated_date = now;
            }
        }
    }

    // ---------------------------------------------------------------
    // API operations
    // ---------------------------------------------------------------

    /// Create a new API.
    pub fn create_api(
        &self,
        input: CreateApiInput,
    ) -> Result<CreateApiResponse, ApiGatewayV2ServiceError> {
        if input.name.is_empty() {
            return Err(ApiGatewayV2ServiceError::BadRequest(
                "Name is required".to_owned(),
            ));
        }
        let api_id = generate_id();
        let now = Utc::now();
        let route_selection_expression = input
            .route_selection_expression
            .unwrap_or_else(|| "${request.method} ${request.path}".to_owned());
        let api_endpoint = self.api_endpoint(&api_id);

        let record = ApiRecord {
            api_id: api_id.clone(),
            name: input.name.clone(),
            protocol_type: input.protocol_type.clone(),
            route_selection_expression: route_selection_expression.clone(),
            api_key_selection_expression: input.api_key_selection_expression.clone(),
            cors_configuration: input.cors_configuration.clone(),
            description: input.description.clone(),
            disable_execute_api_endpoint: input.disable_execute_api_endpoint.unwrap_or(false),
            disable_schema_validation: input.disable_schema_validation.unwrap_or(false),
            ip_address_type: input.ip_address_type.clone(),
            version: input.version.clone(),
            api_endpoint: api_endpoint.clone(),
            tags: input.tags.clone(),
            created_date: now,
            routes: HashMap::new(),
            integrations: HashMap::new(),
            stages: HashMap::new(),
            deployments: HashMap::new(),
            authorizers: HashMap::new(),
            models: HashMap::new(),
        };

        // Store tags by ARN
        if !input.tags.is_empty() {
            let arn = self.api_arn(&api_id);
            self.store.tags.insert(arn, input.tags.clone());
        }

        self.store.apis.insert(api_id.clone(), record);
        info!(api_id = %api_id, name = %input.name, "created API");

        // If target is provided, create a default integration and route
        if let Some(target) = &input.target {
            self.create_quick_api_resources(&api_id, input.route_key.as_deref(), target);
        }

        Ok(CreateApiResponse {
            api_endpoint: Some(api_endpoint),
            api_gateway_managed: Some(false),
            api_id: Some(api_id),
            api_key_selection_expression: input.api_key_selection_expression,
            cors_configuration: input.cors_configuration,
            created_date: Some(now),
            description: input.description,
            disable_execute_api_endpoint: input.disable_execute_api_endpoint,
            disable_schema_validation: input.disable_schema_validation,
            import_info: Vec::new(),
            ip_address_type: input.ip_address_type,
            name: Some(input.name),
            protocol_type: Some(input.protocol_type),
            route_selection_expression: Some(route_selection_expression),
            tags: input.tags,
            version: input.version,
            warnings: Vec::new(),
        })
    }

    fn create_quick_api_resources(&self, api_id: &str, route_key: Option<&str>, target: &str) {
        let mut api_ref = match self.store.apis.get_mut(api_id) {
            Some(r) => r,
            None => return,
        };
        let api = api_ref.value_mut();

        // Create integration
        let integration_id = generate_id();
        let integration = IntegrationRecord {
            integration_id: integration_id.clone(),
            integration_type: ruststack_apigatewayv2_model::types::IntegrationType::AwsProxy,
            integration_method: Some("POST".to_owned()),
            integration_uri: Some(target.to_owned()),
            connection_type: Some(ModelConnectionType::Internet),
            connection_id: None,
            content_handling_strategy: None,
            credentials_arn: None,
            description: None,
            passthrough_behavior: None,
            payload_format_version: Some("2.0".to_owned()),
            request_parameters: HashMap::new(),
            request_templates: HashMap::new(),
            response_parameters: HashMap::new(),
            template_selection_expression: None,
            timeout_in_millis: Some(30_000),
            tls_config: None,
            integration_subtype: None,
            api_gateway_managed: true,
            integration_response_selection_expression: None,
        };
        api.integrations.insert(integration_id.clone(), integration);

        // Create route
        let route_id = generate_id();
        let rk = route_key.unwrap_or("$default").to_owned();
        let route = RouteRecord {
            route_id: route_id.clone(),
            route_key: rk,
            target: Some(format!("integrations/{integration_id}")),
            authorization_type: None,
            authorizer_id: None,
            authorization_scopes: Vec::new(),
            api_key_required: false,
            model_selection_expression: None,
            operation_name: None,
            request_models: HashMap::new(),
            request_parameters: HashMap::new(),
            route_response_selection_expression: None,
            route_responses: HashMap::new(),
            api_gateway_managed: true,
        };
        api.routes.insert(route_id, route);
    }

    /// Get an API by ID.
    pub fn get_api(&self, input: GetApiInput) -> Result<GetApiResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        Ok(api_record_to_get_response(&api))
    }

    /// Update an API.
    pub fn update_api(
        &self,
        input: UpdateApiInput,
    ) -> Result<UpdateApiResponse, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();

        if let Some(name) = &input.name {
            api.name.clone_from(name);
        }
        if let Some(desc) = &input.description {
            api.description = Some(desc.clone());
        }
        if let Some(cors) = input.cors_configuration {
            api.cors_configuration = Some(cors);
        }
        if let Some(v) = input.disable_execute_api_endpoint {
            api.disable_execute_api_endpoint = v;
        }
        if let Some(v) = input.disable_schema_validation {
            api.disable_schema_validation = v;
        }
        if let Some(v) = &input.route_selection_expression {
            api.route_selection_expression.clone_from(v);
        }
        if let Some(v) = &input.api_key_selection_expression {
            api.api_key_selection_expression = Some(v.clone());
        }
        if let Some(v) = &input.version {
            api.version = Some(v.clone());
        }
        if let Some(v) = input.ip_address_type {
            api.ip_address_type = Some(v);
        }

        Ok(UpdateApiResponse {
            api_endpoint: Some(api.api_endpoint.clone()),
            api_gateway_managed: Some(false),
            api_id: Some(api.api_id.clone()),
            api_key_selection_expression: api.api_key_selection_expression.clone(),
            cors_configuration: api.cors_configuration.clone(),
            created_date: Some(api.created_date),
            description: api.description.clone(),
            disable_execute_api_endpoint: Some(api.disable_execute_api_endpoint),
            disable_schema_validation: Some(api.disable_schema_validation),
            import_info: Vec::new(),
            ip_address_type: api.ip_address_type.clone(),
            name: Some(api.name.clone()),
            protocol_type: Some(api.protocol_type.clone()),
            route_selection_expression: Some(api.route_selection_expression.clone()),
            tags: api.tags.clone(),
            version: api.version.clone(),
            warnings: Vec::new(),
        })
    }

    /// Delete an API.
    pub fn delete_api(&self, input: DeleteApiInput) -> Result<(), ApiGatewayV2ServiceError> {
        self.store.apis.remove(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let arn = self.api_arn(&input.api_id);
        self.store.tags.remove(&arn);
        info!(api_id = %input.api_id, "deleted API");
        Ok(())
    }

    /// List all APIs.
    pub fn get_apis(
        &self,
        _input: GetApisInput,
    ) -> Result<GetApisResponse, ApiGatewayV2ServiceError> {
        let items: Vec<Api> = self
            .store
            .apis
            .iter()
            .map(|entry| api_record_to_api(entry.value()))
            .collect();
        Ok(GetApisResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Route operations
    // ---------------------------------------------------------------

    /// Create a route.
    pub fn create_route(
        &self,
        input: CreateRouteInput,
    ) -> Result<CreateRouteResult, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let route_id = generate_id();

        let route = RouteRecord {
            route_id: route_id.clone(),
            route_key: input.route_key.clone(),
            target: input.target.clone(),
            authorization_type: input.authorization_type.clone(),
            authorizer_id: input.authorizer_id.clone(),
            authorization_scopes: input.authorization_scopes.clone(),
            api_key_required: input.api_key_required.unwrap_or(false),
            model_selection_expression: input.model_selection_expression.clone(),
            operation_name: input.operation_name.clone(),
            request_models: input.request_models.clone(),
            request_parameters: input.request_parameters.clone(),
            route_response_selection_expression: input.route_response_selection_expression.clone(),
            route_responses: HashMap::new(),
            api_gateway_managed: false,
        };
        api.routes.insert(route_id.clone(), route);

        // Trigger auto-deploy
        drop(api_ref);
        self.auto_deploy(&input.api_id);

        Ok(CreateRouteResult {
            api_gateway_managed: Some(false),
            api_key_required: input.api_key_required,
            authorization_scopes: input.authorization_scopes,
            authorization_type: input.authorization_type,
            authorizer_id: input.authorizer_id,
            model_selection_expression: input.model_selection_expression,
            operation_name: input.operation_name,
            request_models: input.request_models,
            request_parameters: input.request_parameters,
            route_id: Some(route_id),
            route_key: Some(input.route_key),
            route_response_selection_expression: input.route_response_selection_expression,
            target: input.target,
        })
    }

    /// Get a route.
    pub fn get_route(
        &self,
        input: GetRouteInput,
    ) -> Result<GetRouteResult, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let route = api.routes.get(&input.route_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Route with id '{}'",
                input.route_id
            ))
        })?;
        Ok(route_record_to_get_result(route))
    }

    /// Update a route.
    pub fn update_route(
        &self,
        input: UpdateRouteInput,
    ) -> Result<UpdateRouteResult, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let route = api.routes.get_mut(&input.route_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Route with id '{}'",
                input.route_id
            ))
        })?;

        if let Some(key) = &input.route_key {
            route.route_key.clone_from(key);
        }
        if let Some(target) = &input.target {
            route.target = Some(target.clone());
        }
        if let Some(v) = &input.authorization_type {
            route.authorization_type = Some(v.clone());
        }
        if let Some(v) = &input.authorizer_id {
            route.authorizer_id = Some(v.clone());
        }
        if !input.authorization_scopes.is_empty() {
            route.authorization_scopes = input.authorization_scopes.clone();
        }
        if let Some(v) = input.api_key_required {
            route.api_key_required = v;
        }
        if let Some(v) = &input.model_selection_expression {
            route.model_selection_expression = Some(v.clone());
        }
        if let Some(v) = &input.operation_name {
            route.operation_name = Some(v.clone());
        }
        if !input.request_models.is_empty() {
            route.request_models = input.request_models.clone();
        }
        if !input.request_parameters.is_empty() {
            route.request_parameters = input.request_parameters.clone();
        }
        if let Some(v) = &input.route_response_selection_expression {
            route.route_response_selection_expression = Some(v.clone());
        }

        let result = route_record_to_update_result(route);
        drop(api_ref);
        self.auto_deploy(&input.api_id);
        Ok(result)
    }

    /// Delete a route.
    pub fn delete_route(&self, input: DeleteRouteInput) -> Result<(), ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        api_ref
            .value_mut()
            .routes
            .remove(&input.route_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find Route with id '{}'",
                    input.route_id
                ))
            })?;
        drop(api_ref);
        self.auto_deploy(&input.api_id);
        Ok(())
    }

    /// List routes for an API.
    pub fn get_routes(
        &self,
        input: GetRoutesInput,
    ) -> Result<GetRoutesResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let items: Vec<Route> = api.routes.values().map(route_record_to_route).collect();
        Ok(GetRoutesResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Integration operations
    // ---------------------------------------------------------------

    /// Create an integration.
    pub fn create_integration(
        &self,
        input: CreateIntegrationInput,
    ) -> Result<CreateIntegrationResult, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let integration_id = generate_id();

        let pfv = input
            .payload_format_version
            .clone()
            .unwrap_or_else(|| "1.0".to_owned());
        let timeout = input.timeout_in_millis.unwrap_or(30_000);
        let tls = input.tls_config.as_ref().map(|t| TlsConfig {
            server_name_to_verify: t.server_name_to_verify.clone(),
        });

        let record = IntegrationRecord {
            integration_id: integration_id.clone(),
            integration_type: input.integration_type.clone(),
            integration_method: input.integration_method.clone(),
            integration_uri: input.integration_uri.clone(),
            connection_type: input.connection_type.clone(),
            connection_id: input.connection_id.clone(),
            content_handling_strategy: input.content_handling_strategy.clone(),
            credentials_arn: input.credentials_arn.clone(),
            description: input.description.clone(),
            passthrough_behavior: input.passthrough_behavior.clone(),
            payload_format_version: Some(pfv.clone()),
            request_parameters: input.request_parameters.clone(),
            request_templates: input.request_templates.clone(),
            response_parameters: input.response_parameters.clone(),
            template_selection_expression: input.template_selection_expression.clone(),
            timeout_in_millis: Some(timeout),
            tls_config: tls.clone(),
            integration_subtype: input.integration_subtype.clone(),
            api_gateway_managed: false,
            integration_response_selection_expression: None,
        };

        api.integrations.insert(integration_id.clone(), record);
        drop(api_ref);
        self.auto_deploy(&input.api_id);

        Ok(CreateIntegrationResult {
            api_gateway_managed: Some(false),
            connection_id: input.connection_id,
            connection_type: input.connection_type,
            content_handling_strategy: input.content_handling_strategy,
            credentials_arn: input.credentials_arn,
            description: input.description,
            integration_id: Some(integration_id),
            integration_method: input.integration_method,
            integration_response_selection_expression: None,
            integration_subtype: input.integration_subtype,
            integration_type: Some(input.integration_type),
            integration_uri: input.integration_uri,
            passthrough_behavior: input.passthrough_behavior,
            payload_format_version: Some(pfv),
            request_parameters: input.request_parameters,
            request_templates: input.request_templates,
            response_parameters: input.response_parameters,
            template_selection_expression: input.template_selection_expression,
            timeout_in_millis: Some(timeout),
            tls_config: tls,
        })
    }

    /// Get an integration.
    pub fn get_integration(
        &self,
        input: GetIntegrationInput,
    ) -> Result<GetIntegrationResult, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let integ = api.integrations.get(&input.integration_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Integration with id '{}'",
                input.integration_id
            ))
        })?;
        Ok(integration_record_to_get_result(integ))
    }

    /// Update an integration.
    #[allow(clippy::too_many_lines)]
    pub fn update_integration(
        &self,
        input: UpdateIntegrationInput,
    ) -> Result<UpdateIntegrationResult, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let integ = api
            .integrations
            .get_mut(&input.integration_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find Integration with id '{}'",
                    input.integration_id
                ))
            })?;

        if let Some(v) = &input.integration_type {
            integ.integration_type = v.clone();
        }
        if let Some(v) = &input.integration_method {
            integ.integration_method = Some(v.clone());
        }
        if let Some(v) = &input.integration_uri {
            integ.integration_uri = Some(v.clone());
        }
        if let Some(v) = &input.connection_type {
            integ.connection_type = Some(v.clone());
        }
        if let Some(v) = &input.connection_id {
            integ.connection_id = Some(v.clone());
        }
        if let Some(v) = &input.content_handling_strategy {
            integ.content_handling_strategy = Some(v.clone());
        }
        if let Some(v) = &input.credentials_arn {
            integ.credentials_arn = Some(v.clone());
        }
        if let Some(v) = &input.description {
            integ.description = Some(v.clone());
        }
        if let Some(v) = &input.passthrough_behavior {
            integ.passthrough_behavior = Some(v.clone());
        }
        if let Some(v) = &input.payload_format_version {
            integ.payload_format_version = Some(v.clone());
        }
        if !input.request_parameters.is_empty() {
            integ.request_parameters = input.request_parameters.clone();
        }
        if !input.request_templates.is_empty() {
            integ.request_templates = input.request_templates.clone();
        }
        if !input.response_parameters.is_empty() {
            integ.response_parameters = input.response_parameters.clone();
        }
        if let Some(v) = &input.template_selection_expression {
            integ.template_selection_expression = Some(v.clone());
        }
        if let Some(v) = input.timeout_in_millis {
            integ.timeout_in_millis = Some(v);
        }
        if let Some(v) = &input.tls_config {
            integ.tls_config = Some(TlsConfig {
                server_name_to_verify: v.server_name_to_verify.clone(),
            });
        }
        if let Some(v) = &input.integration_subtype {
            integ.integration_subtype = Some(v.clone());
        }

        let result = integration_record_to_update_result(integ);
        drop(api_ref);
        self.auto_deploy(&input.api_id);
        Ok(result)
    }

    /// Delete an integration.
    pub fn delete_integration(
        &self,
        input: DeleteIntegrationInput,
    ) -> Result<(), ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        api.integrations
            .remove(&input.integration_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find Integration with id '{}'",
                    input.integration_id
                ))
            })?;

        // Clear route targets pointing to this integration
        let target_ref = format!("integrations/{}", input.integration_id);
        for route in api.routes.values_mut() {
            if route.target.as_deref() == Some(&target_ref) {
                route.target = None;
            }
        }

        drop(api_ref);
        self.auto_deploy(&input.api_id);
        Ok(())
    }

    /// List integrations for an API.
    pub fn get_integrations(
        &self,
        input: GetIntegrationsInput,
    ) -> Result<GetIntegrationsResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let items: Vec<Integration> = api
            .integrations
            .values()
            .map(integration_record_to_integration)
            .collect();
        Ok(GetIntegrationsResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Stage operations
    // ---------------------------------------------------------------

    /// Create a stage.
    pub fn create_stage(
        &self,
        input: CreateStageInput,
    ) -> Result<CreateStageResponse, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();

        if api.stages.contains_key(&input.stage_name) {
            return Err(ApiGatewayV2ServiceError::Conflict(format!(
                "Stage '{}' already exists",
                input.stage_name
            )));
        }

        let now = Utc::now();
        let record = StageRecord {
            stage_name: input.stage_name.clone(),
            deployment_id: input.deployment_id.clone(),
            description: input.description.clone(),
            auto_deploy: input.auto_deploy.unwrap_or(false),
            stage_variables: input.stage_variables.clone(),
            default_route_settings: input.default_route_settings.clone(),
            route_settings: input.route_settings.clone(),
            access_log_settings: input.access_log_settings.clone(),
            client_certificate_id: input.client_certificate_id.clone(),
            tags: input.tags.clone(),
            created_date: now,
            last_updated_date: now,
            api_gateway_managed: false,
        };
        api.stages.insert(input.stage_name.clone(), record);

        Ok(CreateStageResponse {
            access_log_settings: input.access_log_settings,
            api_gateway_managed: Some(false),
            auto_deploy: input.auto_deploy,
            client_certificate_id: input.client_certificate_id,
            created_date: Some(now),
            default_route_settings: input.default_route_settings,
            deployment_id: input.deployment_id,
            description: input.description,
            last_deployment_status_message: None,
            last_updated_date: Some(now),
            route_settings: input.route_settings,
            stage_name: Some(input.stage_name),
            stage_variables: input.stage_variables,
            tags: input.tags,
        })
    }

    /// Get a stage.
    pub fn get_stage(
        &self,
        input: GetStageInput,
    ) -> Result<GetStageResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let stage = api.stages.get(&input.stage_name).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Stage with name '{}'",
                input.stage_name
            ))
        })?;
        Ok(stage_record_to_get_response(stage))
    }

    /// Update a stage.
    pub fn update_stage(
        &self,
        input: UpdateStageInput,
    ) -> Result<UpdateStageResponse, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let stage = api.stages.get_mut(&input.stage_name).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Stage with name '{}'",
                input.stage_name
            ))
        })?;

        if let Some(v) = &input.description {
            stage.description = Some(v.clone());
        }
        if let Some(v) = input.auto_deploy {
            stage.auto_deploy = v;
        }
        if let Some(v) = &input.deployment_id {
            stage.deployment_id = Some(v.clone());
        }
        if !input.stage_variables.is_empty() {
            stage.stage_variables = input.stage_variables.clone();
        }
        if let Some(v) = input.default_route_settings {
            stage.default_route_settings = Some(v);
        }
        if !input.route_settings.is_empty() {
            stage.route_settings = input.route_settings.clone();
        }
        if let Some(v) = input.access_log_settings {
            stage.access_log_settings = Some(v);
        }
        if let Some(v) = &input.client_certificate_id {
            stage.client_certificate_id = Some(v.clone());
        }
        stage.last_updated_date = Utc::now();

        let result = stage_record_to_update_response(stage);
        Ok(result)
    }

    /// Delete a stage.
    pub fn delete_stage(&self, input: DeleteStageInput) -> Result<(), ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        api_ref
            .value_mut()
            .stages
            .remove(&input.stage_name)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find Stage with name '{}'",
                    input.stage_name
                ))
            })?;
        Ok(())
    }

    /// List stages for an API.
    pub fn get_stages(
        &self,
        input: GetStagesInput,
    ) -> Result<GetStagesResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let items: Vec<Stage> = api.stages.values().map(stage_record_to_stage).collect();
        Ok(GetStagesResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Deployment operations
    // ---------------------------------------------------------------

    /// Create a deployment.
    pub fn create_deployment(
        &self,
        input: CreateDeploymentInput,
    ) -> Result<CreateDeploymentResponse, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let deployment_id = generate_id();
        let now = Utc::now();

        let record = DeploymentRecord {
            deployment_id: deployment_id.clone(),
            description: input.description.clone(),
            auto_deployed: false,
            deployment_status: DeploymentStatus::Deployed,
            deployment_status_message: None,
            created_date: now,
        };
        api.deployments.insert(deployment_id.clone(), record);

        // If stage_name is provided, update the stage's deployment_id
        if let Some(stage_name) = &input.stage_name {
            if let Some(stage) = api.stages.get_mut(stage_name) {
                stage.deployment_id = Some(deployment_id.clone());
                stage.last_updated_date = now;
            }
        }

        Ok(CreateDeploymentResponse {
            auto_deployed: Some(false),
            created_date: Some(now),
            deployment_id: Some(deployment_id),
            deployment_status: Some(DeploymentStatus::Deployed),
            deployment_status_message: None,
            description: input.description,
        })
    }

    /// Get a deployment.
    pub fn get_deployment(
        &self,
        input: GetDeploymentInput,
    ) -> Result<GetDeploymentResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let dep = api.deployments.get(&input.deployment_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Deployment with id '{}'",
                input.deployment_id
            ))
        })?;
        Ok(deployment_record_to_get_response(dep))
    }

    /// Delete a deployment.
    pub fn delete_deployment(
        &self,
        input: DeleteDeploymentInput,
    ) -> Result<(), ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        api.deployments
            .remove(&input.deployment_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find Deployment with id '{}'",
                    input.deployment_id
                ))
            })?;
        // Clear stage references
        for stage in api.stages.values_mut() {
            if stage.deployment_id.as_deref() == Some(&input.deployment_id) {
                stage.deployment_id = None;
            }
        }
        Ok(())
    }

    /// List deployments for an API.
    pub fn get_deployments(
        &self,
        input: GetDeploymentsInput,
    ) -> Result<GetDeploymentsResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let items: Vec<Deployment> = api
            .deployments
            .values()
            .map(deployment_record_to_deployment)
            .collect();
        Ok(GetDeploymentsResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Route response operations
    // ---------------------------------------------------------------

    /// Create a route response.
    pub fn create_route_response(
        &self,
        input: CreateRouteResponseInput,
    ) -> Result<CreateRouteResponseResponse, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let route = api.routes.get_mut(&input.route_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Route with id '{}'",
                input.route_id
            ))
        })?;

        let rr_id = generate_id();
        let record = RouteResponseRecord {
            route_response_id: rr_id.clone(),
            route_response_key: input.route_response_key.clone(),
            model_selection_expression: input.model_selection_expression.clone(),
            response_models: input.response_models.clone(),
            response_parameters: input.response_parameters.clone(),
        };
        route.route_responses.insert(rr_id.clone(), record);

        Ok(CreateRouteResponseResponse {
            model_selection_expression: input.model_selection_expression,
            response_models: input.response_models,
            response_parameters: input.response_parameters,
            route_response_id: Some(rr_id),
            route_response_key: Some(input.route_response_key),
        })
    }

    /// Get a route response.
    pub fn get_route_response(
        &self,
        input: GetRouteResponseInput,
    ) -> Result<GetRouteResponseResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let route = api.routes.get(&input.route_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Route with id '{}'",
                input.route_id
            ))
        })?;
        let rr = route
            .route_responses
            .get(&input.route_response_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find RouteResponse with id '{}'",
                    input.route_response_id
                ))
            })?;
        Ok(GetRouteResponseResponse {
            model_selection_expression: rr.model_selection_expression.clone(),
            response_models: rr.response_models.clone(),
            response_parameters: rr.response_parameters.clone(),
            route_response_id: Some(rr.route_response_id.clone()),
            route_response_key: Some(rr.route_response_key.clone()),
        })
    }

    /// Delete a route response.
    pub fn delete_route_response(
        &self,
        input: DeleteRouteResponseInput,
    ) -> Result<(), ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let route = api.routes.get_mut(&input.route_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Route with id '{}'",
                input.route_id
            ))
        })?;
        route
            .route_responses
            .remove(&input.route_response_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find RouteResponse with id '{}'",
                    input.route_response_id
                ))
            })?;
        Ok(())
    }

    /// List route responses for a route.
    pub fn get_route_responses(
        &self,
        input: GetRouteResponsesInput,
    ) -> Result<GetRouteResponsesResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let route = api.routes.get(&input.route_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Route with id '{}'",
                input.route_id
            ))
        })?;
        let items: Vec<RouteResponse> = route
            .route_responses
            .values()
            .map(|rr| RouteResponse {
                model_selection_expression: rr.model_selection_expression.clone(),
                response_models: rr.response_models.clone(),
                response_parameters: rr.response_parameters.clone(),
                route_response_id: Some(rr.route_response_id.clone()),
                route_response_key: rr.route_response_key.clone(),
            })
            .collect();
        Ok(GetRouteResponsesResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Authorizer operations
    // ---------------------------------------------------------------

    /// Create an authorizer.
    pub fn create_authorizer(
        &self,
        input: CreateAuthorizerInput,
    ) -> Result<CreateAuthorizerResponse, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let authorizer_id = generate_id();

        let record = AuthorizerRecord {
            authorizer_id: authorizer_id.clone(),
            name: input.name.clone(),
            authorizer_type: input.authorizer_type.clone(),
            jwt_configuration: input.jwt_configuration.clone(),
            authorizer_credentials_arn: input.authorizer_credentials_arn.clone(),
            authorizer_uri: input.authorizer_uri.clone(),
            identity_source: input.identity_source.clone(),
            identity_validation_expression: input.identity_validation_expression.clone(),
            authorizer_payload_format_version: input.authorizer_payload_format_version.clone(),
            authorizer_result_ttl_in_seconds: input.authorizer_result_ttl_in_seconds,
            enable_simple_responses: input.enable_simple_responses,
        };
        api.authorizers.insert(authorizer_id.clone(), record);

        Ok(CreateAuthorizerResponse {
            authorizer_credentials_arn: input.authorizer_credentials_arn,
            authorizer_id: Some(authorizer_id),
            authorizer_payload_format_version: input.authorizer_payload_format_version,
            authorizer_result_ttl_in_seconds: input.authorizer_result_ttl_in_seconds,
            authorizer_type: Some(input.authorizer_type),
            authorizer_uri: input.authorizer_uri,
            enable_simple_responses: input.enable_simple_responses,
            identity_source: input.identity_source,
            identity_validation_expression: input.identity_validation_expression,
            jwt_configuration: input.jwt_configuration,
            name: Some(input.name),
        })
    }

    /// Get an authorizer.
    pub fn get_authorizer(
        &self,
        input: GetAuthorizerInput,
    ) -> Result<GetAuthorizerResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let auth = api.authorizers.get(&input.authorizer_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Authorizer with id '{}'",
                input.authorizer_id
            ))
        })?;
        Ok(authorizer_record_to_get_response(auth))
    }

    /// Update an authorizer.
    pub fn update_authorizer(
        &self,
        input: UpdateAuthorizerInput,
    ) -> Result<UpdateAuthorizerResponse, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let auth = api
            .authorizers
            .get_mut(&input.authorizer_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find Authorizer with id '{}'",
                    input.authorizer_id
                ))
            })?;

        if let Some(v) = &input.name {
            auth.name.clone_from(v);
        }
        if let Some(v) = &input.authorizer_type {
            auth.authorizer_type = v.clone();
        }
        if let Some(v) = &input.jwt_configuration {
            auth.jwt_configuration = Some(v.clone());
        }
        if let Some(v) = &input.authorizer_credentials_arn {
            auth.authorizer_credentials_arn = Some(v.clone());
        }
        if let Some(v) = &input.authorizer_uri {
            auth.authorizer_uri = Some(v.clone());
        }
        if !input.identity_source.is_empty() {
            auth.identity_source = input.identity_source.clone();
        }
        if let Some(v) = &input.identity_validation_expression {
            auth.identity_validation_expression = Some(v.clone());
        }
        if let Some(v) = &input.authorizer_payload_format_version {
            auth.authorizer_payload_format_version = Some(v.clone());
        }
        if let Some(v) = input.authorizer_result_ttl_in_seconds {
            auth.authorizer_result_ttl_in_seconds = Some(v);
        }
        if let Some(v) = input.enable_simple_responses {
            auth.enable_simple_responses = Some(v);
        }

        Ok(authorizer_record_to_update_response(auth))
    }

    /// Delete an authorizer.
    pub fn delete_authorizer(
        &self,
        input: DeleteAuthorizerInput,
    ) -> Result<(), ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        api.authorizers
            .remove(&input.authorizer_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find Authorizer with id '{}'",
                    input.authorizer_id
                ))
            })?;
        // Clear route references
        for route in api.routes.values_mut() {
            if route.authorizer_id.as_deref() == Some(&input.authorizer_id) {
                route.authorizer_id = None;
            }
        }
        Ok(())
    }

    /// List authorizers.
    pub fn get_authorizers(
        &self,
        input: GetAuthorizersInput,
    ) -> Result<GetAuthorizersResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let items: Vec<Authorizer> = api
            .authorizers
            .values()
            .map(authorizer_record_to_authorizer)
            .collect();
        Ok(GetAuthorizersResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Model operations
    // ---------------------------------------------------------------

    /// Create a model.
    pub fn create_model(
        &self,
        input: CreateModelInput,
    ) -> Result<CreateModelResponse, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let model_id = generate_id();

        let record = ModelRecord {
            model_id: model_id.clone(),
            name: input.name.clone(),
            content_type: input.content_type.clone(),
            schema: input.schema.clone(),
            description: input.description.clone(),
        };
        api.models.insert(model_id.clone(), record);

        Ok(CreateModelResponse {
            content_type: input.content_type,
            description: input.description,
            model_id: Some(model_id),
            name: Some(input.name),
            schema: Some(input.schema),
        })
    }

    /// Get a model.
    pub fn get_model(
        &self,
        input: GetModelInput,
    ) -> Result<GetModelResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let model = api.models.get(&input.model_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Model with id '{}'",
                input.model_id
            ))
        })?;
        Ok(GetModelResponse {
            content_type: model.content_type.clone(),
            description: model.description.clone(),
            model_id: Some(model.model_id.clone()),
            name: Some(model.name.clone()),
            schema: Some(model.schema.clone()),
        })
    }

    /// Update a model.
    pub fn update_model(
        &self,
        input: UpdateModelInput,
    ) -> Result<UpdateModelResponse, ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let api = api_ref.value_mut();
        let model = api.models.get_mut(&input.model_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Model with id '{}'",
                input.model_id
            ))
        })?;

        if let Some(v) = &input.name {
            model.name.clone_from(v);
        }
        if let Some(v) = &input.content_type {
            model.content_type = Some(v.clone());
        }
        if let Some(v) = &input.schema {
            model.schema.clone_from(v);
        }
        if let Some(v) = &input.description {
            model.description = Some(v.clone());
        }

        Ok(UpdateModelResponse {
            content_type: model.content_type.clone(),
            description: model.description.clone(),
            model_id: Some(model.model_id.clone()),
            name: Some(model.name.clone()),
            schema: Some(model.schema.clone()),
        })
    }

    /// Delete a model.
    pub fn delete_model(&self, input: DeleteModelInput) -> Result<(), ApiGatewayV2ServiceError> {
        let mut api_ref = self.store.apis.get_mut(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        api_ref
            .value_mut()
            .models
            .remove(&input.model_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find Model with id '{}'",
                    input.model_id
                ))
            })?;
        Ok(())
    }

    /// List models.
    pub fn get_models(
        &self,
        input: GetModelsInput,
    ) -> Result<GetModelsResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let items: Vec<Model> = api
            .models
            .values()
            .map(|m| Model {
                content_type: m.content_type.clone(),
                description: m.description.clone(),
                model_id: Some(m.model_id.clone()),
                name: m.name.clone(),
                schema: Some(m.schema.clone()),
            })
            .collect();
        Ok(GetModelsResponse {
            items,
            next_token: None,
        })
    }

    /// Get a model template.
    pub fn get_model_template(
        &self,
        input: GetModelTemplateInput,
    ) -> Result<GetModelTemplateResponse, ApiGatewayV2ServiceError> {
        let api = self.store.apis.get(&input.api_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Api with id '{}'",
                input.api_id
            ))
        })?;
        let model = api.models.get(&input.model_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find Model with id '{}'",
                input.model_id
            ))
        })?;
        Ok(GetModelTemplateResponse {
            value: Some(model.schema.clone()),
        })
    }

    // ---------------------------------------------------------------
    // Domain name operations
    // ---------------------------------------------------------------

    /// Create a domain name.
    pub fn create_domain_name(
        &self,
        input: CreateDomainNameInput,
    ) -> Result<CreateDomainNameResponse, ApiGatewayV2ServiceError> {
        if self.store.domain_names.contains_key(&input.domain_name) {
            return Err(ApiGatewayV2ServiceError::Conflict(format!(
                "Domain name '{}' already exists",
                input.domain_name
            )));
        }

        let mtls = input
            .mutual_tls_authentication
            .as_ref()
            .map(|m| MutualTlsAuthentication {
                truststore_uri: m.truststore_uri.clone(),
                truststore_version: m.truststore_version.clone(),
                truststore_warnings: Vec::new(),
            });

        let record = DomainNameRecord {
            domain_name: input.domain_name.clone(),
            domain_name_configurations: input.domain_name_configurations.clone(),
            mutual_tls_authentication: mtls.clone(),
            routing_mode: input.routing_mode.clone(),
            tags: input.tags.clone(),
            api_mappings: HashMap::new(),
        };

        if !input.tags.is_empty() {
            let arn = self.domain_arn(&input.domain_name);
            self.store.tags.insert(arn, input.tags.clone());
        }

        self.store
            .domain_names
            .insert(input.domain_name.clone(), record);

        Ok(CreateDomainNameResponse {
            api_mapping_selection_expression: Some("$request.basepath".to_owned()),
            domain_name: Some(input.domain_name.clone()),
            domain_name_arn: Some(self.domain_arn(&input.domain_name)),
            domain_name_configurations: input.domain_name_configurations,
            mutual_tls_authentication: mtls,
            routing_mode: input.routing_mode,
            tags: input.tags,
        })
    }

    /// Get a domain name.
    pub fn get_domain_name(
        &self,
        input: GetDomainNameInput,
    ) -> Result<GetDomainNameResponse, ApiGatewayV2ServiceError> {
        let dn = self
            .store
            .domain_names
            .get(&input.domain_name)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find DomainName '{}'",
                    input.domain_name
                ))
            })?;
        Ok(GetDomainNameResponse {
            api_mapping_selection_expression: Some("$request.basepath".to_owned()),
            domain_name: Some(dn.domain_name.clone()),
            domain_name_arn: Some(self.domain_arn(&dn.domain_name)),
            domain_name_configurations: dn.domain_name_configurations.clone(),
            mutual_tls_authentication: dn.mutual_tls_authentication.clone(),
            routing_mode: dn.routing_mode.clone(),
            tags: dn.tags.clone(),
        })
    }

    /// Update a domain name.
    pub fn update_domain_name(
        &self,
        input: UpdateDomainNameInput,
    ) -> Result<UpdateDomainNameResponse, ApiGatewayV2ServiceError> {
        let mut dn_ref = self
            .store
            .domain_names
            .get_mut(&input.domain_name)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find DomainName '{}'",
                    input.domain_name
                ))
            })?;
        let dn = dn_ref.value_mut();
        if !input.domain_name_configurations.is_empty() {
            dn.domain_name_configurations = input.domain_name_configurations;
        }
        if let Some(m) = input.mutual_tls_authentication {
            dn.mutual_tls_authentication = Some(MutualTlsAuthentication {
                truststore_uri: m.truststore_uri,
                truststore_version: m.truststore_version,
                truststore_warnings: Vec::new(),
            });
        }
        if let Some(r) = input.routing_mode {
            dn.routing_mode = Some(r);
        }

        Ok(UpdateDomainNameResponse {
            api_mapping_selection_expression: Some("$request.basepath".to_owned()),
            domain_name: Some(dn.domain_name.clone()),
            domain_name_arn: Some(self.domain_arn(&dn.domain_name)),
            domain_name_configurations: dn.domain_name_configurations.clone(),
            mutual_tls_authentication: dn.mutual_tls_authentication.clone(),
            routing_mode: dn.routing_mode.clone(),
            tags: dn.tags.clone(),
        })
    }

    /// Delete a domain name.
    pub fn delete_domain_name(
        &self,
        input: DeleteDomainNameInput,
    ) -> Result<(), ApiGatewayV2ServiceError> {
        self.store
            .domain_names
            .remove(&input.domain_name)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find DomainName '{}'",
                    input.domain_name
                ))
            })?;
        let arn = self.domain_arn(&input.domain_name);
        self.store.tags.remove(&arn);
        Ok(())
    }

    /// List domain names.
    pub fn get_domain_names(
        &self,
        _input: GetDomainNamesInput,
    ) -> Result<GetDomainNamesResponse, ApiGatewayV2ServiceError> {
        let items: Vec<DomainName> = self
            .store
            .domain_names
            .iter()
            .map(|entry| {
                let dn = entry.value();
                DomainName {
                    api_mapping_selection_expression: Some("$request.basepath".to_owned()),
                    domain_name: dn.domain_name.clone(),
                    domain_name_arn: Some(self.domain_arn(&dn.domain_name)),
                    domain_name_configurations: dn.domain_name_configurations.clone(),
                    mutual_tls_authentication: dn.mutual_tls_authentication.clone(),
                    routing_mode: dn.routing_mode.clone(),
                    tags: dn.tags.clone(),
                }
            })
            .collect();
        Ok(GetDomainNamesResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // VPC link operations
    // ---------------------------------------------------------------

    /// Create a VPC link.
    pub fn create_vpc_link(
        &self,
        input: CreateVpcLinkInput,
    ) -> Result<CreateVpcLinkResponse, ApiGatewayV2ServiceError> {
        let vpc_link_id = generate_id();
        let now = Utc::now();

        let record = VpcLinkRecord {
            vpc_link_id: vpc_link_id.clone(),
            name: input.name.clone(),
            security_group_ids: input.security_group_ids.clone(),
            subnet_ids: input.subnet_ids.clone(),
            tags: input.tags.clone(),
            created_date: now,
        };

        if !input.tags.is_empty() {
            let arn = self.vpc_link_arn(&vpc_link_id);
            self.store.tags.insert(arn, input.tags.clone());
        }

        self.store.vpc_links.insert(vpc_link_id.clone(), record);

        Ok(CreateVpcLinkResponse {
            created_date: Some(now),
            name: Some(input.name),
            security_group_ids: input.security_group_ids,
            subnet_ids: input.subnet_ids,
            tags: input.tags,
            vpc_link_id: Some(vpc_link_id),
            vpc_link_status: Some(VpcLinkStatus::Available),
            vpc_link_status_message: Some("VPC link is ready".to_owned()),
            vpc_link_version: Some(VpcLinkVersion::V2),
        })
    }

    /// Get a VPC link.
    pub fn get_vpc_link(
        &self,
        input: GetVpcLinkInput,
    ) -> Result<GetVpcLinkResponse, ApiGatewayV2ServiceError> {
        let vl = self
            .store
            .vpc_links
            .get(&input.vpc_link_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find VpcLink with id '{}'",
                    input.vpc_link_id
                ))
            })?;
        Ok(vpc_link_record_to_get_response(&vl))
    }

    /// Update a VPC link.
    pub fn update_vpc_link(
        &self,
        input: UpdateVpcLinkInput,
    ) -> Result<UpdateVpcLinkResponse, ApiGatewayV2ServiceError> {
        let mut vl_ref = self
            .store
            .vpc_links
            .get_mut(&input.vpc_link_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find VpcLink with id '{}'",
                    input.vpc_link_id
                ))
            })?;
        let vl = vl_ref.value_mut();
        if let Some(v) = &input.name {
            vl.name.clone_from(v);
        }
        Ok(UpdateVpcLinkResponse {
            created_date: Some(vl.created_date),
            name: Some(vl.name.clone()),
            security_group_ids: vl.security_group_ids.clone(),
            subnet_ids: vl.subnet_ids.clone(),
            tags: vl.tags.clone(),
            vpc_link_id: Some(vl.vpc_link_id.clone()),
            vpc_link_status: Some(VpcLinkStatus::Available),
            vpc_link_status_message: Some("VPC link is ready".to_owned()),
            vpc_link_version: Some(VpcLinkVersion::V2),
        })
    }

    /// Delete a VPC link.
    pub fn delete_vpc_link(
        &self,
        input: DeleteVpcLinkInput,
    ) -> Result<DeleteVpcLinkResponse, ApiGatewayV2ServiceError> {
        self.store
            .vpc_links
            .remove(&input.vpc_link_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find VpcLink with id '{}'",
                    input.vpc_link_id
                ))
            })?;
        let arn = self.vpc_link_arn(&input.vpc_link_id);
        self.store.tags.remove(&arn);
        Ok(DeleteVpcLinkResponse {})
    }

    /// List VPC links.
    pub fn get_vpc_links(
        &self,
        _input: GetVpcLinksInput,
    ) -> Result<GetVpcLinksResponse, ApiGatewayV2ServiceError> {
        let items: Vec<VpcLink> = self
            .store
            .vpc_links
            .iter()
            .map(|entry| vpc_link_record_to_vpc_link(entry.value()))
            .collect();
        Ok(GetVpcLinksResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Tag operations
    // ---------------------------------------------------------------

    /// Tag a resource.
    pub fn tag_resource(
        &self,
        input: TagResourceInput,
    ) -> Result<TagResourceResponse, ApiGatewayV2ServiceError> {
        self.store
            .tags
            .entry(input.resource_arn)
            .and_modify(|existing| {
                for (k, v) in &input.tags {
                    existing.insert(k.clone(), v.clone());
                }
            })
            .or_insert(input.tags);
        Ok(TagResourceResponse {})
    }

    /// Untag a resource.
    pub fn untag_resource(
        &self,
        input: UntagResourceInput,
    ) -> Result<(), ApiGatewayV2ServiceError> {
        if let Some(mut tags) = self.store.tags.get_mut(&input.resource_arn) {
            for key in &input.tag_keys {
                tags.value_mut().remove(key);
            }
        }
        Ok(())
    }

    /// Get tags for a resource.
    pub fn get_tags(
        &self,
        input: GetTagsInput,
    ) -> Result<GetTagsResponse, ApiGatewayV2ServiceError> {
        let tags = self
            .store
            .tags
            .get(&input.resource_arn)
            .map(|entry| entry.value().clone())
            .unwrap_or_default();
        Ok(GetTagsResponse { tags })
    }

    // ---------------------------------------------------------------
    // API Mapping operations
    // ---------------------------------------------------------------

    /// Create an API mapping.
    pub fn create_api_mapping(
        &self,
        input: CreateApiMappingInput,
    ) -> Result<CreateApiMappingResponse, ApiGatewayV2ServiceError> {
        let mut dn_ref = self
            .store
            .domain_names
            .get_mut(&input.domain_name)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find DomainName '{}'",
                    input.domain_name
                ))
            })?;
        let mapping_id = generate_id();
        let record = ApiMappingRecord {
            api_mapping_id: mapping_id.clone(),
            api_id: input.api_id.clone(),
            api_mapping_key: input.api_mapping_key.clone(),
            stage: input.stage.clone(),
        };
        dn_ref
            .value_mut()
            .api_mappings
            .insert(mapping_id.clone(), record);

        Ok(CreateApiMappingResponse {
            api_id: Some(input.api_id),
            api_mapping_id: Some(mapping_id),
            api_mapping_key: input.api_mapping_key,
            stage: Some(input.stage),
        })
    }

    /// Get an API mapping.
    pub fn get_api_mapping(
        &self,
        input: GetApiMappingInput,
    ) -> Result<GetApiMappingResponse, ApiGatewayV2ServiceError> {
        let dn = self
            .store
            .domain_names
            .get(&input.domain_name)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find DomainName '{}'",
                    input.domain_name
                ))
            })?;
        let mapping = dn.api_mappings.get(&input.api_mapping_id).ok_or_else(|| {
            ApiGatewayV2ServiceError::NotFound(format!(
                "Unable to find ApiMapping with id '{}'",
                input.api_mapping_id
            ))
        })?;
        Ok(GetApiMappingResponse {
            api_id: Some(mapping.api_id.clone()),
            api_mapping_id: Some(mapping.api_mapping_id.clone()),
            api_mapping_key: mapping.api_mapping_key.clone(),
            stage: Some(mapping.stage.clone()),
        })
    }

    /// Update an API mapping.
    pub fn update_api_mapping(
        &self,
        input: UpdateApiMappingInput,
    ) -> Result<UpdateApiMappingResponse, ApiGatewayV2ServiceError> {
        let mut dn_ref = self
            .store
            .domain_names
            .get_mut(&input.domain_name)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find DomainName '{}'",
                    input.domain_name
                ))
            })?;
        let mapping = dn_ref
            .value_mut()
            .api_mappings
            .get_mut(&input.api_mapping_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find ApiMapping with id '{}'",
                    input.api_mapping_id
                ))
            })?;
        mapping.api_id.clone_from(&input.api_id);
        if let Some(v) = &input.api_mapping_key {
            mapping.api_mapping_key = Some(v.clone());
        }
        if let Some(v) = &input.stage {
            mapping.stage.clone_from(v);
        }

        Ok(UpdateApiMappingResponse {
            api_id: Some(mapping.api_id.clone()),
            api_mapping_id: Some(mapping.api_mapping_id.clone()),
            api_mapping_key: mapping.api_mapping_key.clone(),
            stage: Some(mapping.stage.clone()),
        })
    }

    /// Delete an API mapping.
    pub fn delete_api_mapping(
        &self,
        input: DeleteApiMappingInput,
    ) -> Result<(), ApiGatewayV2ServiceError> {
        let mut dn_ref = self
            .store
            .domain_names
            .get_mut(&input.domain_name)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find DomainName '{}'",
                    input.domain_name
                ))
            })?;
        dn_ref
            .value_mut()
            .api_mappings
            .remove(&input.api_mapping_id)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find ApiMapping with id '{}'",
                    input.api_mapping_id
                ))
            })?;
        Ok(())
    }

    /// List API mappings.
    pub fn get_api_mappings(
        &self,
        input: GetApiMappingsInput,
    ) -> Result<GetApiMappingsResponse, ApiGatewayV2ServiceError> {
        let dn = self
            .store
            .domain_names
            .get(&input.domain_name)
            .ok_or_else(|| {
                ApiGatewayV2ServiceError::NotFound(format!(
                    "Unable to find DomainName '{}'",
                    input.domain_name
                ))
            })?;
        let items: Vec<ApiMapping> = dn
            .api_mappings
            .values()
            .map(|m| ApiMapping {
                api_id: m.api_id.clone(),
                api_mapping_id: Some(m.api_mapping_id.clone()),
                api_mapping_key: m.api_mapping_key.clone(),
                stage: m.stage.clone(),
            })
            .collect();
        Ok(GetApiMappingsResponse {
            items,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Execution engine
    // ---------------------------------------------------------------

    /// Handle an API execution request (for the invoke path).
    pub async fn handle_execution(
        &self,
        api_id: &str,
        stage_name: &str,
        method: &http::Method,
        path: &str,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<http::Response<bytes::Bytes>, ApiGatewayV2ServiceError> {
        crate::execution::handle_execution(self, api_id, stage_name, method, path, headers, body)
            .await
    }
}

// ---------------------------------------------------------------
// Conversion helpers: internal records -> model output types
// ---------------------------------------------------------------

fn api_record_to_get_response(api: &ApiRecord) -> GetApiResponse {
    GetApiResponse {
        api_endpoint: Some(api.api_endpoint.clone()),
        api_gateway_managed: Some(false),
        api_id: Some(api.api_id.clone()),
        api_key_selection_expression: api.api_key_selection_expression.clone(),
        cors_configuration: api.cors_configuration.clone(),
        created_date: Some(api.created_date),
        description: api.description.clone(),
        disable_execute_api_endpoint: Some(api.disable_execute_api_endpoint),
        disable_schema_validation: Some(api.disable_schema_validation),
        import_info: Vec::new(),
        ip_address_type: api.ip_address_type.clone(),
        name: Some(api.name.clone()),
        protocol_type: Some(api.protocol_type.clone()),
        route_selection_expression: Some(api.route_selection_expression.clone()),
        tags: api.tags.clone(),
        version: api.version.clone(),
        warnings: Vec::new(),
    }
}

fn api_record_to_api(api: &ApiRecord) -> Api {
    Api {
        api_endpoint: Some(api.api_endpoint.clone()),
        api_gateway_managed: Some(false),
        api_id: Some(api.api_id.clone()),
        api_key_selection_expression: api.api_key_selection_expression.clone(),
        cors_configuration: api.cors_configuration.clone(),
        created_date: Some(api.created_date),
        description: api.description.clone(),
        disable_execute_api_endpoint: Some(api.disable_execute_api_endpoint),
        disable_schema_validation: Some(api.disable_schema_validation),
        import_info: Vec::new(),
        ip_address_type: api.ip_address_type.clone(),
        name: api.name.clone(),
        protocol_type: api.protocol_type.clone(),
        route_selection_expression: api.route_selection_expression.clone(),
        tags: api.tags.clone(),
        version: api.version.clone(),
        warnings: Vec::new(),
    }
}

fn route_record_to_get_result(r: &RouteRecord) -> GetRouteResult {
    GetRouteResult {
        api_gateway_managed: Some(r.api_gateway_managed),
        api_key_required: Some(r.api_key_required),
        authorization_scopes: r.authorization_scopes.clone(),
        authorization_type: r.authorization_type.clone(),
        authorizer_id: r.authorizer_id.clone(),
        model_selection_expression: r.model_selection_expression.clone(),
        operation_name: r.operation_name.clone(),
        request_models: r.request_models.clone(),
        request_parameters: r.request_parameters.clone(),
        route_id: Some(r.route_id.clone()),
        route_key: Some(r.route_key.clone()),
        route_response_selection_expression: r.route_response_selection_expression.clone(),
        target: r.target.clone(),
    }
}

fn route_record_to_update_result(r: &RouteRecord) -> UpdateRouteResult {
    UpdateRouteResult {
        api_gateway_managed: Some(r.api_gateway_managed),
        api_key_required: Some(r.api_key_required),
        authorization_scopes: r.authorization_scopes.clone(),
        authorization_type: r.authorization_type.clone(),
        authorizer_id: r.authorizer_id.clone(),
        model_selection_expression: r.model_selection_expression.clone(),
        operation_name: r.operation_name.clone(),
        request_models: r.request_models.clone(),
        request_parameters: r.request_parameters.clone(),
        route_id: Some(r.route_id.clone()),
        route_key: Some(r.route_key.clone()),
        route_response_selection_expression: r.route_response_selection_expression.clone(),
        target: r.target.clone(),
    }
}

fn route_record_to_route(r: &RouteRecord) -> Route {
    Route {
        api_gateway_managed: Some(r.api_gateway_managed),
        api_key_required: Some(r.api_key_required),
        authorization_scopes: r.authorization_scopes.clone(),
        authorization_type: r.authorization_type.clone(),
        authorizer_id: r.authorizer_id.clone(),
        model_selection_expression: r.model_selection_expression.clone(),
        operation_name: r.operation_name.clone(),
        request_models: r.request_models.clone(),
        request_parameters: r.request_parameters.clone(),
        route_id: Some(r.route_id.clone()),
        route_key: r.route_key.clone(),
        route_response_selection_expression: r.route_response_selection_expression.clone(),
        target: r.target.clone(),
    }
}

fn integration_record_to_get_result(i: &IntegrationRecord) -> GetIntegrationResult {
    GetIntegrationResult {
        api_gateway_managed: Some(i.api_gateway_managed),
        connection_id: i.connection_id.clone(),
        connection_type: i.connection_type.clone(),
        content_handling_strategy: i.content_handling_strategy.clone(),
        credentials_arn: i.credentials_arn.clone(),
        description: i.description.clone(),
        integration_id: Some(i.integration_id.clone()),
        integration_method: i.integration_method.clone(),
        integration_response_selection_expression: i
            .integration_response_selection_expression
            .clone(),
        integration_subtype: i.integration_subtype.clone(),
        integration_type: Some(i.integration_type.clone()),
        integration_uri: i.integration_uri.clone(),
        passthrough_behavior: i.passthrough_behavior.clone(),
        payload_format_version: i.payload_format_version.clone(),
        request_parameters: i.request_parameters.clone(),
        request_templates: i.request_templates.clone(),
        response_parameters: i.response_parameters.clone(),
        template_selection_expression: i.template_selection_expression.clone(),
        timeout_in_millis: i.timeout_in_millis,
        tls_config: i.tls_config.clone(),
    }
}

fn integration_record_to_update_result(i: &IntegrationRecord) -> UpdateIntegrationResult {
    UpdateIntegrationResult {
        api_gateway_managed: Some(i.api_gateway_managed),
        connection_id: i.connection_id.clone(),
        connection_type: i.connection_type.clone(),
        content_handling_strategy: i.content_handling_strategy.clone(),
        credentials_arn: i.credentials_arn.clone(),
        description: i.description.clone(),
        integration_id: Some(i.integration_id.clone()),
        integration_method: i.integration_method.clone(),
        integration_response_selection_expression: i
            .integration_response_selection_expression
            .clone(),
        integration_subtype: i.integration_subtype.clone(),
        integration_type: Some(i.integration_type.clone()),
        integration_uri: i.integration_uri.clone(),
        passthrough_behavior: i.passthrough_behavior.clone(),
        payload_format_version: i.payload_format_version.clone(),
        request_parameters: i.request_parameters.clone(),
        request_templates: i.request_templates.clone(),
        response_parameters: i.response_parameters.clone(),
        template_selection_expression: i.template_selection_expression.clone(),
        timeout_in_millis: i.timeout_in_millis,
        tls_config: i.tls_config.clone(),
    }
}

fn integration_record_to_integration(i: &IntegrationRecord) -> Integration {
    Integration {
        api_gateway_managed: Some(i.api_gateway_managed),
        connection_id: i.connection_id.clone(),
        connection_type: i.connection_type.clone(),
        content_handling_strategy: i.content_handling_strategy.clone(),
        credentials_arn: i.credentials_arn.clone(),
        description: i.description.clone(),
        integration_id: Some(i.integration_id.clone()),
        integration_method: i.integration_method.clone(),
        integration_response_selection_expression: i
            .integration_response_selection_expression
            .clone(),
        integration_subtype: i.integration_subtype.clone(),
        integration_type: Some(i.integration_type.clone()),
        integration_uri: i.integration_uri.clone(),
        passthrough_behavior: i.passthrough_behavior.clone(),
        payload_format_version: i.payload_format_version.clone(),
        request_parameters: i.request_parameters.clone(),
        request_templates: i.request_templates.clone(),
        response_parameters: i.response_parameters.clone(),
        template_selection_expression: i.template_selection_expression.clone(),
        timeout_in_millis: i.timeout_in_millis,
        tls_config: i.tls_config.clone(),
    }
}

fn stage_record_to_get_response(s: &StageRecord) -> GetStageResponse {
    GetStageResponse {
        access_log_settings: s.access_log_settings.clone(),
        api_gateway_managed: Some(s.api_gateway_managed),
        auto_deploy: Some(s.auto_deploy),
        client_certificate_id: s.client_certificate_id.clone(),
        created_date: Some(s.created_date),
        default_route_settings: s.default_route_settings.clone(),
        deployment_id: s.deployment_id.clone(),
        description: s.description.clone(),
        last_deployment_status_message: None,
        last_updated_date: Some(s.last_updated_date),
        route_settings: s.route_settings.clone(),
        stage_name: Some(s.stage_name.clone()),
        stage_variables: s.stage_variables.clone(),
        tags: s.tags.clone(),
    }
}

fn stage_record_to_update_response(s: &StageRecord) -> UpdateStageResponse {
    UpdateStageResponse {
        access_log_settings: s.access_log_settings.clone(),
        api_gateway_managed: Some(s.api_gateway_managed),
        auto_deploy: Some(s.auto_deploy),
        client_certificate_id: s.client_certificate_id.clone(),
        created_date: Some(s.created_date),
        default_route_settings: s.default_route_settings.clone(),
        deployment_id: s.deployment_id.clone(),
        description: s.description.clone(),
        last_deployment_status_message: None,
        last_updated_date: Some(s.last_updated_date),
        route_settings: s.route_settings.clone(),
        stage_name: Some(s.stage_name.clone()),
        stage_variables: s.stage_variables.clone(),
        tags: s.tags.clone(),
    }
}

fn stage_record_to_stage(s: &StageRecord) -> Stage {
    Stage {
        access_log_settings: s.access_log_settings.clone(),
        api_gateway_managed: Some(s.api_gateway_managed),
        auto_deploy: Some(s.auto_deploy),
        client_certificate_id: s.client_certificate_id.clone(),
        created_date: Some(s.created_date),
        default_route_settings: s.default_route_settings.clone(),
        deployment_id: s.deployment_id.clone(),
        description: s.description.clone(),
        last_deployment_status_message: None,
        last_updated_date: Some(s.last_updated_date),
        route_settings: s.route_settings.clone(),
        stage_name: s.stage_name.clone(),
        stage_variables: s.stage_variables.clone(),
        tags: s.tags.clone(),
    }
}

fn deployment_record_to_get_response(d: &DeploymentRecord) -> GetDeploymentResponse {
    GetDeploymentResponse {
        auto_deployed: Some(d.auto_deployed),
        created_date: Some(d.created_date),
        deployment_id: Some(d.deployment_id.clone()),
        deployment_status: Some(d.deployment_status.clone()),
        deployment_status_message: d.deployment_status_message.clone(),
        description: d.description.clone(),
    }
}

fn deployment_record_to_deployment(d: &DeploymentRecord) -> Deployment {
    Deployment {
        auto_deployed: Some(d.auto_deployed),
        created_date: Some(d.created_date),
        deployment_id: Some(d.deployment_id.clone()),
        deployment_status: Some(d.deployment_status.clone()),
        deployment_status_message: d.deployment_status_message.clone(),
        description: d.description.clone(),
    }
}

fn authorizer_record_to_get_response(a: &AuthorizerRecord) -> GetAuthorizerResponse {
    GetAuthorizerResponse {
        authorizer_credentials_arn: a.authorizer_credentials_arn.clone(),
        authorizer_id: Some(a.authorizer_id.clone()),
        authorizer_payload_format_version: a.authorizer_payload_format_version.clone(),
        authorizer_result_ttl_in_seconds: a.authorizer_result_ttl_in_seconds,
        authorizer_type: Some(a.authorizer_type.clone()),
        authorizer_uri: a.authorizer_uri.clone(),
        enable_simple_responses: a.enable_simple_responses,
        identity_source: a.identity_source.clone(),
        identity_validation_expression: a.identity_validation_expression.clone(),
        jwt_configuration: a.jwt_configuration.clone(),
        name: Some(a.name.clone()),
    }
}

fn authorizer_record_to_update_response(a: &AuthorizerRecord) -> UpdateAuthorizerResponse {
    UpdateAuthorizerResponse {
        authorizer_credentials_arn: a.authorizer_credentials_arn.clone(),
        authorizer_id: Some(a.authorizer_id.clone()),
        authorizer_payload_format_version: a.authorizer_payload_format_version.clone(),
        authorizer_result_ttl_in_seconds: a.authorizer_result_ttl_in_seconds,
        authorizer_type: Some(a.authorizer_type.clone()),
        authorizer_uri: a.authorizer_uri.clone(),
        enable_simple_responses: a.enable_simple_responses,
        identity_source: a.identity_source.clone(),
        identity_validation_expression: a.identity_validation_expression.clone(),
        jwt_configuration: a.jwt_configuration.clone(),
        name: Some(a.name.clone()),
    }
}

fn authorizer_record_to_authorizer(a: &AuthorizerRecord) -> Authorizer {
    Authorizer {
        authorizer_credentials_arn: a.authorizer_credentials_arn.clone(),
        authorizer_id: Some(a.authorizer_id.clone()),
        authorizer_payload_format_version: a.authorizer_payload_format_version.clone(),
        authorizer_result_ttl_in_seconds: a.authorizer_result_ttl_in_seconds,
        authorizer_type: Some(a.authorizer_type.clone()),
        authorizer_uri: a.authorizer_uri.clone(),
        enable_simple_responses: a.enable_simple_responses,
        identity_source: a.identity_source.clone(),
        identity_validation_expression: a.identity_validation_expression.clone(),
        jwt_configuration: a.jwt_configuration.clone(),
        name: a.name.clone(),
    }
}

fn vpc_link_record_to_get_response(vl: &VpcLinkRecord) -> GetVpcLinkResponse {
    GetVpcLinkResponse {
        created_date: Some(vl.created_date),
        name: Some(vl.name.clone()),
        security_group_ids: vl.security_group_ids.clone(),
        subnet_ids: vl.subnet_ids.clone(),
        tags: vl.tags.clone(),
        vpc_link_id: Some(vl.vpc_link_id.clone()),
        vpc_link_status: Some(VpcLinkStatus::Available),
        vpc_link_status_message: Some("VPC link is ready".to_owned()),
        vpc_link_version: Some(VpcLinkVersion::V2),
    }
}

fn vpc_link_record_to_vpc_link(vl: &VpcLinkRecord) -> VpcLink {
    VpcLink {
        created_date: Some(vl.created_date),
        name: vl.name.clone(),
        security_group_ids: vl.security_group_ids.clone(),
        subnet_ids: vl.subnet_ids.clone(),
        tags: vl.tags.clone(),
        vpc_link_id: vl.vpc_link_id.clone(),
        vpc_link_status: Some(VpcLinkStatus::Available),
        vpc_link_status_message: Some("VPC link is ready".to_owned()),
        vpc_link_version: Some(VpcLinkVersion::V2),
    }
}
