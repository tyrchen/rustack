//! API Gateway v2 integration tests against a running `RustStack` server.
//!
//! These tests cover the full lifecycle of API Gateway v2 resources:
//! APIs, Routes, Integrations, Stages, Deployments, Route Responses,
//! Authorizers, Models, Domain Names, VPC Links, Tags, API Mappings,
//! Execution with Mock integration, and CORS preflight handling.

#[cfg(test)]
mod tests {
    use aws_sdk_apigatewayv2::types::{
        AuthorizerType, ConnectionType, IntegrationType, ProtocolType,
    };

    use crate::apigatewayv2_client;

    /// Generate a unique identifier for test resources.
    fn unique_id() -> String {
        uuid::Uuid::new_v4().to_string()[..8].to_owned()
    }

    // ---------------------------------------------------------------------------
    // API CRUD
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_api() {
        let client = apigatewayv2_client();
        let name = format!("test-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        let get = client
            .get_api()
            .api_id(api_id)
            .send()
            .await
            .expect("get_api should succeed");

        assert_eq!(get.name(), Some(name.as_str()));

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_apis() {
        let client = apigatewayv2_client();
        let name = format!("test-list-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        let list = client
            .get_apis()
            .send()
            .await
            .expect("get_apis should succeed");

        let found = list.items().iter().any(|a| a.api_id() == Some(api_id));
        assert!(found, "listed APIs should contain the created API");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_api() {
        let client = apigatewayv2_client();
        let name = format!("test-update-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        let new_name = format!("updated-{name}");
        client
            .update_api()
            .api_id(api_id)
            .name(&new_name)
            .send()
            .await
            .expect("update_api should succeed");

        let get = client
            .get_api()
            .api_id(api_id)
            .send()
            .await
            .expect("get_api should succeed");

        assert_eq!(get.name(), Some(new_name.as_str()));

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // Route CRUD
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_route() {
        let client = apigatewayv2_client();
        let name = format!("test-route-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        let route = client
            .create_route()
            .api_id(api_id)
            .route_key("GET /items")
            .send()
            .await
            .expect("create_route should succeed");

        let route_id = route.route_id().expect("should have route_id");

        let get = client
            .get_route()
            .api_id(api_id)
            .route_id(route_id)
            .send()
            .await
            .expect("get_route should succeed");

        assert_eq!(get.route_key(), Some("GET /items"));

        let routes = client
            .get_routes()
            .api_id(api_id)
            .send()
            .await
            .expect("get_routes should succeed");

        assert!(
            routes
                .items()
                .iter()
                .any(|r| r.route_id() == Some(route_id)),
            "listed routes should contain the created route"
        );

        client
            .delete_route()
            .api_id(api_id)
            .route_id(route_id)
            .send()
            .await
            .expect("delete_route should succeed");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // Integration CRUD
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_integration() {
        let client = apigatewayv2_client();
        let name = format!("test-int-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        let integration = client
            .create_integration()
            .api_id(api_id)
            .integration_type(IntegrationType::Mock)
            .connection_type(ConnectionType::Internet)
            .send()
            .await
            .expect("create_integration should succeed");

        let integration_id = integration
            .integration_id()
            .expect("should have integration_id");

        let get = client
            .get_integration()
            .api_id(api_id)
            .integration_id(integration_id)
            .send()
            .await
            .expect("get_integration should succeed");

        assert_eq!(get.integration_type(), Some(&IntegrationType::Mock));

        let integrations = client
            .get_integrations()
            .api_id(api_id)
            .send()
            .await
            .expect("get_integrations should succeed");

        assert!(
            integrations
                .items()
                .iter()
                .any(|i| i.integration_id() == Some(integration_id)),
            "listed integrations should contain the created integration"
        );

        client
            .delete_integration()
            .api_id(api_id)
            .integration_id(integration_id)
            .send()
            .await
            .expect("delete_integration should succeed");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // Stage & Deployment
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_stage_and_deployment() {
        let client = apigatewayv2_client();
        let name = format!("test-stage-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        // Create a stage.
        let stage = client
            .create_stage()
            .api_id(api_id)
            .stage_name("dev")
            .send()
            .await
            .expect("create_stage should succeed");

        assert_eq!(stage.stage_name(), Some("dev"));

        // Get the stage.
        let get = client
            .get_stage()
            .api_id(api_id)
            .stage_name("dev")
            .send()
            .await
            .expect("get_stage should succeed");

        assert_eq!(get.stage_name(), Some("dev"));

        // List stages.
        let stages = client
            .get_stages()
            .api_id(api_id)
            .send()
            .await
            .expect("get_stages should succeed");

        assert!(
            stages.items().iter().any(|s| s.stage_name() == Some("dev")),
            "listed stages should contain 'dev'"
        );

        // Create a deployment.
        let deployment = client
            .create_deployment()
            .api_id(api_id)
            .send()
            .await
            .expect("create_deployment should succeed");

        let deployment_id = deployment
            .deployment_id()
            .expect("should have deployment_id");

        // Get deployment.
        let get_dep = client
            .get_deployment()
            .api_id(api_id)
            .deployment_id(deployment_id)
            .send()
            .await
            .expect("get_deployment should succeed");

        assert_eq!(get_dep.deployment_id(), Some(deployment_id));

        // List deployments.
        let deployments = client
            .get_deployments()
            .api_id(api_id)
            .send()
            .await
            .expect("get_deployments should succeed");

        assert!(
            deployments
                .items()
                .iter()
                .any(|d| d.deployment_id() == Some(deployment_id)),
            "listed deployments should contain the created deployment"
        );

        // Delete deployment, stage, and API.
        client
            .delete_deployment()
            .api_id(api_id)
            .deployment_id(deployment_id)
            .send()
            .await
            .expect("delete_deployment should succeed");

        client
            .delete_stage()
            .api_id(api_id)
            .stage_name("dev")
            .send()
            .await
            .expect("delete_stage should succeed");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // Route Response
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_route_response() {
        let client = apigatewayv2_client();
        let name = format!("test-rr-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Websocket)
            .route_selection_expression("$request.body.action")
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        let route = client
            .create_route()
            .api_id(api_id)
            .route_key("$default")
            .send()
            .await
            .expect("create_route should succeed");

        let route_id = route.route_id().expect("should have route_id");

        let rr = client
            .create_route_response()
            .api_id(api_id)
            .route_id(route_id)
            .route_response_key("$default")
            .send()
            .await
            .expect("create_route_response should succeed");

        let rr_id = rr
            .route_response_id()
            .expect("should have route_response_id");

        let get = client
            .get_route_response()
            .api_id(api_id)
            .route_id(route_id)
            .route_response_id(rr_id)
            .send()
            .await
            .expect("get_route_response should succeed");

        assert_eq!(get.route_response_key(), Some("$default"));

        client
            .delete_route_response()
            .api_id(api_id)
            .route_id(route_id)
            .route_response_id(rr_id)
            .send()
            .await
            .expect("delete_route_response should succeed");

        client
            .delete_route()
            .api_id(api_id)
            .route_id(route_id)
            .send()
            .await
            .expect("delete_route should succeed");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // Authorizer
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_authorizer() {
        let client = apigatewayv2_client();
        let name = format!("test-auth-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        let auth = client
            .create_authorizer()
            .api_id(api_id)
            .authorizer_type(AuthorizerType::Jwt)
            .identity_source("$request.header.Authorization")
            .name("test-auth")
            .send()
            .await
            .expect("create_authorizer should succeed");

        let auth_id = auth.authorizer_id().expect("should have authorizer_id");

        let get = client
            .get_authorizer()
            .api_id(api_id)
            .authorizer_id(auth_id)
            .send()
            .await
            .expect("get_authorizer should succeed");

        assert_eq!(get.name(), Some("test-auth"));

        let auths = client
            .get_authorizers()
            .api_id(api_id)
            .send()
            .await
            .expect("get_authorizers should succeed");

        assert!(
            auths
                .items()
                .iter()
                .any(|a| a.authorizer_id() == Some(auth_id)),
            "listed authorizers should contain the created authorizer"
        );

        client
            .delete_authorizer()
            .api_id(api_id)
            .authorizer_id(auth_id)
            .send()
            .await
            .expect("delete_authorizer should succeed");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // Model
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_model() {
        let client = apigatewayv2_client();
        let name = format!("test-model-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        let model = client
            .create_model()
            .api_id(api_id)
            .name("TestModel")
            .content_type("application/json")
            .schema(r#"{"type":"object"}"#)
            .send()
            .await
            .expect("create_model should succeed");

        let model_id = model.model_id().expect("should have model_id");

        let get = client
            .get_model()
            .api_id(api_id)
            .model_id(model_id)
            .send()
            .await
            .expect("get_model should succeed");

        assert_eq!(get.name(), Some("TestModel"));

        let models = client
            .get_models()
            .api_id(api_id)
            .send()
            .await
            .expect("get_models should succeed");

        assert!(
            models
                .items()
                .iter()
                .any(|m| m.model_id() == Some(model_id)),
            "listed models should contain the created model"
        );

        client
            .delete_model()
            .api_id(api_id)
            .model_id(model_id)
            .send()
            .await
            .expect("delete_model should succeed");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // Domain Name
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_domain_name() {
        let client = apigatewayv2_client();
        let domain = format!("{}.example.com", unique_id());

        let create = client
            .create_domain_name()
            .domain_name(&domain)
            .send()
            .await
            .expect("create_domain_name should succeed");

        assert_eq!(create.domain_name(), Some(domain.as_str()));

        let get = client
            .get_domain_name()
            .domain_name(&domain)
            .send()
            .await
            .expect("get_domain_name should succeed");

        assert_eq!(get.domain_name(), Some(domain.as_str()));

        let domains = client
            .get_domain_names()
            .send()
            .await
            .expect("get_domain_names should succeed");

        assert!(
            domains
                .items()
                .iter()
                .any(|d| d.domain_name() == Some(domain.as_str())),
            "listed domains should contain the created domain"
        );

        client
            .delete_domain_name()
            .domain_name(&domain)
            .send()
            .await
            .expect("delete_domain_name should succeed");
    }

    // ---------------------------------------------------------------------------
    // VPC Link
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_vpc_link() {
        let client = apigatewayv2_client();
        let name = format!("test-vpc-{}", unique_id());

        let create = client
            .create_vpc_link()
            .name(&name)
            .subnet_ids("subnet-12345")
            .send()
            .await
            .expect("create_vpc_link should succeed");

        let vpc_link_id = create.vpc_link_id().expect("should have vpc_link_id");

        let get = client
            .get_vpc_link()
            .vpc_link_id(vpc_link_id)
            .send()
            .await
            .expect("get_vpc_link should succeed");

        assert_eq!(get.name(), Some(name.as_str()));

        let links = client
            .get_vpc_links()
            .send()
            .await
            .expect("get_vpc_links should succeed");

        assert!(
            links
                .items()
                .iter()
                .any(|l| l.vpc_link_id() == Some(vpc_link_id)),
            "listed VPC links should contain the created link"
        );

        client
            .delete_vpc_link()
            .vpc_link_id(vpc_link_id)
            .send()
            .await
            .expect("delete_vpc_link should succeed");
    }

    // ---------------------------------------------------------------------------
    // Tags
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_tag_and_untag_api() {
        let client = apigatewayv2_client();
        let name = format!("test-tag-api-{}", unique_id());

        let create = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = create.api_id().expect("should have api_id");

        // Tag the resource.
        let arn = format!("arn:aws:apigateway:us-east-1::/apis/{api_id}");
        client
            .tag_resource()
            .resource_arn(&arn)
            .tags("env", "test")
            .tags("team", "ruststack")
            .send()
            .await
            .expect("tag_resource should succeed");

        // Get tags.
        let tags_resp = client
            .get_tags()
            .resource_arn(&arn)
            .send()
            .await
            .expect("get_tags should succeed");

        let tags = tags_resp.tags().expect("tags should be present");
        assert_eq!(tags.get("env"), Some(&"test".to_owned()));
        assert_eq!(tags.get("team"), Some(&"ruststack".to_owned()));

        // Untag.
        client
            .untag_resource()
            .resource_arn(&arn)
            .tag_keys("team")
            .send()
            .await
            .expect("untag_resource should succeed");

        let tags_resp = client
            .get_tags()
            .resource_arn(&arn)
            .send()
            .await
            .expect("get_tags should succeed");

        let tags = tags_resp.tags().expect("tags should be present");
        assert_eq!(tags.get("env"), Some(&"test".to_owned()));
        assert!(!tags.contains_key("team"), "team tag should be removed");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // API Mapping
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_api_mapping() {
        let client = apigatewayv2_client();
        let name = format!("test-mapping-api-{}", unique_id());
        let domain = format!("{}.example.com", unique_id());

        let api = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = api.api_id().expect("should have api_id");

        client
            .create_stage()
            .api_id(api_id)
            .stage_name("prod")
            .send()
            .await
            .expect("create_stage should succeed");

        client
            .create_domain_name()
            .domain_name(&domain)
            .send()
            .await
            .expect("create_domain_name should succeed");

        let mapping = client
            .create_api_mapping()
            .domain_name(&domain)
            .api_id(api_id)
            .stage("prod")
            .send()
            .await
            .expect("create_api_mapping should succeed");

        let mapping_id = mapping
            .api_mapping_id()
            .expect("should have api_mapping_id");

        let get = client
            .get_api_mapping()
            .domain_name(&domain)
            .api_mapping_id(mapping_id)
            .send()
            .await
            .expect("get_api_mapping should succeed");

        assert_eq!(get.api_id(), Some(api_id));

        let mappings = client
            .get_api_mappings()
            .domain_name(&domain)
            .send()
            .await
            .expect("get_api_mappings should succeed");

        assert!(
            mappings
                .items()
                .iter()
                .any(|m| m.api_mapping_id() == Some(mapping_id)),
            "listed mappings should contain the created mapping"
        );

        client
            .delete_api_mapping()
            .domain_name(&domain)
            .api_mapping_id(mapping_id)
            .send()
            .await
            .expect("delete_api_mapping should succeed");

        client
            .delete_stage()
            .api_id(api_id)
            .stage_name("prod")
            .send()
            .await
            .expect("delete_stage should succeed");

        client
            .delete_domain_name()
            .domain_name(&domain)
            .send()
            .await
            .expect("delete_domain_name should succeed");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // Execution with Mock Integration
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_execute_mock_integration() {
        let client = apigatewayv2_client();
        let name = format!("test-exec-api-{}", unique_id());

        // Create API.
        let api = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = api.api_id().expect("should have api_id");

        // Create mock integration.
        let integration = client
            .create_integration()
            .api_id(api_id)
            .integration_type(IntegrationType::Mock)
            .connection_type(ConnectionType::Internet)
            .send()
            .await
            .expect("create_integration should succeed");

        let integration_id = integration
            .integration_id()
            .expect("should have integration_id");

        // Create route pointing to integration.
        client
            .create_route()
            .api_id(api_id)
            .route_key("GET /mock")
            .target(format!("integrations/{integration_id}"))
            .send()
            .await
            .expect("create_route should succeed");

        // Create stage.
        client
            .create_stage()
            .api_id(api_id)
            .stage_name("test")
            .send()
            .await
            .expect("create_stage should succeed");

        // Execute against the mock.
        let endpoint = crate::endpoint_url();
        let url = format!("{endpoint}/_aws/execute-api/{api_id}/test/mock");

        let http_client = reqwest::Client::new();
        let resp = http_client
            .get(&url)
            .send()
            .await
            .expect("execution request should succeed");

        assert_eq!(
            resp.status().as_u16(),
            200,
            "mock integration should return 200"
        );

        // Cleanup.
        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // CORS Preflight
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_handle_cors_preflight() {
        let client = apigatewayv2_client();
        let name = format!("test-cors-api-{}", unique_id());

        let cors = aws_sdk_apigatewayv2::types::Cors::builder()
            .allow_origins("*")
            .allow_methods("GET")
            .allow_methods("POST")
            .allow_headers("Content-Type")
            .build();

        let api = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .cors_configuration(cors)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = api.api_id().expect("should have api_id");

        // Create mock integration and route.
        let integration = client
            .create_integration()
            .api_id(api_id)
            .integration_type(IntegrationType::Mock)
            .connection_type(ConnectionType::Internet)
            .send()
            .await
            .expect("create_integration should succeed");

        let integration_id = integration
            .integration_id()
            .expect("should have integration_id");

        client
            .create_route()
            .api_id(api_id)
            .route_key("OPTIONS /cors-test")
            .target(format!("integrations/{integration_id}"))
            .send()
            .await
            .expect("create_route should succeed");

        client
            .create_stage()
            .api_id(api_id)
            .stage_name("dev")
            .send()
            .await
            .expect("create_stage should succeed");

        // Send OPTIONS preflight.
        let endpoint = crate::endpoint_url();
        let url = format!("{endpoint}/_aws/execute-api/{api_id}/dev/cors-test");

        let http_client = reqwest::Client::new();
        let resp = http_client
            .request(reqwest::Method::OPTIONS, &url)
            .header("Origin", "https://example.com")
            .header("Access-Control-Request-Method", "GET")
            .send()
            .await
            .expect("OPTIONS request should succeed");

        // CORS preflight should return 204 or 200.
        let status = resp.status().as_u16();
        assert!(
            status == 200 || status == 204,
            "CORS preflight should return 200 or 204, got {status}"
        );

        // Cleanup.
        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");
    }

    // ---------------------------------------------------------------------------
    // Full Lifecycle
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    #[allow(clippy::too_many_lines)]
    async fn test_should_handle_full_api_lifecycle() {
        let client = apigatewayv2_client();
        let name = format!("test-lifecycle-{}", unique_id());

        // 1. Create API.
        let api = client
            .create_api()
            .name(&name)
            .protocol_type(ProtocolType::Http)
            .send()
            .await
            .expect("create_api should succeed");

        let api_id = api.api_id().expect("should have api_id");

        // 2. Create integration.
        let integration = client
            .create_integration()
            .api_id(api_id)
            .integration_type(IntegrationType::Mock)
            .connection_type(ConnectionType::Internet)
            .send()
            .await
            .expect("create_integration should succeed");

        let int_id = integration
            .integration_id()
            .expect("should have integration_id");

        // 3. Create route.
        let route = client
            .create_route()
            .api_id(api_id)
            .route_key("GET /lifecycle")
            .target(format!("integrations/{int_id}"))
            .send()
            .await
            .expect("create_route should succeed");

        let route_id = route.route_id().expect("should have route_id");

        // 4. Create stage.
        client
            .create_stage()
            .api_id(api_id)
            .stage_name("prod")
            .send()
            .await
            .expect("create_stage should succeed");

        // 5. Create deployment.
        let deploy = client
            .create_deployment()
            .api_id(api_id)
            .stage_name("prod")
            .send()
            .await
            .expect("create_deployment should succeed");

        let deploy_id = deploy.deployment_id().expect("should have deployment_id");

        // 6. Execute.
        let endpoint = crate::endpoint_url();
        let url = format!("{endpoint}/_aws/execute-api/{api_id}/prod/lifecycle");

        let http_client = reqwest::Client::new();
        let resp = http_client
            .get(&url)
            .send()
            .await
            .expect("execution request should succeed");

        assert_eq!(
            resp.status().as_u16(),
            200,
            "mock execution should return 200"
        );

        // 7. Update route.
        client
            .update_route()
            .api_id(api_id)
            .route_id(route_id)
            .route_key("POST /lifecycle")
            .send()
            .await
            .expect("update_route should succeed");

        // 8. Update integration.
        client
            .update_integration()
            .api_id(api_id)
            .integration_id(int_id)
            .description("Updated mock")
            .send()
            .await
            .expect("update_integration should succeed");

        // 9. Update stage.
        client
            .update_stage()
            .api_id(api_id)
            .stage_name("prod")
            .description("Production stage")
            .send()
            .await
            .expect("update_stage should succeed");

        // 10. Cleanup in reverse order.
        client
            .delete_deployment()
            .api_id(api_id)
            .deployment_id(deploy_id)
            .send()
            .await
            .expect("delete_deployment should succeed");

        client
            .delete_route()
            .api_id(api_id)
            .route_id(route_id)
            .send()
            .await
            .expect("delete_route should succeed");

        client
            .delete_integration()
            .api_id(api_id)
            .integration_id(int_id)
            .send()
            .await
            .expect("delete_integration should succeed");

        client
            .delete_stage()
            .api_id(api_id)
            .stage_name("prod")
            .send()
            .await
            .expect("delete_stage should succeed");

        client
            .delete_api()
            .api_id(api_id)
            .send()
            .await
            .expect("delete_api should succeed");

        // Verify API is deleted.
        let result = client.get_api().api_id(api_id).send().await;
        assert!(result.is_err(), "get_api should fail after deletion");
    }
}
