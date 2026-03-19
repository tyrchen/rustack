//! KMS integration tests.
//!
//! These tests require a running RustStack server at `localhost:4566`.

#[cfg(test)]
mod tests {
    use crate::kms_client;

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_describe_key() {
        let client = kms_client();
        let resp = client
            .create_key()
            .description("test key")
            .send()
            .await
            .unwrap();
        let metadata = resp.key_metadata().unwrap();
        assert!(!metadata.key_id().is_empty());
        assert!(metadata.arn().is_some());
        assert_eq!(metadata.description(), Some("test key"));
        assert_eq!(
            metadata.key_state(),
            Some(&aws_sdk_kms::types::KeyState::Enabled)
        );

        let key_id = metadata.key_id().to_string();
        let desc = client.describe_key().key_id(&key_id).send().await.unwrap();
        let dm = desc.key_metadata().unwrap();
        assert_eq!(dm.key_id(), key_id);
        assert_eq!(dm.description(), Some("test key"));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_keys() {
        let client = kms_client();
        client.create_key().send().await.unwrap();
        let resp = client.list_keys().send().await.unwrap();
        assert!(!resp.keys().is_empty());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_enable_disable_key() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        client.disable_key().key_id(&key_id).send().await.unwrap();
        let desc = client.describe_key().key_id(&key_id).send().await.unwrap();
        assert_eq!(
            desc.key_metadata().unwrap().key_state(),
            Some(&aws_sdk_kms::types::KeyState::Disabled)
        );

        client.enable_key().key_id(&key_id).send().await.unwrap();
        let desc = client.describe_key().key_id(&key_id).send().await.unwrap();
        assert_eq!(
            desc.key_metadata().unwrap().key_state(),
            Some(&aws_sdk_kms::types::KeyState::Enabled)
        );
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_schedule_and_cancel_key_deletion() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let sched = client
            .schedule_key_deletion()
            .key_id(&key_id)
            .pending_window_in_days(7)
            .send()
            .await
            .unwrap();
        assert!(sched.deletion_date().is_some());

        let desc = client.describe_key().key_id(&key_id).send().await.unwrap();
        assert_eq!(
            desc.key_metadata().unwrap().key_state(),
            Some(&aws_sdk_kms::types::KeyState::PendingDeletion)
        );

        client
            .cancel_key_deletion()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        let desc = client.describe_key().key_id(&key_id).send().await.unwrap();
        assert_eq!(
            desc.key_metadata().unwrap().key_state(),
            Some(&aws_sdk_kms::types::KeyState::Disabled)
        );
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_encrypt_decrypt_symmetric() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let plaintext = aws_sdk_kms::primitives::Blob::new(b"hello world".to_vec());
        let enc = client
            .encrypt()
            .key_id(&key_id)
            .plaintext(plaintext.clone())
            .send()
            .await
            .unwrap();
        let ct = enc.ciphertext_blob().unwrap();

        let dec = client
            .decrypt()
            .ciphertext_blob(ct.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(dec.plaintext().unwrap().as_ref(), b"hello world");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_encrypt_decrypt_with_encryption_context() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let plaintext = aws_sdk_kms::primitives::Blob::new(b"contextual data".to_vec());
        let enc = client
            .encrypt()
            .key_id(&key_id)
            .plaintext(plaintext)
            .encryption_context("purpose", "test")
            .send()
            .await
            .unwrap();
        let ct = enc.ciphertext_blob().unwrap();

        // Decrypt with correct context.
        let dec = client
            .decrypt()
            .ciphertext_blob(ct.clone())
            .encryption_context("purpose", "test")
            .send()
            .await
            .unwrap();
        assert_eq!(dec.plaintext().unwrap().as_ref(), b"contextual data");

        // Decrypt without context should fail.
        let result = client.decrypt().ciphertext_blob(ct.clone()).send().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_generate_data_key() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let dk = client
            .generate_data_key()
            .key_id(&key_id)
            .key_spec(aws_sdk_kms::types::DataKeySpec::Aes256)
            .send()
            .await
            .unwrap();

        assert!(dk.plaintext().is_some());
        assert!(dk.ciphertext_blob().is_some());
        assert_eq!(dk.plaintext().unwrap().as_ref().len(), 32);
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_generate_data_key_without_plaintext() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let dk = client
            .generate_data_key_without_plaintext()
            .key_id(&key_id)
            .key_spec(aws_sdk_kms::types::DataKeySpec::Aes256)
            .send()
            .await
            .unwrap();

        assert!(dk.ciphertext_blob().is_some());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_re_encrypt() {
        let client = kms_client();
        let resp1 = client.create_key().send().await.unwrap();
        let key1 = resp1.key_metadata().unwrap().key_id().to_string();
        let resp2 = client.create_key().send().await.unwrap();
        let key2 = resp2.key_metadata().unwrap().key_id().to_string();

        let plaintext = aws_sdk_kms::primitives::Blob::new(b"re-encrypt me".to_vec());
        let enc = client
            .encrypt()
            .key_id(&key1)
            .plaintext(plaintext)
            .send()
            .await
            .unwrap();
        let ct = enc.ciphertext_blob().unwrap();

        let re = client
            .re_encrypt()
            .ciphertext_blob(ct.clone())
            .destination_key_id(&key2)
            .send()
            .await
            .unwrap();
        assert!(re.ciphertext_blob().is_some());

        // Decrypt the re-encrypted ciphertext with key2.
        let dec = client
            .decrypt()
            .ciphertext_blob(re.ciphertext_blob().unwrap().clone())
            .key_id(&key2)
            .send()
            .await
            .unwrap();
        assert_eq!(dec.plaintext().unwrap().as_ref(), b"re-encrypt me");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_list_aliases() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let alias_name = format!("alias/test-{}", uuid::Uuid::new_v4());
        client
            .create_alias()
            .alias_name(&alias_name)
            .target_key_id(&key_id)
            .send()
            .await
            .unwrap();

        let list = client.list_aliases().send().await.unwrap();
        let found = list
            .aliases()
            .iter()
            .any(|a| a.alias_name() == Some(alias_name.as_str()));
        assert!(found);
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_encrypt_via_alias() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let alias_name = format!("alias/enc-test-{}", uuid::Uuid::new_v4());
        client
            .create_alias()
            .alias_name(&alias_name)
            .target_key_id(&key_id)
            .send()
            .await
            .unwrap();

        let plaintext = aws_sdk_kms::primitives::Blob::new(b"via alias".to_vec());
        let enc = client
            .encrypt()
            .key_id(&alias_name)
            .plaintext(plaintext)
            .send()
            .await
            .unwrap();
        assert!(enc.ciphertext_blob().is_some());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_and_delete_alias() {
        let client = kms_client();
        let resp1 = client.create_key().send().await.unwrap();
        let key1 = resp1.key_metadata().unwrap().key_id().to_string();
        let resp2 = client.create_key().send().await.unwrap();
        let key2 = resp2.key_metadata().unwrap().key_id().to_string();

        let alias_name = format!("alias/upd-{}", uuid::Uuid::new_v4());
        client
            .create_alias()
            .alias_name(&alias_name)
            .target_key_id(&key1)
            .send()
            .await
            .unwrap();

        client
            .update_alias()
            .alias_name(&alias_name)
            .target_key_id(&key2)
            .send()
            .await
            .unwrap();

        let list = client.list_aliases().key_id(&key2).send().await.unwrap();
        let found = list
            .aliases()
            .iter()
            .any(|a| a.alias_name() == Some(alias_name.as_str()));
        assert!(found);

        client
            .delete_alias()
            .alias_name(&alias_name)
            .send()
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_tag_and_untag_key() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let tag = aws_sdk_kms::types::Tag::builder()
            .tag_key("env")
            .tag_value("test")
            .build()
            .unwrap();
        client
            .tag_resource()
            .key_id(&key_id)
            .tags(tag)
            .send()
            .await
            .unwrap();

        let tags = client
            .list_resource_tags()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert!(tags.tags().iter().any(|t| t.tag_key() == "env"));

        client
            .untag_resource()
            .key_id(&key_id)
            .tag_keys("env")
            .send()
            .await
            .unwrap();

        let tags = client
            .list_resource_tags()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert!(!tags.tags().iter().any(|t| t.tag_key() == "env"));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_resource_tags() {
        let client = kms_client();
        let tag = aws_sdk_kms::types::Tag::builder()
            .tag_key("project")
            .tag_value("ruststack")
            .build()
            .unwrap();
        let resp = client.create_key().tags(tag).send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let tags = client
            .list_resource_tags()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert_eq!(tags.tags().len(), 1);
        assert_eq!(tags.tags()[0].tag_key(), "project");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_and_put_key_policy() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let get = client
            .get_key_policy()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert!(get.policy().is_some());

        let new_policy = r#"{"Version":"2012-10-17","Statement":[]}"#;
        client
            .put_key_policy()
            .key_id(&key_id)
            .policy(new_policy)
            .send()
            .await
            .unwrap();

        let get = client
            .get_key_policy()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert_eq!(get.policy(), Some(new_policy));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_key_policies() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let list = client
            .list_key_policies()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert!(list.policy_names().contains(&"default".to_string()));
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_sign_and_verify_rsa() {
        let client = kms_client();
        let resp = client
            .create_key()
            .key_spec(aws_sdk_kms::types::KeySpec::Rsa2048)
            .key_usage(aws_sdk_kms::types::KeyUsageType::SignVerify)
            .send()
            .await
            .unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let message = aws_sdk_kms::primitives::Blob::new(b"sign this message".to_vec());
        let sign_resp = client
            .sign()
            .key_id(&key_id)
            .message(message.clone())
            .signing_algorithm(aws_sdk_kms::types::SigningAlgorithmSpec::RsassaPkcs1V15Sha256)
            .message_type(aws_sdk_kms::types::MessageType::Raw)
            .send()
            .await
            .unwrap();
        let sig = sign_resp.signature().unwrap();

        let verify_resp = client
            .verify()
            .key_id(&key_id)
            .message(message)
            .signature(sig.clone())
            .signing_algorithm(aws_sdk_kms::types::SigningAlgorithmSpec::RsassaPkcs1V15Sha256)
            .message_type(aws_sdk_kms::types::MessageType::Raw)
            .send()
            .await
            .unwrap();
        assert!(verify_resp.signature_valid());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_sign_and_verify_ecdsa() {
        let client = kms_client();
        let resp = client
            .create_key()
            .key_spec(aws_sdk_kms::types::KeySpec::EccNistP256)
            .key_usage(aws_sdk_kms::types::KeyUsageType::SignVerify)
            .send()
            .await
            .unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let message = aws_sdk_kms::primitives::Blob::new(b"ecdsa test message".to_vec());
        let sign_resp = client
            .sign()
            .key_id(&key_id)
            .message(message.clone())
            .signing_algorithm(aws_sdk_kms::types::SigningAlgorithmSpec::EcdsaSha256)
            .message_type(aws_sdk_kms::types::MessageType::Raw)
            .send()
            .await
            .unwrap();
        let sig = sign_resp.signature().unwrap();

        let verify_resp = client
            .verify()
            .key_id(&key_id)
            .message(message)
            .signature(sig.clone())
            .signing_algorithm(aws_sdk_kms::types::SigningAlgorithmSpec::EcdsaSha256)
            .message_type(aws_sdk_kms::types::MessageType::Raw)
            .send()
            .await
            .unwrap();
        assert!(verify_resp.signature_valid());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_generate_and_verify_mac() {
        let client = kms_client();
        let resp = client
            .create_key()
            .key_spec(aws_sdk_kms::types::KeySpec::Hmac256)
            .key_usage(aws_sdk_kms::types::KeyUsageType::GenerateVerifyMac)
            .send()
            .await
            .unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let message = aws_sdk_kms::primitives::Blob::new(b"hmac message".to_vec());
        let mac_resp = client
            .generate_mac()
            .key_id(&key_id)
            .message(message.clone())
            .mac_algorithm(aws_sdk_kms::types::MacAlgorithmSpec::HmacSha256)
            .send()
            .await
            .unwrap();
        let mac_tag = mac_resp.mac().unwrap();

        let verify_resp = client
            .verify_mac()
            .key_id(&key_id)
            .message(message)
            .mac(mac_tag.clone())
            .mac_algorithm(aws_sdk_kms::types::MacAlgorithmSpec::HmacSha256)
            .send()
            .await
            .unwrap();
        assert!(verify_resp.mac_valid());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_public_key() {
        let client = kms_client();
        let resp = client
            .create_key()
            .key_spec(aws_sdk_kms::types::KeySpec::Rsa2048)
            .key_usage(aws_sdk_kms::types::KeyUsageType::SignVerify)
            .send()
            .await
            .unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let pk = client
            .get_public_key()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert!(pk.public_key().is_some());
        assert!(!pk.public_key().unwrap().as_ref().is_empty());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_generate_random() {
        let client = kms_client();
        let resp = client
            .generate_random()
            .number_of_bytes(64)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.plaintext().unwrap().as_ref().len(), 64);
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_generate_data_key_pair() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let dkp = client
            .generate_data_key_pair()
            .key_id(&key_id)
            .key_pair_spec(aws_sdk_kms::types::DataKeyPairSpec::EccNistP256)
            .send()
            .await
            .unwrap();
        assert!(dkp.private_key_plaintext().is_some());
        assert!(dkp.private_key_ciphertext_blob().is_some());
        assert!(dkp.public_key().is_some());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_list_grants() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let grant = client
            .create_grant()
            .key_id(&key_id)
            .grantee_principal("arn:aws:iam::000000000000:user/test")
            .operations(aws_sdk_kms::types::GrantOperation::Encrypt)
            .operations(aws_sdk_kms::types::GrantOperation::Decrypt)
            .send()
            .await
            .unwrap();
        assert!(grant.grant_id().is_some());

        let list = client.list_grants().key_id(&key_id).send().await.unwrap();
        assert!(!list.grants().is_empty());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_revoke_grant() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let grant = client
            .create_grant()
            .key_id(&key_id)
            .grantee_principal("arn:aws:iam::000000000000:user/test")
            .operations(aws_sdk_kms::types::GrantOperation::Encrypt)
            .send()
            .await
            .unwrap();
        let grant_id = grant.grant_id().unwrap().to_string();

        client
            .revoke_grant()
            .key_id(&key_id)
            .grant_id(&grant_id)
            .send()
            .await
            .unwrap();

        let list = client.list_grants().key_id(&key_id).send().await.unwrap();
        assert!(
            !list
                .grants()
                .iter()
                .any(|g| g.grant_id() == Some(grant_id.as_str()))
        );
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_enable_disable_key_rotation() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        client
            .enable_key_rotation()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();

        let status = client
            .get_key_rotation_status()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert!(status.key_rotation_enabled());

        client
            .disable_key_rotation()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();

        let status = client
            .get_key_rotation_status()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert!(!status.key_rotation_enabled());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_key_rotation_status() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let status = client
            .get_key_rotation_status()
            .key_id(&key_id)
            .send()
            .await
            .unwrap();
        assert!(!status.key_rotation_enabled());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_fail_encrypt_on_disabled_key() {
        let client = kms_client();
        let resp = client.create_key().send().await.unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        client.disable_key().key_id(&key_id).send().await.unwrap();

        let plaintext = aws_sdk_kms::primitives::Blob::new(b"should fail".to_vec());
        let result = client
            .encrypt()
            .key_id(&key_id)
            .plaintext(plaintext)
            .send()
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_fail_encrypt_on_wrong_key_usage() {
        let client = kms_client();
        let resp = client
            .create_key()
            .key_spec(aws_sdk_kms::types::KeySpec::Rsa2048)
            .key_usage(aws_sdk_kms::types::KeyUsageType::SignVerify)
            .send()
            .await
            .unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        let plaintext = aws_sdk_kms::primitives::Blob::new(b"should fail".to_vec());
        let result = client
            .encrypt()
            .key_id(&key_id)
            .plaintext(plaintext)
            .send()
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_update_key_description() {
        let client = kms_client();
        let resp = client
            .create_key()
            .description("original")
            .send()
            .await
            .unwrap();
        let key_id = resp.key_metadata().unwrap().key_id().to_string();

        client
            .update_key_description()
            .key_id(&key_id)
            .description("updated")
            .send()
            .await
            .unwrap();

        let desc = client.describe_key().key_id(&key_id).send().await.unwrap();
        assert_eq!(desc.key_metadata().unwrap().description(), Some("updated"));
    }
} // mod tests
