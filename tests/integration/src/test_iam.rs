//! Integration tests for the IAM service.
//!
//! These tests require a running RustStack server at `localhost:4566`.
//! They are marked `#[ignore = "requires running RustStack server"]` so they don't run during
//! normal `cargo test`.

#[allow(unused_imports)]
use crate::iam_client;

// ---------------------------------------------------------------------------
// Phase 0: Core CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_create_and_get_user() {
    let client = iam_client();

    let create = client
        .create_user()
        .user_name("test-user-1")
        .send()
        .await
        .expect("create user");

    let user = create.user().expect("user in response");
    assert_eq!(user.user_name(), "test-user-1");
    assert!(user.arn().contains("arn:aws:iam::"));
    assert!(user.user_id().starts_with("AIDA"));

    let get = client
        .get_user()
        .user_name("test-user-1")
        .send()
        .await
        .expect("get user");

    let user = get.user().expect("user in get response");
    assert_eq!(user.user_name(), "test-user-1");

    // Cleanup
    client
        .delete_user()
        .user_name("test-user-1")
        .send()
        .await
        .expect("delete user");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_list_users() {
    let client = iam_client();

    // Create a few users
    for i in 1..=3 {
        client
            .create_user()
            .user_name(format!("list-user-{i}"))
            .send()
            .await
            .expect("create user");
    }

    let list = client.list_users().send().await.expect("list users");
    let users = list.users();
    assert!(users.len() >= 3);

    // Cleanup
    for i in 1..=3 {
        client
            .delete_user()
            .user_name(format!("list-user-{i}"))
            .send()
            .await
            .expect("delete user");
    }
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_create_role_and_get_role() {
    let client = iam_client();
    let trust_policy = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"lambda.amazonaws.com"},"Action":"sts:AssumeRole"}]}"#;

    let create = client
        .create_role()
        .role_name("test-lambda-role")
        .assume_role_policy_document(trust_policy)
        .send()
        .await
        .expect("create role");

    let role = create.role().expect("role in response");
    assert_eq!(role.role_name(), "test-lambda-role");
    assert!(role.arn().contains("arn:aws:iam::"));
    assert!(role.role_id().starts_with("AROA"));

    let get = client
        .get_role()
        .role_name("test-lambda-role")
        .send()
        .await
        .expect("get role");

    let role = get.role().expect("role in get response");
    assert_eq!(role.role_name(), "test-lambda-role");

    // Cleanup
    client
        .delete_role()
        .role_name("test-lambda-role")
        .send()
        .await
        .expect("delete role");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_create_policy_and_attach_to_role() {
    let client = iam_client();
    let trust_policy = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"lambda.amazonaws.com"},"Action":"sts:AssumeRole"}]}"#;
    let policy_doc = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:*","Resource":"*"}]}"#;

    // Create role
    client
        .create_role()
        .role_name("attach-test-role")
        .assume_role_policy_document(trust_policy)
        .send()
        .await
        .expect("create role");

    // Create policy
    let create_policy = client
        .create_policy()
        .policy_name("attach-test-policy")
        .policy_document(policy_doc)
        .send()
        .await
        .expect("create policy");

    let policy = create_policy.policy().expect("policy in response");
    let policy_arn = policy.arn().expect("policy arn");
    assert!(policy.policy_id().expect("policy_id").starts_with("ANPA"));

    // Attach
    client
        .attach_role_policy()
        .role_name("attach-test-role")
        .policy_arn(policy_arn)
        .send()
        .await
        .expect("attach role policy");

    // List attached
    let attached = client
        .list_attached_role_policies()
        .role_name("attach-test-role")
        .send()
        .await
        .expect("list attached");

    let policies = attached.attached_policies();
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0].policy_arn().expect("arn"), policy_arn);

    // Detach and cleanup
    client
        .detach_role_policy()
        .role_name("attach-test-role")
        .policy_arn(policy_arn)
        .send()
        .await
        .expect("detach");

    client
        .delete_policy()
        .policy_arn(policy_arn)
        .send()
        .await
        .expect("delete policy");

    client
        .delete_role()
        .role_name("attach-test-role")
        .send()
        .await
        .expect("delete role");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_manage_access_keys() {
    let client = iam_client();

    client
        .create_user()
        .user_name("access-key-user")
        .send()
        .await
        .expect("create user");

    let create_key = client
        .create_access_key()
        .user_name("access-key-user")
        .send()
        .await
        .expect("create access key");

    let key = create_key.access_key().expect("access key");
    assert!(key.access_key_id().starts_with("AKIA"));
    assert!(!key.secret_access_key().is_empty());

    // List keys
    let list = client
        .list_access_keys()
        .user_name("access-key-user")
        .send()
        .await
        .expect("list access keys");

    assert_eq!(list.access_key_metadata().len(), 1);

    // Delete key
    client
        .delete_access_key()
        .user_name("access-key-user")
        .access_key_id(key.access_key_id())
        .send()
        .await
        .expect("delete access key");

    // Cleanup
    client
        .delete_user()
        .user_name("access-key-user")
        .send()
        .await
        .expect("delete user");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_enforce_delete_conflicts() {
    let client = iam_client();
    let trust_policy = r#"{"Version":"2012-10-17","Statement":[]}"#;
    let policy_doc = r#"{"Version":"2012-10-17","Statement":[]}"#;

    client
        .create_role()
        .role_name("conflict-role")
        .assume_role_policy_document(trust_policy)
        .send()
        .await
        .expect("create role");

    let create_policy = client
        .create_policy()
        .policy_name("conflict-policy")
        .policy_document(policy_doc)
        .send()
        .await
        .expect("create policy");

    let policy_arn = create_policy
        .policy()
        .expect("policy")
        .arn()
        .expect("arn")
        .to_string();

    client
        .attach_role_policy()
        .role_name("conflict-role")
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("attach");

    // Delete role should fail (has attached policy)
    let result = client.delete_role().role_name("conflict-role").send().await;
    assert!(
        result.is_err(),
        "delete role should fail with attached policy"
    );

    // Detach, then delete should succeed
    client
        .detach_role_policy()
        .role_name("conflict-role")
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("detach");

    client
        .delete_role()
        .role_name("conflict-role")
        .send()
        .await
        .expect("delete role after detach");

    client
        .delete_policy()
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("delete policy");
}

// ---------------------------------------------------------------------------
// Phase 1: Groups + Instance Profiles
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_manage_groups_and_membership() {
    let client = iam_client();

    client
        .create_group()
        .group_name("test-group")
        .send()
        .await
        .expect("create group");

    client
        .create_user()
        .user_name("group-user")
        .send()
        .await
        .expect("create user");

    client
        .add_user_to_group()
        .group_name("test-group")
        .user_name("group-user")
        .send()
        .await
        .expect("add user to group");

    // List groups for user
    let groups = client
        .list_groups_for_user()
        .user_name("group-user")
        .send()
        .await
        .expect("list groups for user");

    assert_eq!(groups.groups().len(), 1);
    assert_eq!(groups.groups()[0].group_name(), "test-group");

    // Get group (should include user)
    let get = client
        .get_group()
        .group_name("test-group")
        .send()
        .await
        .expect("get group");

    assert_eq!(get.users().len(), 1);

    // Remove and cleanup
    client
        .remove_user_from_group()
        .group_name("test-group")
        .user_name("group-user")
        .send()
        .await
        .expect("remove user from group");

    client
        .delete_group()
        .group_name("test-group")
        .send()
        .await
        .expect("delete group");

    client
        .delete_user()
        .user_name("group-user")
        .send()
        .await
        .expect("delete user");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_manage_instance_profiles() {
    let client = iam_client();
    let trust_policy = r#"{"Version":"2012-10-17","Statement":[]}"#;

    client
        .create_instance_profile()
        .instance_profile_name("test-profile")
        .send()
        .await
        .expect("create instance profile");

    client
        .create_role()
        .role_name("profile-role")
        .assume_role_policy_document(trust_policy)
        .send()
        .await
        .expect("create role");

    client
        .add_role_to_instance_profile()
        .instance_profile_name("test-profile")
        .role_name("profile-role")
        .send()
        .await
        .expect("add role to instance profile");

    let get = client
        .get_instance_profile()
        .instance_profile_name("test-profile")
        .send()
        .await
        .expect("get instance profile");

    let profile = get.instance_profile().expect("instance profile");
    assert_eq!(profile.roles().len(), 1);

    // Cleanup
    client
        .remove_role_from_instance_profile()
        .instance_profile_name("test-profile")
        .role_name("profile-role")
        .send()
        .await
        .expect("remove role from instance profile");

    client
        .delete_instance_profile()
        .instance_profile_name("test-profile")
        .send()
        .await
        .expect("delete instance profile");

    client
        .delete_role()
        .role_name("profile-role")
        .send()
        .await
        .expect("delete role");
}

// ---------------------------------------------------------------------------
// Phase 2: Policy Versions + Inline Policies
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_manage_policy_versions() {
    let client = iam_client();
    let policy_doc_v1 = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:GetObject","Resource":"*"}]}"#;
    let policy_doc_v2 = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:*","Resource":"*"}]}"#;

    let create = client
        .create_policy()
        .policy_name("version-policy")
        .policy_document(policy_doc_v1)
        .send()
        .await
        .expect("create policy");

    let policy_arn = create
        .policy()
        .expect("policy")
        .arn()
        .expect("arn")
        .to_string();

    // Create version v2
    let v2 = client
        .create_policy_version()
        .policy_arn(&policy_arn)
        .policy_document(policy_doc_v2)
        .set_as_default(true)
        .send()
        .await
        .expect("create policy version");

    let version = v2.policy_version().expect("version");
    assert!(version.is_default_version());

    // List versions
    let versions = client
        .list_policy_versions()
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("list versions");

    assert_eq!(versions.versions().len(), 2);

    // Get specific version
    let v1 = client
        .get_policy_version()
        .policy_arn(&policy_arn)
        .version_id("v1")
        .send()
        .await
        .expect("get version v1");

    assert!(!v1.policy_version().expect("v").is_default_version());

    // Delete non-default version
    client
        .delete_policy_version()
        .policy_arn(&policy_arn)
        .version_id("v1")
        .send()
        .await
        .expect("delete version v1");

    // Cleanup
    client
        .delete_policy()
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("delete policy");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_manage_inline_policies() {
    let client = iam_client();
    let trust_policy = r#"{"Version":"2012-10-17","Statement":[]}"#;
    let inline_doc = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"logs:*","Resource":"*"}]}"#;

    client
        .create_role()
        .role_name("inline-role")
        .assume_role_policy_document(trust_policy)
        .send()
        .await
        .expect("create role");

    // Put inline policy
    client
        .put_role_policy()
        .role_name("inline-role")
        .policy_name("InlineLogsPolicy")
        .policy_document(inline_doc)
        .send()
        .await
        .expect("put role policy");

    // Get inline policy
    let get = client
        .get_role_policy()
        .role_name("inline-role")
        .policy_name("InlineLogsPolicy")
        .send()
        .await
        .expect("get role policy");

    assert_eq!(get.policy_name(), "InlineLogsPolicy");

    // List inline policies
    let list = client
        .list_role_policies()
        .role_name("inline-role")
        .send()
        .await
        .expect("list role policies");

    assert_eq!(list.policy_names().len(), 1);

    // Delete inline policy
    client
        .delete_role_policy()
        .role_name("inline-role")
        .policy_name("InlineLogsPolicy")
        .send()
        .await
        .expect("delete role policy");

    // Cleanup
    client
        .delete_role()
        .role_name("inline-role")
        .send()
        .await
        .expect("delete role");
}

