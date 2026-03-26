//! Lambda integration tests against a running `RustStack` server.
//!
//! These tests cover the Lambda CRUD operations, versioning, aliases,
//! permissions, tags, account settings, and function URL configurations.

#[cfg(test)]
mod tests {
    use aws_sdk_lambda::{
        primitives::Blob,
        types::{Architecture, Environment, FunctionCode, FunctionUrlAuthType, Runtime},
    };

    use crate::lambda_client;

    /// Helper: generate a unique function name.
    fn func_name(prefix: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
        format!("test-{prefix}-{id}")
    }

    /// Create a minimal zip file (PK header + dummy data) as base64 for code uploads.
    fn dummy_zip_blob() -> Blob {
        // Minimal zip content (not a real zip, but enough for the server to store).
        Blob::new(b"PK\x03\x04fake-lambda-code".to_vec())
    }

    /// Helper: create a function and return its name.
    async fn create_test_function(client: &aws_sdk_lambda::Client, prefix: &str) -> String {
        let name = func_name(prefix);
        client
            .create_function()
            .function_name(&name)
            .runtime(Runtime::Python312)
            .role("arn:aws:iam::000000000000:role/test-role")
            .handler("index.handler")
            .code(FunctionCode::builder().zip_file(dummy_zip_blob()).build())
            .send()
            .await
            .unwrap_or_else(|e| panic!("failed to create function {name}: {e}"));
        name
    }

    /// Helper: delete a function (ignoring errors).
    async fn cleanup_function(client: &aws_sdk_lambda::Client, name: &str) {
        let _ = client.delete_function().function_name(name).send().await;
    }

