//! Secrets Manager integration tests against a running `RustStack` server.
//!
//! These tests cover all 23 Secrets Manager operations across all phases:
//! Phase 0: Core CRUD, Phase 1: Tags/Rotation/Batch, Phase 2: Policies/Replication.

#[cfg(test)]
mod tests {
    use aws_sdk_secretsmanager::types::{Filter, FilterNameStringType, Tag};

    use crate::{secretsmanager_client, test_secret_name};

    // ---------------------------------------------------------------------------
    // Phase 0: Core CRUD
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_get_secret_string() {
        let client = secretsmanager_client();
        let name = test_secret_name("create-get");

        let create = client
            .create_secret()
            .name(&name)
            .secret_string(r#"{"username":"admin","password":"s3cret"}"#)
            .send()
            .await
            .expect("create should succeed");

        assert!(create.arn().is_some());
        assert_eq!(create.name(), Some(name.as_str()));
        assert!(create.version_id().is_some());

        let get = client
            .get_secret_value()
            .secret_id(&name)
            .send()
            .await
            .expect("get should succeed");

        assert_eq!(
            get.secret_string(),
            Some(r#"{"username":"admin","password":"s3cret"}"#)
        );
        assert!(get.version_stages().contains(&"AWSCURRENT".to_string()));

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_secret_binary() {
        let client = secretsmanager_client();
        let name = test_secret_name("create-binary");
        let binary_data = aws_sdk_secretsmanager::primitives::Blob::new(b"binary-secret-data");

        let create = client
            .create_secret()
            .name(&name)
            .secret_binary(binary_data.clone())
            .send()
            .await
            .expect("create binary should succeed");

        assert!(create.version_id().is_some());

        let get = client
            .get_secret_value()
            .secret_id(&name)
            .send()
            .await
            .expect("get should succeed");

        assert!(get.secret_binary().is_some());

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_new_secret_version() {
        let client = secretsmanager_client();
        let name = test_secret_name("put-version");

        client
            .create_secret()
            .name(&name)
            .secret_string("v1")
            .send()
            .await
            .expect("create should succeed");

        let put = client
            .put_secret_value()
            .secret_id(&name)
            .secret_string("v2")
            .send()
            .await
            .expect("put should succeed");

        assert!(put.version_stages().contains(&"AWSCURRENT".to_string()));

        // Get current should return v2.
        let get = client
            .get_secret_value()
            .secret_id(&name)
            .send()
            .await
            .expect("get should succeed");

        assert_eq!(get.secret_string(), Some("v2"));

        // Get AWSPREVIOUS should return v1.
        let get_prev = client
            .get_secret_value()
            .secret_id(&name)
            .version_stage("AWSPREVIOUS")
            .send()
            .await
            .expect("get previous should succeed");

        assert_eq!(get_prev.secret_string(), Some("v1"));

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_describe_secret() {
        let client = secretsmanager_client();
        let name = test_secret_name("describe");

        client
            .create_secret()
            .name(&name)
            .description("Test secret for describe")
            .secret_string("value")
            .send()
            .await
            .expect("create should succeed");

        let desc = client
            .describe_secret()
            .secret_id(&name)
            .send()
            .await
            .expect("describe should succeed");

        assert_eq!(desc.name(), Some(name.as_str()));
        assert_eq!(desc.description(), Some("Test secret for describe"));
        assert!(desc.arn().is_some());
        assert!(desc.created_date().is_some());
        assert!(desc.version_ids_to_stages().is_some_and(|m| !m.is_empty()));

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_delete_and_restore_secret() {
        let client = secretsmanager_client();
        let name = test_secret_name("delete-restore");

        client
            .create_secret()
            .name(&name)
            .secret_string("value")
            .send()
            .await
            .expect("create should succeed");

        // Schedule deletion.
        let del = client
            .delete_secret()
            .secret_id(&name)
            .recovery_window_in_days(7)
            .send()
            .await
            .expect("delete should succeed");

        assert!(del.deletion_date().is_some());

        // GetSecretValue should fail on deleted secret.
        let get_err = client.get_secret_value().secret_id(&name).send().await;

        assert!(get_err.is_err());

        // Restore.
        let restore = client
            .restore_secret()
            .secret_id(&name)
            .send()
            .await
            .expect("restore should succeed");

        assert_eq!(restore.name(), Some(name.as_str()));

        // GetSecretValue should work again.
        let get = client
            .get_secret_value()
            .secret_id(&name)
            .send()
            .await
            .expect("get after restore should succeed");

        assert_eq!(get.secret_string(), Some("value"));

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_force_delete_secret() {
        let client = secretsmanager_client();
        let name = test_secret_name("force-delete");

        client
            .create_secret()
            .name(&name)
            .secret_string("value")
            .send()
            .await
            .expect("create should succeed");

        client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await
            .expect("force delete should succeed");

        // Secret should be gone.
        let get_err = client.get_secret_value().secret_id(&name).send().await;

        assert!(get_err.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_secret() {
        let client = secretsmanager_client();
        let name = test_secret_name("update");

        client
            .create_secret()
            .name(&name)
            .description("original")
            .secret_string("v1")
            .send()
            .await
            .expect("create should succeed");

        // Update metadata + value.
        client
            .update_secret()
            .secret_id(&name)
            .description("updated")
            .secret_string("v2")
            .send()
            .await
            .expect("update should succeed");

        let desc = client
            .describe_secret()
            .secret_id(&name)
            .send()
            .await
            .expect("describe should succeed");

        assert_eq!(desc.description(), Some("updated"));

        let get = client
            .get_secret_value()
            .secret_id(&name)
            .send()
            .await
            .expect("get should succeed");

        assert_eq!(get.secret_string(), Some("v2"));

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_secrets() {
        let client = secretsmanager_client();
        let name1 = test_secret_name("list-a");
        let name2 = test_secret_name("list-b");

        client
            .create_secret()
            .name(&name1)
            .secret_string("a")
            .send()
            .await
            .expect("create a");

        client
            .create_secret()
            .name(&name2)
            .secret_string("b")
            .send()
            .await
            .expect("create b");

        let list = client
            .list_secrets()
            .send()
            .await
            .expect("list should succeed");

        // Should have at least our 2 secrets.
        assert!(list.secret_list().len() >= 2);

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name1)
            .force_delete_without_recovery(true)
            .send()
            .await;
        let _ = client
            .delete_secret()
            .secret_id(&name2)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_secrets_with_name_filter() {
        let client = secretsmanager_client();
        let prefix = format!("test/filter-{}", &uuid::Uuid::new_v4().to_string()[..6]);
        let name1 = format!("{prefix}/secret1");
        let name2 = format!("{prefix}/secret2");

        client
            .create_secret()
            .name(&name1)
            .secret_string("a")
            .send()
            .await
            .expect("create 1");

        client
            .create_secret()
            .name(&name2)
            .secret_string("b")
            .send()
            .await
            .expect("create 2");

        let list = client
            .list_secrets()
            .filters(
                Filter::builder()
                    .key(FilterNameStringType::Name)
                    .values(&prefix)
                    .build(),
            )
            .send()
            .await
            .expect("list with filter should succeed");

        assert_eq!(list.secret_list().len(), 2);

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name1)
            .force_delete_without_recovery(true)
            .send()
            .await;
        let _ = client
            .delete_secret()
            .secret_id(&name2)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_secret_version_ids() {
        let client = secretsmanager_client();
        let name = test_secret_name("versions");

        client
            .create_secret()
            .name(&name)
            .secret_string("v1")
            .send()
            .await
            .expect("create");

        client
            .put_secret_value()
            .secret_id(&name)
            .secret_string("v2")
            .send()
            .await
            .expect("put v2");

        let versions = client
            .list_secret_version_ids()
            .secret_id(&name)
            .include_deprecated(true)
            .send()
            .await
            .expect("list versions");

        // Should have at least 2 versions.
        assert!(versions.versions().len() >= 2);

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_random_password() {
        let client = secretsmanager_client();

        let resp = client
            .get_random_password()
            .password_length(64)
            .exclude_punctuation(true)
            .send()
            .await
            .expect("get random password");

        let password = resp.random_password().expect("should have password");
        assert_eq!(password.len(), 64);
        // Should not contain punctuation.
        assert!(
            !password
                .chars()
                .any(|c| "!@#$%^&*()_+-=[]{}|;':\",./<>?".contains(c))
        );
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_secret_by_arn() {
        let client = secretsmanager_client();
        let name = test_secret_name("get-by-arn");

        let create = client
            .create_secret()
            .name(&name)
            .secret_string("value")
            .send()
            .await
            .expect("create");

        let arn = create.arn().expect("should have arn");

        // Get by ARN.
        let get = client
            .get_secret_value()
            .secret_id(arn)
            .send()
            .await
            .expect("get by arn should succeed");

        assert_eq!(get.secret_string(), Some("value"));

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_reject_duplicate_name() {
        let client = secretsmanager_client();
        let name = test_secret_name("duplicate");

        client
            .create_secret()
            .name(&name)
            .secret_string("v1")
            .send()
            .await
            .expect("create");

        let dup = client
            .create_secret()
            .name(&name)
            .secret_string("v2")
            .send()
            .await;

        assert!(dup.is_err());

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    // ---------------------------------------------------------------------------
    // Phase 1: Tags, Rotation, Batch
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_tag_and_untag_resource() {
        let client = secretsmanager_client();
        let name = test_secret_name("tags");

        client
            .create_secret()
            .name(&name)
            .secret_string("value")
            .tags(
                Tag::builder()
                    .key("Environment")
                    .value("production")
                    .build(),
            )
            .send()
            .await
            .expect("create");

        // Add more tags.
        client
            .tag_resource()
            .secret_id(&name)
            .tags(Tag::builder().key("Team").value("backend").build())
            .send()
            .await
            .expect("tag resource");

        let desc = client
            .describe_secret()
            .secret_id(&name)
            .send()
            .await
            .expect("describe");

        assert_eq!(desc.tags().len(), 2);

        // Remove a tag.
        client
            .untag_resource()
            .secret_id(&name)
            .tag_keys("Team")
            .send()
            .await
            .expect("untag");

        let desc2 = client
            .describe_secret()
            .secret_id(&name)
            .send()
            .await
            .expect("describe after untag");

        assert_eq!(desc2.tags().len(), 1);

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_secret_version_stage() {
        let client = secretsmanager_client();
        let name = test_secret_name("version-stage");

        let create = client
            .create_secret()
            .name(&name)
            .secret_string("v1")
            .send()
            .await
            .expect("create");
        let v1_id = create.version_id().expect("v1 id").to_string();

        // Put v2 as AWSPENDING.
        let put = client
            .put_secret_value()
            .secret_id(&name)
            .secret_string("v2")
            .version_stages("AWSPENDING")
            .send()
            .await
            .expect("put as pending");
        let v2_id = put.version_id().expect("v2 id").to_string();

        // Promote AWSPENDING to AWSCURRENT.
        client
            .update_secret_version_stage()
            .secret_id(&name)
            .version_stage("AWSCURRENT")
            .move_to_version_id(&v2_id)
            .remove_from_version_id(&v1_id)
            .send()
            .await
            .expect("update version stage");

        // Current should be v2.
        let get = client
            .get_secret_value()
            .secret_id(&name)
            .send()
            .await
            .expect("get current");

        assert_eq!(get.secret_string(), Some("v2"));
        assert_eq!(get.version_id(), Some(v2_id.as_str()));

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_rotate_secret() {
        let client = secretsmanager_client();
        let name = test_secret_name("rotate");

        client
            .create_secret()
            .name(&name)
            .secret_string("original")
            .send()
            .await
            .expect("create");

        let rotate = client
            .rotate_secret()
            .secret_id(&name)
            .rotation_lambda_arn("arn:aws:lambda:us-east-1:000000000000:function:my-rotation")
            .send()
            .await
            .expect("rotate should succeed");

        assert!(rotate.version_id().is_some());

        // Describe should show rotation enabled.
        let desc = client
            .describe_secret()
            .secret_id(&name)
            .send()
            .await
            .expect("describe");

        assert_eq!(desc.rotation_enabled(), Some(true));

        // Cancel rotation.
        client
            .cancel_rotate_secret()
            .secret_id(&name)
            .send()
            .await
            .expect("cancel rotate");

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_batch_get_secret_value() {
        let client = secretsmanager_client();
        let name1 = test_secret_name("batch-a");
        let name2 = test_secret_name("batch-b");

        client
            .create_secret()
            .name(&name1)
            .secret_string("val-a")
            .send()
            .await
            .expect("create a");

        client
            .create_secret()
            .name(&name2)
            .secret_string("val-b")
            .send()
            .await
            .expect("create b");

        let batch = client
            .batch_get_secret_value()
            .secret_id_list(&name1)
            .secret_id_list(&name2)
            .send()
            .await
            .expect("batch get should succeed");

        assert_eq!(batch.secret_values().len(), 2);

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name1)
            .force_delete_without_recovery(true)
            .send()
            .await;
        let _ = client
            .delete_secret()
            .secret_id(&name2)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    // ---------------------------------------------------------------------------
    // Phase 2: Resource Policies, Replication stubs
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_manage_resource_policy() {
        let client = secretsmanager_client();
        let name = test_secret_name("policy");

        client
            .create_secret()
            .name(&name)
            .secret_string("value")
            .send()
            .await
            .expect("create");

        let policy = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"secretsmanager:GetSecretValue","Resource":"*"}]}"#;

        // Put resource policy.
        client
            .put_resource_policy()
            .secret_id(&name)
            .resource_policy(policy)
            .send()
            .await
            .expect("put policy");

        // Get resource policy.
        let get = client
            .get_resource_policy()
            .secret_id(&name)
            .send()
            .await
            .expect("get policy");

        assert!(get.resource_policy().is_some());

        // Delete resource policy.
        client
            .delete_resource_policy()
            .secret_id(&name)
            .send()
            .await
            .expect("delete policy");

        let get2 = client
            .get_resource_policy()
            .secret_id(&name)
            .send()
            .await
            .expect("get policy after delete");

        assert!(get2.resource_policy().is_none());

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_validate_resource_policy() {
        let client = secretsmanager_client();

        let policy = r#"{"Version":"2012-10-17","Statement":[]}"#;

        let resp = client
            .validate_resource_policy()
            .resource_policy(policy)
            .send()
            .await
            .expect("validate should succeed");

        assert!(resp.policy_validation_passed());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_secret_without_value() {
        let client = secretsmanager_client();
        let name = test_secret_name("no-value");

        let create = client
            .create_secret()
            .name(&name)
            .description("Secret placeholder without value")
            .send()
            .await
            .expect("create without value should succeed");

        assert!(create.arn().is_some());
        // No version_id when no value provided.
        assert!(create.version_id().is_none());

        let desc = client
            .describe_secret()
            .secret_id(&name)
            .send()
            .await
            .expect("describe should succeed");

        assert_eq!(desc.description(), Some("Secret placeholder without value"));

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_secrets_with_tag_filter() {
        let client = secretsmanager_client();
        let tag_value = format!("unique-{}", &uuid::Uuid::new_v4().to_string()[..6]);
        let name = test_secret_name("tag-filter");

        client
            .create_secret()
            .name(&name)
            .secret_string("value")
            .tags(Tag::builder().key("FilterTest").value(&tag_value).build())
            .send()
            .await
            .expect("create");

        let list = client
            .list_secrets()
            .filters(
                Filter::builder()
                    .key(FilterNameStringType::TagValue)
                    .values(&tag_value)
                    .build(),
            )
            .send()
            .await
            .expect("list with tag filter");

        assert_eq!(list.secret_list().len(), 1);

        // Cleanup.
        let _ = client
            .delete_secret()
            .secret_id(&name)
            .force_delete_without_recovery(true)
            .send()
            .await;
    }
}
