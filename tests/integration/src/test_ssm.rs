//! SSM Parameter Store integration tests against a running `RustStack` server.
//!
//! These tests cover all 13 SSM Parameter Store operations: CRUD, path queries,
//! version/label management, tags, describe, and history.

#[cfg(test)]
mod tests {
    use aws_sdk_ssm::types::{ParameterStringFilter, ParameterType, Tag};

    use crate::ssm_client;

    /// Helper: generate a unique parameter name.
    fn param_name(prefix: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
        format!("/test/{prefix}/{id}")
    }

    // ---------------------------------------------------------------------------
    // PutParameter + GetParameter
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_and_get_parameter() {
        let client = ssm_client();
        let name = param_name("put-get");

        let put = client
            .put_parameter()
            .name(&name)
            .value("hello-world")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put should succeed");
        assert_eq!(put.version(), 1);

        let get = client
            .get_parameter()
            .name(&name)
            .send()
            .await
            .expect("get should succeed");
        let param = get.parameter().expect("parameter should exist");
        assert_eq!(param.value(), Some("hello-world"));
        assert_eq!(param.version(), 1);

        // Cleanup.
        let _ = client.delete_parameter().name(&name).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_overwrite_parameter() {
        let client = ssm_client();
        let name = param_name("overwrite");

        client
            .put_parameter()
            .name(&name)
            .value("v1")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put v1");

        let put2 = client
            .put_parameter()
            .name(&name)
            .value("v2")
            .r#type(ParameterType::String)
            .overwrite(true)
            .send()
            .await
            .expect("put v2");
        assert_eq!(put2.version(), 2);

        let get = client
            .get_parameter()
            .name(&name)
            .send()
            .await
            .expect("get");
        assert_eq!(get.parameter().unwrap().value(), Some("v2"));

        let _ = client.delete_parameter().name(&name).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_reject_put_without_overwrite() {
        let client = ssm_client();
        let name = param_name("no-overwrite");

        client
            .put_parameter()
            .name(&name)
            .value("v1")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put v1");

        let err = client
            .put_parameter()
            .name(&name)
            .value("v2")
            .r#type(ParameterType::String)
            .send()
            .await;
        assert!(err.is_err(), "should reject without overwrite");

        let _ = client.delete_parameter().name(&name).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_secure_string() {
        let client = ssm_client();
        let name = param_name("secure");

        client
            .put_parameter()
            .name(&name)
            .value("my-secret")
            .r#type(ParameterType::SecureString)
            .send()
            .await
            .expect("put secure string");

        let get = client
            .get_parameter()
            .name(&name)
            .send()
            .await
            .expect("get");
        let param = get.parameter().unwrap();
        assert_eq!(param.value(), Some("my-secret"));

        let _ = client.delete_parameter().name(&name).send().await;
    }

    // ---------------------------------------------------------------------------
    // GetParameters (batch)
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_parameters_batch() {
        let client = ssm_client();
        let name1 = param_name("batch1");
        let name2 = param_name("batch2");
        let missing = param_name("missing");

        client
            .put_parameter()
            .name(&name1)
            .value("val1")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put 1");
        client
            .put_parameter()
            .name(&name2)
            .value("val2")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put 2");

        let resp = client
            .get_parameters()
            .names(&name1)
            .names(&name2)
            .names(&missing)
            .send()
            .await
            .expect("get_parameters");

        assert_eq!(resp.parameters().len(), 2);
        assert_eq!(resp.invalid_parameters().len(), 1);
        assert!(resp.invalid_parameters().contains(&missing));

        let _ = client.delete_parameter().name(&name1).send().await;
        let _ = client.delete_parameter().name(&name2).send().await;
    }

    // ---------------------------------------------------------------------------
    // GetParametersByPath
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_parameters_by_path() {
        let client = ssm_client();
        let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
        let base = format!("/test/path/{id}");
        let p1 = format!("{base}/host");
        let p2 = format!("{base}/port");
        let p3 = format!("{base}/sub/deep");

        for (name, val) in [(&p1, "h"), (&p2, "p"), (&p3, "d")] {
            client
                .put_parameter()
                .name(name)
                .value(val)
                .r#type(ParameterType::String)
                .send()
                .await
                .expect("put");
        }

        // Non-recursive: only direct children.
        let resp = client
            .get_parameters_by_path()
            .path(&base)
            .send()
            .await
            .expect("by path");
        assert_eq!(resp.parameters().len(), 2);

        // Recursive: all descendants.
        let resp = client
            .get_parameters_by_path()
            .path(&base)
            .recursive(true)
            .send()
            .await
            .expect("by path recursive");
        assert_eq!(resp.parameters().len(), 3);

        for name in [&p1, &p2, &p3] {
            let _ = client.delete_parameter().name(name).send().await;
        }
    }

    // ---------------------------------------------------------------------------
    // DeleteParameter + DeleteParameters
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_parameter() {
        let client = ssm_client();
        let name = param_name("delete");

        client
            .put_parameter()
            .name(&name)
            .value("val")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put");

        client
            .delete_parameter()
            .name(&name)
            .send()
            .await
            .expect("delete");

        let err = client.get_parameter().name(&name).send().await;
        assert!(err.is_err(), "should not find deleted parameter");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_parameters_batch() {
        let client = ssm_client();
        let name1 = param_name("delbatch1");
        let name2 = param_name("delbatch2");
        let missing = param_name("delmissing");

        client
            .put_parameter()
            .name(&name1)
            .value("v1")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put");
        client
            .put_parameter()
            .name(&name2)
            .value("v2")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put");

        let resp = client
            .delete_parameters()
            .names(&name1)
            .names(&name2)
            .names(&missing)
            .send()
            .await
            .expect("delete_parameters");

        assert_eq!(resp.deleted_parameters().len(), 2);
        assert_eq!(resp.invalid_parameters().len(), 1);
    }

    // ---------------------------------------------------------------------------
    // DescribeParameters
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_describe_parameters() {
        let client = ssm_client();
        let id = uuid::Uuid::new_v4().to_string()[..8].to_owned();
        let base = format!("/test/describe/{id}");
        let names: Vec<String> = (0..3).map(|i| format!("{base}/p{i}")).collect();

        for name in &names {
            client
                .put_parameter()
                .name(name)
                .value("v")
                .r#type(ParameterType::String)
                .send()
                .await
                .expect("put");
        }

        // Filter by name prefix.
        let resp = client
            .describe_parameters()
            .parameter_filters(
                ParameterStringFilter::builder()
                    .key("Name")
                    .option("BeginsWith")
                    .values(&base)
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("describe");
        assert_eq!(resp.parameters().len(), 3);

        for name in &names {
            let _ = client.delete_parameter().name(name).send().await;
        }
    }

    // ---------------------------------------------------------------------------
    // GetParameterHistory
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_parameter_history() {
        let client = ssm_client();
        let name = param_name("history");

        for i in 1..=3 {
            client
                .put_parameter()
                .name(&name)
                .value(format!("v{i}"))
                .r#type(ParameterType::String)
                .overwrite(i > 1)
                .send()
                .await
                .expect("put");
        }

        let resp = client
            .get_parameter_history()
            .name(&name)
            .send()
            .await
            .expect("history");
        assert_eq!(resp.parameters().len(), 3);

        let _ = client.delete_parameter().name(&name).send().await;
    }

    // ---------------------------------------------------------------------------
    // Tags
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_tags() {
        let client = ssm_client();
        let name = param_name("tags");

        client
            .put_parameter()
            .name(&name)
            .value("val")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put");

        // Add tags.
        client
            .add_tags_to_resource()
            .resource_type(aws_sdk_ssm::types::ResourceTypeForTagging::Parameter)
            .resource_id(&name)
            .tags(Tag::builder().key("env").value("prod").build().unwrap())
            .tags(Tag::builder().key("team").value("infra").build().unwrap())
            .send()
            .await
            .expect("add tags");

        // List tags.
        let resp = client
            .list_tags_for_resource()
            .resource_type(aws_sdk_ssm::types::ResourceTypeForTagging::Parameter)
            .resource_id(&name)
            .send()
            .await
            .expect("list tags");
        assert_eq!(resp.tag_list().len(), 2);

        // Remove one tag.
        client
            .remove_tags_from_resource()
            .resource_type(aws_sdk_ssm::types::ResourceTypeForTagging::Parameter)
            .resource_id(&name)
            .tag_keys("team")
            .send()
            .await
            .expect("remove tags");

        let resp = client
            .list_tags_for_resource()
            .resource_type(aws_sdk_ssm::types::ResourceTypeForTagging::Parameter)
            .resource_id(&name)
            .send()
            .await
            .expect("list tags after remove");
        assert_eq!(resp.tag_list().len(), 1);

        let _ = client.delete_parameter().name(&name).send().await;
    }

    // ---------------------------------------------------------------------------
    // Labels
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_label_and_unlabel_parameter_version() {
        let client = ssm_client();
        let name = param_name("labels");

        // Create version 1 and 2.
        client
            .put_parameter()
            .name(&name)
            .value("v1")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put v1");
        client
            .put_parameter()
            .name(&name)
            .value("v2")
            .r#type(ParameterType::String)
            .overwrite(true)
            .send()
            .await
            .expect("put v2");

        // Label version 1.
        let resp = client
            .label_parameter_version()
            .name(&name)
            .parameter_version(1)
            .labels("release")
            .send()
            .await
            .expect("label");
        assert!(resp.invalid_labels().is_empty());
        assert_eq!(resp.parameter_version(), 1);

        // Get by label selector.
        let get = client
            .get_parameter()
            .name(format!("{name}:release"))
            .send()
            .await
            .expect("get by label");
        assert_eq!(get.parameter().unwrap().value(), Some("v1"));
        assert_eq!(get.parameter().unwrap().version(), 1);

        // Move label to version 2 (labels are unique per parameter).
        client
            .label_parameter_version()
            .name(&name)
            .parameter_version(2)
            .labels("release")
            .send()
            .await
            .expect("relabel");

        let get2 = client
            .get_parameter()
            .name(format!("{name}:release"))
            .send()
            .await
            .expect("get by label after move");
        assert_eq!(get2.parameter().unwrap().version(), 2);

        // Unlabel.
        let unlabel = client
            .unlabel_parameter_version()
            .name(&name)
            .parameter_version(2)
            .labels("release")
            .send()
            .await
            .expect("unlabel");
        assert_eq!(unlabel.removed_labels().len(), 1);

        let _ = client.delete_parameter().name(&name).send().await;
    }

    // ---------------------------------------------------------------------------
    // Version selectors
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_parameter_by_version_selector() {
        let client = ssm_client();
        let name = param_name("version-sel");

        client
            .put_parameter()
            .name(&name)
            .value("v1")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put v1");
        client
            .put_parameter()
            .name(&name)
            .value("v2")
            .r#type(ParameterType::String)
            .overwrite(true)
            .send()
            .await
            .expect("put v2");

        // Get version 1 explicitly.
        let get = client
            .get_parameter()
            .name(format!("{name}:1"))
            .send()
            .await
            .expect("get v1");
        assert_eq!(get.parameter().unwrap().value(), Some("v1"));
        assert_eq!(get.parameter().unwrap().version(), 1);

        // Get latest (version 2).
        let get2 = client
            .get_parameter()
            .name(&name)
            .send()
            .await
            .expect("get latest");
        assert_eq!(get2.parameter().unwrap().value(), Some("v2"));
        assert_eq!(get2.parameter().unwrap().version(), 2);

        let _ = client.delete_parameter().name(&name).send().await;
    }

    // ---------------------------------------------------------------------------
    // Error cases
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_return_parameter_not_found() {
        let client = ssm_client();
        let name = param_name("notfound");

        let err = client.get_parameter().name(&name).send().await;
        assert!(err.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_reject_type_change_on_overwrite() {
        let client = ssm_client();
        let name = param_name("type-change");

        client
            .put_parameter()
            .name(&name)
            .value("val")
            .r#type(ParameterType::String)
            .send()
            .await
            .expect("put as String");

        let err = client
            .put_parameter()
            .name(&name)
            .value("val2")
            .r#type(ParameterType::SecureString)
            .overwrite(true)
            .send()
            .await;
        assert!(err.is_err(), "should reject type change");

        let _ = client.delete_parameter().name(&name).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_support_string_list() {
        let client = ssm_client();
        let name = param_name("stringlist");

        client
            .put_parameter()
            .name(&name)
            .value("a,b,c")
            .r#type(ParameterType::StringList)
            .send()
            .await
            .expect("put StringList");

        let get = client
            .get_parameter()
            .name(&name)
            .send()
            .await
            .expect("get");
        let param = get.parameter().unwrap();
        assert_eq!(param.value(), Some("a,b,c"));

        let _ = client.delete_parameter().name(&name).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_with_tags() {
        let client = ssm_client();
        let name = param_name("with-tags");

        client
            .put_parameter()
            .name(&name)
            .value("val")
            .r#type(ParameterType::String)
            .tags(Tag::builder().key("env").value("staging").build().unwrap())
            .send()
            .await
            .expect("put with tags");

        let tags = client
            .list_tags_for_resource()
            .resource_type(aws_sdk_ssm::types::ResourceTypeForTagging::Parameter)
            .resource_id(&name)
            .send()
            .await
            .expect("list tags");
        assert_eq!(tags.tag_list().len(), 1);
        assert_eq!(tags.tag_list()[0].key(), "env");
        assert_eq!(tags.tag_list()[0].value(), "staging");

        let _ = client.delete_parameter().name(&name).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_with_allowed_pattern() {
        let client = ssm_client();
        let name = param_name("pattern");

        // Pattern matches.
        client
            .put_parameter()
            .name(&name)
            .value("abc123")
            .r#type(ParameterType::String)
            .allowed_pattern("[a-z0-9]+")
            .send()
            .await
            .expect("put with matching pattern");

        // Pattern doesn't match.
        let err = client
            .put_parameter()
            .name(&name)
            .value("ABC!!!")
            .r#type(ParameterType::String)
            .allowed_pattern("[a-z0-9]+")
            .overwrite(true)
            .send()
            .await;
        assert!(err.is_err(), "should reject pattern mismatch");

        let _ = client.delete_parameter().name(&name).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_with_description() {
        let client = ssm_client();
        let name = param_name("desc");

        client
            .put_parameter()
            .name(&name)
            .value("val")
            .r#type(ParameterType::String)
            .description("A test parameter")
            .send()
            .await
            .expect("put with description");

        // Verify description appears in describe output.
        let resp = client
            .describe_parameters()
            .parameter_filters(
                ParameterStringFilter::builder()
                    .key("Name")
                    .option("Equals")
                    .values(&name)
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("describe");
        assert_eq!(resp.parameters().len(), 1);
        assert_eq!(resp.parameters()[0].description(), Some("A test parameter"));

        let _ = client.delete_parameter().name(&name).send().await;
    }
}