    // ---------------------------------------------------------------------------
    // CreateFunction + GetFunction
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_function() {
        let client = lambda_client();
        let name = create_test_function(&client, "create-get").await;

        let resp = client
            .get_function()
            .function_name(&name)
            .send()
            .await
            .expect("get should succeed");

        let config = resp.configuration().expect("should have configuration");
        assert_eq!(config.function_name(), Some(name.as_str()));
        assert_eq!(config.runtime(), Some(&Runtime::Python312));
        assert_eq!(config.handler(), Some("index.handler"));
        assert_eq!(config.state(), Some(&aws_sdk_lambda::types::State::Active));

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // GetFunctionConfiguration
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_function_configuration() {
        let client = lambda_client();
        let name = create_test_function(&client, "get-config").await;

        let config = client
            .get_function_configuration()
            .function_name(&name)
            .send()
            .await
            .expect("get config should succeed");

        assert_eq!(config.function_name(), Some(name.as_str()));
        assert_eq!(config.timeout(), Some(3)); // default
        assert_eq!(config.memory_size(), Some(128)); // default

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // UpdateFunctionConfiguration
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_function_configuration() {
        let client = lambda_client();
        let name = create_test_function(&client, "update-config").await;

        let updated = client
            .update_function_configuration()
            .function_name(&name)
            .timeout(30)
            .memory_size(256)
            .description("Updated description")
            .environment(
                Environment::builder()
                    .variables("MY_VAR", "my_value")
                    .build(),
            )
            .send()
            .await
            .expect("update config should succeed");

        assert_eq!(updated.timeout(), Some(30));
        assert_eq!(updated.memory_size(), Some(256));
        assert_eq!(updated.description(), Some("Updated description"));

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // UpdateFunctionCode
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_function_code() {
        let client = lambda_client();
        let name = create_test_function(&client, "update-code").await;

        let updated = client
            .update_function_code()
            .function_name(&name)
            .zip_file(Blob::new(b"PK\x03\x04new-code-data".to_vec()))
            .send()
            .await
            .expect("update code should succeed");

        assert_eq!(updated.function_name(), Some(name.as_str()));
        assert!(updated.code_size() > 0);

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // DeleteFunction
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_function() {
        let client = lambda_client();
        let name = create_test_function(&client, "delete").await;

        client
            .delete_function()
            .function_name(&name)
            .send()
            .await
            .expect("delete should succeed");

        // Getting the function should now fail.
        let err = client.get_function().function_name(&name).send().await;
        assert!(err.is_err());
    }

    // ---------------------------------------------------------------------------
    // ListFunctions
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_functions() {
        let client = lambda_client();
        let name1 = create_test_function(&client, "list-a").await;
        let name2 = create_test_function(&client, "list-b").await;

        let resp = client
            .list_functions()
            .send()
            .await
            .expect("list should succeed");

        let found: Vec<&str> = resp
            .functions()
            .iter()
            .filter_map(|f| f.function_name())
            .collect();
        assert!(found.contains(&name1.as_str()));
        assert!(found.contains(&name2.as_str()));

        cleanup_function(&client, &name1).await;
        cleanup_function(&client, &name2).await;
    }

    // ---------------------------------------------------------------------------
    // PublishVersion + ListVersionsByFunction
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_publish_version_and_list() {
        let client = lambda_client();
        let name = create_test_function(&client, "publish").await;

        // Publish version 1.
        let v1 = client
            .publish_version()
            .function_name(&name)
            .description("Version 1")
            .send()
            .await
            .expect("publish should succeed");
        assert_eq!(v1.version(), Some("1"));

        // Publish version 2.
        let v2 = client
            .publish_version()
            .function_name(&name)
            .send()
            .await
            .expect("publish v2 should succeed");
        assert_eq!(v2.version(), Some("2"));

        // List versions.
        let resp = client
            .list_versions_by_function()
            .function_name(&name)
            .send()
            .await
            .expect("list versions should succeed");

        let versions: Vec<&str> = resp.versions().iter().filter_map(|v| v.version()).collect();
        assert!(versions.contains(&"$LATEST"));
        assert!(versions.contains(&"1"));
        assert!(versions.contains(&"2"));

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // CreateAlias + GetAlias + UpdateAlias + DeleteAlias + ListAliases
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_aliases() {
        let client = lambda_client();
        let name = create_test_function(&client, "alias").await;

        // Publish a version first.
        client
            .publish_version()
            .function_name(&name)
            .send()
            .await
            .expect("publish should succeed");

        // Create alias.
        let alias = client
            .create_alias()
            .function_name(&name)
            .name("prod")
            .function_version("1")
            .description("Production alias")
            .send()
            .await
            .expect("create alias should succeed");
        assert_eq!(alias.name(), Some("prod"));
        assert_eq!(alias.function_version(), Some("1"));

        // Get alias.
        let got = client
            .get_alias()
            .function_name(&name)
            .name("prod")
            .send()
            .await
            .expect("get alias should succeed");
        assert_eq!(got.function_version(), Some("1"));

        // Update alias (publish v2 first, then update to point to it).
        client
            .publish_version()
            .function_name(&name)
            .send()
            .await
            .unwrap();
        let updated = client
            .update_alias()
            .function_name(&name)
            .name("prod")
            .function_version("2")
            .send()
            .await
            .expect("update alias should succeed");
        assert_eq!(updated.function_version(), Some("2"));

        // List aliases.
        let resp = client
            .list_aliases()
            .function_name(&name)
            .send()
            .await
            .expect("list aliases should succeed");
        assert_eq!(resp.aliases().len(), 1);
        assert_eq!(resp.aliases()[0].name(), Some("prod"));

        // Delete alias.
        client
            .delete_alias()
            .function_name(&name)
            .name("prod")
            .send()
            .await
            .expect("delete alias should succeed");

        // List should be empty now.
        let resp = client
            .list_aliases()
            .function_name(&name)
            .send()
            .await
            .expect("list after delete should succeed");
        assert!(resp.aliases().is_empty());

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // AddPermission + GetPolicy + RemovePermission
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_permissions() {
        let client = lambda_client();
        let name = create_test_function(&client, "perm").await;

        // Add permission.
        let resp = client
            .add_permission()
            .function_name(&name)
            .statement_id("s3-invoke")
            .action("lambda:InvokeFunction")
            .principal("s3.amazonaws.com")
            .send()
            .await
            .expect("add permission should succeed");
        assert!(resp.statement().is_some());

        // Get policy.
        let policy = client
            .get_policy()
            .function_name(&name)
            .send()
            .await
            .expect("get policy should succeed");
        let policy_str = policy.policy().expect("should have policy");
        assert!(policy_str.contains("s3-invoke"));

        // Remove permission.
        client
            .remove_permission()
            .function_name(&name)
            .statement_id("s3-invoke")
            .send()
            .await
            .expect("remove permission should succeed");

        // Policy should be empty (not found).
        let err = client.get_policy().function_name(&name).send().await;
        assert!(err.is_err());

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // TagResource + ListTags + UntagResource
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_tags() {
        let client = lambda_client();
        let name = create_test_function(&client, "tags").await;

        // Get the function ARN.
        let func = client
            .get_function()
            .function_name(&name)
            .send()
            .await
            .unwrap();
        let arn = func
            .configuration()
            .unwrap()
            .function_arn()
            .unwrap()
            .to_owned();

        // Tag the function.
        client
            .tag_resource()
            .resource(&arn)
            .tags("env", "test")
            .tags("team", "platform")
            .send()
            .await
            .expect("tag should succeed");

        // List tags.
        let tags = client
            .list_tags()
            .resource(&arn)
            .send()
            .await
            .expect("list tags should succeed");
        let tag_map = tags.tags().expect("should have tags");
        assert_eq!(tag_map.get("env"), Some(&"test".to_owned()));
        assert_eq!(tag_map.get("team"), Some(&"platform".to_owned()));

        // Untag.
        client
            .untag_resource()
            .resource(&arn)
            .tag_keys("team")
            .send()
            .await
            .expect("untag should succeed");

        // Verify tag removed.
        let tags = client.list_tags().resource(&arn).send().await.unwrap();
        let tag_map = tags.tags().expect("should have tags");
        assert!(tag_map.get("team").is_none());
        assert_eq!(tag_map.get("env"), Some(&"test".to_owned()));

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // GetAccountSettings
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_account_settings() {
        let client = lambda_client();

        let resp = client
            .get_account_settings()
            .send()
            .await
            .expect("get account settings should succeed");

        let limit = resp.account_limit().expect("should have limit");
        assert!(limit.total_code_size() > 0);
        assert!(limit.concurrent_executions() > 0);
    }

    // ---------------------------------------------------------------------------
    // CreateFunctionUrlConfig + GetFunctionUrlConfig + UpdateFunctionUrlConfig
    // + DeleteFunctionUrlConfig
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_function_url_config() {
        let client = lambda_client();
        let name = create_test_function(&client, "url").await;

        // Create URL config.
        let created = client
            .create_function_url_config()
            .function_name(&name)
            .auth_type(FunctionUrlAuthType::None)
            .send()
            .await
            .expect("create url config should succeed");
        assert!(created.function_url().contains("lambda-url"));
        assert_eq!(created.auth_type(), &FunctionUrlAuthType::None);

        // Get URL config.
        let got = client
            .get_function_url_config()
            .function_name(&name)
            .send()
            .await
            .expect("get url config should succeed");
        assert_eq!(got.function_url(), created.function_url());

        // Update URL config.
        let updated = client
            .update_function_url_config()
            .function_name(&name)
            .auth_type(FunctionUrlAuthType::AwsIam)
            .send()
            .await
            .expect("update url config should succeed");
        assert_eq!(updated.auth_type(), &FunctionUrlAuthType::AwsIam);

        // Delete URL config.
        client
            .delete_function_url_config()
            .function_name(&name)
            .send()
            .await
            .expect("delete url config should succeed");

        // Get should fail now.
        let err = client
            .get_function_url_config()
            .function_name(&name)
            .send()
            .await;
        assert!(err.is_err());

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // Invoke (DryRun)
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_invoke_dry_run() {
        let client = lambda_client();
        let name = create_test_function(&client, "invoke-dry").await;

        let resp = client
            .invoke()
            .function_name(&name)
            .invocation_type(aws_sdk_lambda::types::InvocationType::DryRun)
            .payload(Blob::new(b"{}".to_vec()))
            .send()
            .await
            .expect("dry run should succeed");

        assert_eq!(resp.status_code(), 204);

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // CreateFunction with architectures
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_function_with_arm64() {
        let client = lambda_client();
        let name = func_name("arm64");

        let resp = client
            .create_function()
            .function_name(&name)
            .runtime(Runtime::Python312)
            .role("arn:aws:iam::000000000000:role/test-role")
            .handler("index.handler")
            .architectures(Architecture::Arm64)
            .code(FunctionCode::builder().zip_file(dummy_zip_blob()).build())
            .send()
            .await
            .expect("create with arm64 should succeed");

        let archs = resp.architectures();
        assert!(archs.contains(&Architecture::Arm64));

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // Error: duplicate function name
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_reject_duplicate_function_name() {
        let client = lambda_client();
        let name = create_test_function(&client, "dup").await;

        let err = client
            .create_function()
            .function_name(&name)
            .runtime(Runtime::Python312)
            .role("arn:aws:iam::000000000000:role/test-role")
            .handler("index.handler")
            .code(FunctionCode::builder().zip_file(dummy_zip_blob()).build())
            .send()
            .await;
        assert!(err.is_err());

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // Error: get nonexistent function
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_error_on_nonexistent_function() {
        let client = lambda_client();

        let err = client
            .get_function()
            .function_name("nonexistent-function-12345")
            .send()
            .await;
        assert!(err.is_err());
    }

    // ---------------------------------------------------------------------------
    // Publish on create (publish=true)
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_publish_on_create() {
        let client = lambda_client();
        let name = func_name("pub-create");

        let resp = client
            .create_function()
            .function_name(&name)
            .runtime(Runtime::Python312)
            .role("arn:aws:iam::000000000000:role/test-role")
            .handler("index.handler")
            .publish(true)
            .code(FunctionCode::builder().zip_file(dummy_zip_blob()).build())
            .send()
            .await
            .expect("create with publish should succeed");

        // When publish=true, the response version should be "1".
        assert_eq!(resp.version(), Some("1"));

        // Should have $LATEST and version 1.
        let versions = client
            .list_versions_by_function()
            .function_name(&name)
            .send()
            .await
            .unwrap();
        assert_eq!(versions.versions().len(), 2);

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // Layers: PublishLayerVersion + GetLayerVersion
    // ---------------------------------------------------------------------------

    /// Minimal valid zip file (22-byte empty zip archive).
    fn minimal_zip_blob() -> Blob {
        Blob::new(vec![
            80, 75, 5, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ])
    }

    /// Helper: generate a unique layer name.
    fn layer_name(prefix: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
        format!("test-layer-{prefix}-{id}")
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_publish_and_get_layer_version() {
        use aws_sdk_lambda::types::LayerVersionContentInput;

        let client = lambda_client();
        let name = layer_name("pub-get");

        // Publish a layer version.
        let published = client
            .publish_layer_version()
            .layer_name(&name)
            .description("Test layer v1")
            .content(
                LayerVersionContentInput::builder()
                    .zip_file(minimal_zip_blob())
                    .build(),
            )
            .compatible_runtimes(Runtime::Python312)
            .send()
            .await
            .expect("publish layer version should succeed");

        assert_eq!(published.version(), 1);
        assert!(
            published.layer_arn().is_some(),
            "layer ARN should be present"
        );
        assert!(
            published.layer_arn().unwrap_or_default().contains(&name),
            "layer ARN should contain the layer name"
        );
        assert_eq!(published.description(), Some("Test layer v1"));

        // Get the layer version.
        let got = client
            .get_layer_version()
            .layer_name(&name)
            .version_number(1)
            .send()
            .await
            .expect("get layer version should succeed");

        assert_eq!(got.version(), 1);
        assert_eq!(got.description(), Some("Test layer v1"));
        assert!(got.content().is_some());

        // Cleanup: delete the layer version.
        let _ = client
            .delete_layer_version()
            .layer_name(&name)
            .version_number(1)
            .send()
            .await;
    }

    // ---------------------------------------------------------------------------
    // Layers: ListLayers
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_layers() {
        use aws_sdk_lambda::types::LayerVersionContentInput;

        let client = lambda_client();
        let name = layer_name("list");

        // Publish a layer version.
        client
            .publish_layer_version()
            .layer_name(&name)
            .description("Layer for listing")
            .content(
                LayerVersionContentInput::builder()
                    .zip_file(minimal_zip_blob())
                    .build(),
            )
            .send()
            .await
            .expect("publish layer should succeed");

        // List layers and verify ours appears.
        let resp = client
            .list_layers()
            .send()
            .await
            .expect("list layers should succeed");

        let found = resp
            .layers()
            .iter()
            .any(|l| l.layer_name().is_some_and(|n| n == name));
        assert!(found, "published layer should appear in list_layers");

        // Cleanup.
        let _ = client
            .delete_layer_version()
            .layer_name(&name)
            .version_number(1)
            .send()
            .await;
    }

    // ---------------------------------------------------------------------------
    // Layers: DeleteLayerVersion
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_layer_version() {
        use aws_sdk_lambda::types::LayerVersionContentInput;

        let client = lambda_client();
        let name = layer_name("delete");

        // Publish a layer version.
        client
            .publish_layer_version()
            .layer_name(&name)
            .content(
                LayerVersionContentInput::builder()
                    .zip_file(minimal_zip_blob())
                    .build(),
            )
            .send()
            .await
            .expect("publish layer should succeed");

        // Delete the layer version.
        client
            .delete_layer_version()
            .layer_name(&name)
            .version_number(1)
            .send()
            .await
            .expect("delete layer version should succeed");

        // Getting the deleted layer version should fail.
        let err = client
            .get_layer_version()
            .layer_name(&name)
            .version_number(1)
            .send()
            .await;
        assert!(
            err.is_err(),
            "get deleted layer version should return error"
        );
    }

    // ---------------------------------------------------------------------------
    // Event Source Mappings: Create + List
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_list_event_source_mappings() {
        let client = lambda_client();
        let name = create_test_function(&client, "esm-create").await;

        let fake_sqs_arn = format!(
            "arn:aws:sqs:us-east-1:000000000000:test-queue-{}",
            &uuid::Uuid::new_v4().to_string()[..8]
        );

        // Create an event source mapping.
        let created = client
            .create_event_source_mapping()
            .function_name(&name)
            .event_source_arn(&fake_sqs_arn)
            .batch_size(10)
            .enabled(true)
            .send()
            .await
            .expect("create event source mapping should succeed");

        let esm_uuid = created.uuid().expect("should have UUID").to_owned();
        assert_eq!(created.batch_size(), Some(10));
        assert_eq!(created.event_source_arn(), Some(fake_sqs_arn.as_str()));

        // List event source mappings for the function.
        let resp = client
            .list_event_source_mappings()
            .function_name(&name)
            .send()
            .await
            .expect("list event source mappings should succeed");

        let found = resp
            .event_source_mappings()
            .iter()
            .any(|m| m.uuid() == Some(esm_uuid.as_str()));
        assert!(found, "created ESM should appear in list");

        // Cleanup.
        let _ = client
            .delete_event_source_mapping()
            .uuid(&esm_uuid)
            .send()
            .await;
        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // Event Source Mappings: Update
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_event_source_mapping() {
        let client = lambda_client();
        let name = create_test_function(&client, "esm-update").await;

        let fake_sqs_arn = format!(
            "arn:aws:sqs:us-east-1:000000000000:test-queue-{}",
            &uuid::Uuid::new_v4().to_string()[..8]
        );

        // Create an event source mapping.
        let created = client
            .create_event_source_mapping()
            .function_name(&name)
            .event_source_arn(&fake_sqs_arn)
            .batch_size(5)
            .send()
            .await
            .expect("create ESM should succeed");

        let esm_uuid = created.uuid().expect("should have UUID").to_owned();

        // Update batch size.
        let updated = client
            .update_event_source_mapping()
            .uuid(&esm_uuid)
            .batch_size(20)
            .send()
            .await
            .expect("update ESM should succeed");

        assert_eq!(updated.batch_size(), Some(20));

        // Get and verify.
        let got = client
            .get_event_source_mapping()
            .uuid(&esm_uuid)
            .send()
            .await
            .expect("get ESM should succeed");

        assert_eq!(got.batch_size(), Some(20));

        // Cleanup.
        let _ = client
            .delete_event_source_mapping()
            .uuid(&esm_uuid)
            .send()
            .await;
        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // Event Source Mappings: Delete
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_event_source_mapping() {
        let client = lambda_client();
        let name = create_test_function(&client, "esm-delete").await;

        let fake_sqs_arn = format!(
            "arn:aws:sqs:us-east-1:000000000000:test-queue-{}",
            &uuid::Uuid::new_v4().to_string()[..8]
        );

        // Create an event source mapping.
        let created = client
            .create_event_source_mapping()
            .function_name(&name)
            .event_source_arn(&fake_sqs_arn)
            .batch_size(10)
            .send()
            .await
            .expect("create ESM should succeed");

        let esm_uuid = created.uuid().expect("should have UUID").to_owned();

        // Delete the event source mapping.
        client
            .delete_event_source_mapping()
            .uuid(&esm_uuid)
            .send()
            .await
            .expect("delete ESM should succeed");

        // Getting the deleted ESM should fail.
        let err = client
            .get_event_source_mapping()
            .uuid(&esm_uuid)
            .send()
            .await;
        assert!(err.is_err(), "get deleted ESM should return error");

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // Function Concurrency: Put + Get + Delete
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_and_get_function_concurrency() {
        let client = lambda_client();
        let name = create_test_function(&client, "concurrency").await;

        // Put function concurrency.
        let put_resp = client
            .put_function_concurrency()
            .function_name(&name)
            .reserved_concurrent_executions(50)
            .send()
            .await
            .expect("put concurrency should succeed");

        assert_eq!(put_resp.reserved_concurrent_executions(), Some(50));

        // Get function concurrency directly.
        let get_resp = client
            .get_function_concurrency()
            .function_name(&name)
            .send()
            .await
            .expect("get concurrency should succeed");

        assert_eq!(get_resp.reserved_concurrent_executions(), Some(50));

        // Delete function concurrency.
        client
            .delete_function_concurrency()
            .function_name(&name)
            .send()
            .await
            .expect("delete concurrency should succeed");

        // Verify concurrency is removed — get_function_concurrency returns None or 0.
        let after = client
            .get_function_concurrency()
            .function_name(&name)
            .send()
            .await
            .expect("get concurrency after delete should succeed");

        assert!(
            after.reserved_concurrent_executions().is_none()
                || after.reserved_concurrent_executions() == Some(0),
            "concurrency should be cleared after deletion"
        );

        cleanup_function(&client, &name).await;
    }

    // ---------------------------------------------------------------------------
    // Event Invoke Config: Put + List
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_and_list_event_invoke_config() {
        let client = lambda_client();
        let name = create_test_function(&client, "invoke-cfg").await;

        // Put function event invoke config.
        let put_resp = client
            .put_function_event_invoke_config()
            .function_name(&name)
            .maximum_retry_attempts(1)
            .maximum_event_age_in_seconds(300)
            .send()
            .await
            .expect("put event invoke config should succeed");

        assert_eq!(put_resp.maximum_retry_attempts(), Some(1));
        assert_eq!(put_resp.maximum_event_age_in_seconds(), Some(300));

        // List function event invoke configs.
        let list_resp = client
            .list_function_event_invoke_configs()
            .function_name(&name)
            .send()
            .await
            .expect("list event invoke configs should succeed");

        let configs = list_resp.function_event_invoke_configs();
        assert!(
            !configs.is_empty(),
            "should have at least one event invoke config"
        );

        let our_config = configs
            .iter()
            .find(|c| c.function_arn().is_some_and(|arn| arn.contains(&name)))
            .expect("should find config for our function");

        assert_eq!(our_config.maximum_retry_attempts(), Some(1));
        assert_eq!(our_config.maximum_event_age_in_seconds(), Some(300));

        // Cleanup: delete event invoke config, then function.
        let _ = client
            .delete_function_event_invoke_config()
            .function_name(&name)
            .send()
            .await;
        cleanup_function(&client, &name).await;
    }
}
