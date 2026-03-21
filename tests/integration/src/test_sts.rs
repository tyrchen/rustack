//! STS integration tests against a running `RustStack` server.
//!
//! These tests cover all 8 STS operations: GetCallerIdentity, AssumeRole,
//! GetSessionToken, GetAccessKeyInfo, AssumeRoleWithSAML,
//! AssumeRoleWithWebIdentity, DecodeAuthorizationMessage, GetFederationToken.

#[cfg(test)]
mod tests {
    use crate::{sts_client, sts_client_with_credentials};

    // ---------------------------------------------------------------------------
    // Phase 0: Core Operations
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_caller_identity_root() {
        let client = sts_client();
        let result = client.get_caller_identity().send().await.unwrap();
        assert_eq!(result.account(), Some("000000000000"));
        assert_eq!(result.arn(), Some("arn:aws:iam::000000000000:root"));
        assert_eq!(result.user_id(), Some("000000000000"));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_assume_role_and_get_credentials() {
        let client = sts_client();
        let result = client
            .assume_role()
            .role_arn("arn:aws:iam::123456789012:role/TestRole")
            .role_session_name("test-session")
            .send()
            .await
            .unwrap();

        let creds = result.credentials().unwrap();
        assert!(creds.access_key_id().starts_with("ASIA"));
        assert!(!creds.secret_access_key().is_empty());
        assert!(!creds.session_token().is_empty());

        let assumed = result.assumed_role_user().unwrap();
        assert!(assumed.arn().contains("assumed-role/TestRole/test-session"));
        assert!(assumed.assumed_role_id().contains(":test-session"));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_caller_identity_after_assume_role() {
        let client = sts_client();

        // Assume a role.
        let assume_result = client
            .assume_role()
            .role_arn("arn:aws:iam::123456789012:role/TestRole")
            .role_session_name("identity-check")
            .send()
            .await
            .unwrap();

        let creds = assume_result.credentials().unwrap();

        // Create a new STS client with the temporary credentials.
        let temp_client = sts_client_with_credentials(
            creds.access_key_id(),
            creds.secret_access_key(),
            creds.session_token(),
        );

        // GetCallerIdentity should return the assumed role identity.
        let identity = temp_client.get_caller_identity().send().await.unwrap();
        assert_eq!(identity.account(), Some("123456789012"));
        assert!(
            identity
                .arn()
                .unwrap()
                .contains("assumed-role/TestRole/identity-check")
        );
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_session_token() {
        let client = sts_client();
        let result = client.get_session_token().send().await.unwrap();

        let creds = result.credentials().unwrap();
        assert!(creds.access_key_id().starts_with("ASIA"));
        assert!(!creds.secret_access_key().is_empty());
        assert!(!creds.session_token().is_empty());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_access_key_info() {
        let client = sts_client();
        let result = client
            .get_access_key_info()
            .access_key_id("AKIAIOSFODNN7EXAMPLE")
            .send()
            .await
            .unwrap();

        assert!(result.account().is_some());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_reject_invalid_role_arn() {
        let client = sts_client();
        let result = client
            .assume_role()
            .role_arn("not-a-valid-arn")
            .role_session_name("test")
            .send()
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_reject_short_session_name() {
        let client = sts_client();
        let result = client
            .assume_role()
            .role_arn("arn:aws:iam::123456789012:role/TestRole")
            .role_session_name("x") // Too short (min 2)
            .send()
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_assume_role_with_tags() {
        let client = sts_client();
        let result = client
            .assume_role()
            .role_arn("arn:aws:iam::123456789012:role/TaggedRole")
            .role_session_name("tagged-session")
            .tags(
                aws_sdk_sts::types::Tag::builder()
                    .key("Project")
                    .value("MyProject")
                    .build()
                    .unwrap(),
            )
            .tags(
                aws_sdk_sts::types::Tag::builder()
                    .key("Environment")
                    .value("Dev")
                    .build()
                    .unwrap(),
            )
            .transitive_tag_keys("Project")
            .send()
            .await
            .unwrap();

        let creds = result.credentials().unwrap();
        assert!(creds.access_key_id().starts_with("ASIA"));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_propagate_transitive_tags() {
        let client = sts_client();

        // First AssumeRole with transitive tags.
        let result1 = client
            .assume_role()
            .role_arn("arn:aws:iam::123456789012:role/RoleA")
            .role_session_name("session-a")
            .tags(
                aws_sdk_sts::types::Tag::builder()
                    .key("Project")
                    .value("MyProject")
                    .build()
                    .unwrap(),
            )
            .transitive_tag_keys("Project")
            .send()
            .await
            .unwrap();

        let creds1 = result1.credentials().unwrap();

        // Second AssumeRole using first role's credentials.
        let temp_client = sts_client_with_credentials(
            creds1.access_key_id(),
            creds1.secret_access_key(),
            creds1.session_token(),
        );

        let result2 = temp_client
            .assume_role()
            .role_arn("arn:aws:iam::123456789012:role/RoleB")
            .role_session_name("session-b")
            .send()
            .await
            .unwrap();

        // Verify the chained session exists.
        assert!(result2.credentials().is_some());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_assume_role_with_custom_duration() {
        let client = sts_client();
        let result = client
            .assume_role()
            .role_arn("arn:aws:iam::123456789012:role/TestRole")
            .role_session_name("duration-test")
            .duration_seconds(7200)
            .send()
            .await
            .unwrap();

        assert!(result.credentials().is_some());
    }

    // ---------------------------------------------------------------------------
    // Phase 1: Federation and Advanced Operations
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_assume_role_with_web_identity() {
        let client = sts_client();

        // Create a minimal JWT (not cryptographically valid, but STS accepts it).
        let header = base64_url_encode(b"{\"alg\":\"RS256\",\"typ\":\"JWT\"}");
        let payload = base64_url_encode(
            b"{\"sub\":\"user123\",\"aud\":\"my-app\",\"iss\":\"https://idp.example.com\"}",
        );
        let token = format!("{header}.{payload}.fake-signature");

        let result = client
            .assume_role_with_web_identity()
            .role_arn("arn:aws:iam::123456789012:role/WebIdentityRole")
            .role_session_name("web-session")
            .web_identity_token(&token)
            .send()
            .await
            .unwrap();

        let creds = result.credentials().unwrap();
        assert!(creds.access_key_id().starts_with("ASIA"));
        assert!(result.assumed_role_user().is_some());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_assume_role_with_saml() {
        let client = sts_client();

        // A minimal base64-encoded SAML assertion (not cryptographically valid).
        let saml_assertion = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "<SAMLResponse><Assertion><Subject><NameID>user@example.com</NameID></Subject></Assertion></SAMLResponse>",
        );

        let result = client
            .assume_role_with_saml()
            .role_arn("arn:aws:iam::123456789012:role/SAMLRole")
            .principal_arn("arn:aws:iam::123456789012:saml-provider/MyProvider")
            .saml_assertion(&saml_assertion)
            .send()
            .await
            .unwrap();

        let creds = result.credentials().unwrap();
        assert!(creds.access_key_id().starts_with("ASIA"));
        assert!(result.assumed_role_user().is_some());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_decode_authorization_message() {
        let client = sts_client();
        let result = client
            .decode_authorization_message()
            .encoded_message("some-encoded-message")
            .send()
            .await
            .unwrap();

        let decoded = result.decoded_message().unwrap();
        assert!(decoded.contains("allowed"));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_federation_token() {
        let client = sts_client();
        let result = client
            .get_federation_token()
            .name("bob")
            .send()
            .await
            .unwrap();

        let creds = result.credentials().unwrap();
        assert!(creds.access_key_id().starts_with("ASIA"));

        let fed_user = result.federated_user().unwrap();
        assert!(fed_user.arn().contains("federated-user/bob"));
        assert!(fed_user.federated_user_id().contains("bob"));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_reject_invalid_federated_name() {
        let client = sts_client();
        let result = client
            .get_federation_token()
            .name("x") // Too short (min 2)
            .send()
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_access_key_info_for_assumed_role_key() {
        let client = sts_client();

        // First assume a role to get an ASIA key.
        let assume_result = client
            .assume_role()
            .role_arn("arn:aws:iam::123456789012:role/TestRole")
            .role_session_name("key-info-test")
            .send()
            .await
            .unwrap();

        let creds = assume_result.credentials().unwrap();

        // GetAccessKeyInfo on the temporary key.
        let info = client
            .get_access_key_info()
            .access_key_id(creds.access_key_id())
            .send()
            .await
            .unwrap();

        assert_eq!(info.account(), Some("123456789012"));
    }

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    use base64::Engine;

    fn base64_url_encode(data: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
    }
}