// ---------------------------------------------------------------------------
// Phase 3: Tags + Service-Linked Roles + Advanced
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_manage_tags() {
    let client = iam_client();
    let trust_policy = r#"{"Version":"2012-10-17","Statement":[]}"#;

    client
        .create_role()
        .role_name("tag-role")
        .assume_role_policy_document(trust_policy)
        .send()
        .await
        .expect("create role");

    // Tag role
    client
        .tag_role()
        .role_name("tag-role")
        .tags(
            aws_sdk_iam::types::Tag::builder()
                .key("Environment")
                .value("production")
                .build()
                .expect("tag"),
        )
        .tags(
            aws_sdk_iam::types::Tag::builder()
                .key("Team")
                .value("platform")
                .build()
                .expect("tag"),
        )
        .send()
        .await
        .expect("tag role");

    // List tags
    let tags = client
        .list_role_tags()
        .role_name("tag-role")
        .send()
        .await
        .expect("list role tags");

    assert_eq!(tags.tags().len(), 2);

    // Untag
    client
        .untag_role()
        .role_name("tag-role")
        .tag_keys("Team")
        .send()
        .await
        .expect("untag role");

    let tags_after = client
        .list_role_tags()
        .role_name("tag-role")
        .send()
        .await
        .expect("list tags after untag");

    assert_eq!(tags_after.tags().len(), 1);

    // Cleanup
    client
        .delete_role()
        .role_name("tag-role")
        .send()
        .await
        .expect("delete role");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_update_assume_role_policy() {
    let client = iam_client();
    let trust_policy = r#"{"Version":"2012-10-17","Statement":[]}"#;
    let new_trust = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}"#;

    client
        .create_role()
        .role_name("trust-role")
        .assume_role_policy_document(trust_policy)
        .send()
        .await
        .expect("create role");

    client
        .update_assume_role_policy()
        .role_name("trust-role")
        .policy_document(new_trust)
        .send()
        .await
        .expect("update assume role policy");

    // Cleanup
    client
        .delete_role()
        .role_name("trust-role")
        .send()
        .await
        .expect("delete role");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_create_service_linked_role() {
    let client = iam_client();

    let result = client
        .create_service_linked_role()
        .aws_service_name("elasticmapreduce.amazonaws.com")
        .send()
        .await
        .expect("create service linked role");

    let role = result.role().expect("role");
    assert!(role.role_name().contains("AWSServiceRoleFor"));

    // Delete service-linked role
    let delete = client
        .delete_service_linked_role()
        .role_name(role.role_name())
        .send()
        .await
        .expect("delete service linked role");

    assert!(!delete.deletion_task_id().is_empty());

    // Check deletion status
    let status = client
        .get_service_linked_role_deletion_status()
        .deletion_task_id(delete.deletion_task_id())
        .send()
        .await
        .expect("get deletion status");

    assert_eq!(status.status().as_str(), "SUCCEEDED");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_list_entities_for_policy() {
    let client = iam_client();
    let trust_policy = r#"{"Version":"2012-10-17","Statement":[]}"#;
    let policy_doc = r#"{"Version":"2012-10-17","Statement":[]}"#;

    // Create policy and role
    let create = client
        .create_policy()
        .policy_name("entity-list-policy")
        .policy_document(policy_doc)
        .send()
        .await
        .expect("create policy");

    let policy_arn = create.policy().expect("p").arn().expect("arn").to_string();

    client
        .create_role()
        .role_name("entity-list-role")
        .assume_role_policy_document(trust_policy)
        .send()
        .await
        .expect("create role");

    client
        .attach_role_policy()
        .role_name("entity-list-role")
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("attach");

    // List entities
    let entities = client
        .list_entities_for_policy()
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("list entities for policy");

    assert_eq!(entities.policy_roles().len(), 1);

    // Cleanup
    client
        .detach_role_policy()
        .role_name("entity-list-role")
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("detach");

    client
        .delete_role()
        .role_name("entity-list-role")
        .send()
        .await
        .expect("delete role");

    client
        .delete_policy()
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("delete policy");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_get_account_authorization_details() {
    let client = iam_client();
    let trust_policy = r#"{"Version":"2012-10-17","Statement":[]}"#;

    // Create some entities
    client
        .create_user()
        .user_name("auth-details-user")
        .send()
        .await
        .expect("create user");

    client
        .create_role()
        .role_name("auth-details-role")
        .assume_role_policy_document(trust_policy)
        .send()
        .await
        .expect("create role");

    let result = client
        .get_account_authorization_details()
        .send()
        .await
        .expect("get account authorization details");

    assert!(!result.user_detail_list().is_empty());
    assert!(!result.role_detail_list().is_empty());

    // Cleanup
    client
        .delete_user()
        .user_name("auth-details-user")
        .send()
        .await
        .expect("delete user");

    client
        .delete_role()
        .role_name("auth-details-role")
        .send()
        .await
        .expect("delete role");
}

// ---------------------------------------------------------------------------
// Phase 4: OIDC Providers + Policy Tags + Instance Profile Tags
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_create_and_get_oidc_provider() {
    let client = iam_client();

    let create = client
        .create_open_id_connect_provider()
        .url("https://token.example.com")
        .client_id_list("my-client-id")
        .thumbprint_list("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .send()
        .await
        .expect("create OIDC provider");

    let provider_arn = create
        .open_id_connect_provider_arn()
        .expect("provider ARN")
        .to_string();

    assert!(provider_arn.contains("oidc-provider/token.example.com"));

    // Get and verify
    let get = client
        .get_open_id_connect_provider()
        .open_id_connect_provider_arn(&provider_arn)
        .send()
        .await
        .expect("get OIDC provider");

    assert_eq!(get.url(), Some("https://token.example.com"));
    assert!(
        get.client_id_list().iter().any(|c| c == "my-client-id"),
        "client ID list should contain my-client-id"
    );
    assert!(
        get.thumbprint_list()
            .iter()
            .any(|t| t == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        "thumbprint list should contain the expected thumbprint"
    );

    // Cleanup
    client
        .delete_open_id_connect_provider()
        .open_id_connect_provider_arn(&provider_arn)
        .send()
        .await
        .expect("delete OIDC provider");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_list_oidc_providers() {
    let client = iam_client();

    let create = client
        .create_open_id_connect_provider()
        .url("https://list-test.example.com")
        .thumbprint_list("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        .send()
        .await
        .expect("create OIDC provider");

    let provider_arn = create
        .open_id_connect_provider_arn()
        .expect("provider ARN")
        .to_string();

    let list = client
        .list_open_id_connect_providers()
        .send()
        .await
        .expect("list OIDC providers");

    assert!(
        list.open_id_connect_provider_list()
            .iter()
            .any(|p| p.arn() == Some(provider_arn.as_str())),
        "provider should appear in the list"
    );

    // Cleanup
    client
        .delete_open_id_connect_provider()
        .open_id_connect_provider_arn(&provider_arn)
        .send()
        .await
        .expect("delete OIDC provider");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_delete_oidc_provider() {
    let client = iam_client();

    let create = client
        .create_open_id_connect_provider()
        .url("https://delete-test.example.com")
        .thumbprint_list("cccccccccccccccccccccccccccccccccccccccc")
        .send()
        .await
        .expect("create OIDC provider");

    let provider_arn = create
        .open_id_connect_provider_arn()
        .expect("provider ARN")
        .to_string();

    // Delete
    client
        .delete_open_id_connect_provider()
        .open_id_connect_provider_arn(&provider_arn)
        .send()
        .await
        .expect("delete OIDC provider");

    // Get should fail with NoSuchEntity
    let result = client
        .get_open_id_connect_provider()
        .open_id_connect_provider_arn(&provider_arn)
        .send()
        .await;

    assert!(result.is_err(), "get after delete should fail");
    let err = result.unwrap_err();
    let service_err = err.as_service_error().expect("should be a service error");
    assert!(
        service_err.is_no_such_entity_exception(),
        "error should be NoSuchEntity, got: {service_err:?}"
    );
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_tag_and_list_policy_tags() {
    let client = iam_client();
    let policy_doc = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:GetObject","Resource":"*"}]}"#;

    let create = client
        .create_policy()
        .policy_name("tag-test-policy")
        .policy_document(policy_doc)
        .send()
        .await
        .expect("create policy");

    let policy_arn = create
        .policy()
        .expect("policy")
        .arn()
        .expect("arn")
        .to_string();

    // Tag the policy
    client
        .tag_policy()
        .policy_arn(&policy_arn)
        .tags(
            aws_sdk_iam::types::Tag::builder()
                .key("CostCenter")
                .value("12345")
                .build()
                .expect("tag"),
        )
        .tags(
            aws_sdk_iam::types::Tag::builder()
                .key("Project")
                .value("alpha")
                .build()
                .expect("tag"),
        )
        .send()
        .await
        .expect("tag policy");

    // List tags
    let tags = client
        .list_policy_tags()
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("list policy tags");

    assert_eq!(tags.tags().len(), 2);
    assert!(
        tags.tags()
            .iter()
            .any(|t| t.key() == "CostCenter" && t.value() == "12345"),
        "should contain CostCenter tag"
    );
    assert!(
        tags.tags()
            .iter()
            .any(|t| t.key() == "Project" && t.value() == "alpha"),
        "should contain Project tag"
    );

    // Cleanup
    client
        .delete_policy()
        .policy_arn(&policy_arn)
        .send()
        .await
        .expect("delete policy");
}

#[tokio::test]
#[ignore = "requires running RustStack server"]
async fn test_should_tag_and_list_instance_profile_tags() {
    let client = iam_client();

    client
        .create_instance_profile()
        .instance_profile_name("tag-test-profile")
        .send()
        .await
        .expect("create instance profile");

    // Tag the instance profile
    client
        .tag_instance_profile()
        .instance_profile_name("tag-test-profile")
        .tags(
            aws_sdk_iam::types::Tag::builder()
                .key("Environment")
                .value("staging")
                .build()
                .expect("tag"),
        )
        .tags(
            aws_sdk_iam::types::Tag::builder()
                .key("Owner")
                .value("platform-team")
                .build()
                .expect("tag"),
        )
        .send()
        .await
        .expect("tag instance profile");

    // List tags
    let tags = client
        .list_instance_profile_tags()
        .instance_profile_name("tag-test-profile")
        .send()
        .await
        .expect("list instance profile tags");

    assert_eq!(tags.tags().len(), 2);
    assert!(
        tags.tags()
            .iter()
            .any(|t| t.key() == "Environment" && t.value() == "staging"),
        "should contain Environment tag"
    );
    assert!(
        tags.tags()
            .iter()
            .any(|t| t.key() == "Owner" && t.value() == "platform-team"),
        "should contain Owner tag"
    );

    // Cleanup
    client
        .delete_instance_profile()
        .instance_profile_name("tag-test-profile")
        .send()
        .await
        .expect("delete instance profile");
}
