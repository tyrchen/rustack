//! SES integration tests against a running `Rustack` server.
//!
//! These tests cover identity management, sending emails, templates,
//! configuration sets, receipt rules, notifications, DKIM, and identity policies.

#[cfg(test)]
mod tests {
    use aws_sdk_ses as ses;
    use aws_sdk_ses::types::{
        Body, Content, Destination, EventDestination, EventType, Message, NotificationType,
        RawMessage, ReceiptRule, Template,
    };

    use crate::ses_client;

    /// Generate a unique identifier for test resources.
    fn unique_id() -> String {
        uuid::Uuid::new_v4().to_string()[..8].to_owned()
    }

    // ---------------------------------------------------------------------------
    // Phase 0: Identity Management & Sending
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_verify_email_identity_and_list() {
        let client = ses_client();
        let email = format!("test-{}@example.com", unique_id());

        // Verify email identity.
        client
            .verify_email_identity()
            .email_address(&email)
            .send()
            .await
            .expect("verify_email_identity should succeed");

        // List identities should include the email.
        let list = client
            .list_identities()
            .send()
            .await
            .expect("list_identities should succeed");
        assert!(
            list.identities().contains(&email),
            "identities should contain {email}"
        );

        // Get verification attributes.
        let attrs = client
            .get_identity_verification_attributes()
            .identities(&email)
            .send()
            .await
            .expect("get_identity_verification_attributes should succeed");
        let entries = attrs.verification_attributes();
        assert!(
            entries.contains_key(&email),
            "verification attributes should contain {email}"
        );

        // Delete identity.
        client
            .delete_identity()
            .identity(&email)
            .send()
            .await
            .expect("delete_identity should succeed");

        // Verify it's gone.
        let list = client
            .list_identities()
            .send()
            .await
            .expect("list_identities should succeed");
        assert!(
            !list.identities().contains(&email),
            "identities should no longer contain {email}"
        );
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_verify_domain_identity() {
        let client = ses_client();
        let domain = format!("test-{}.example.com", unique_id());

        let result = client
            .verify_domain_identity()
            .domain(&domain)
            .send()
            .await
            .expect("verify_domain_identity should succeed");

        // Should return a verification token.
        assert!(
            !result.verification_token().is_empty(),
            "should return a non-empty verification token"
        );

        // Should appear in list.
        let list = client
            .list_identities()
            .send()
            .await
            .expect("list_identities should succeed");
        assert!(
            list.identities().contains(&domain),
            "identities should contain {domain}"
        );

        // Cleanup.
        let _ = client.delete_identity().identity(&domain).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_send_email() {
        let client = ses_client();
        let source = format!("sender-{}@example.com", unique_id());

        // Verify the sender identity first.
        client
            .verify_email_identity()
            .email_address(&source)
            .send()
            .await
            .expect("verify should succeed");

        let result = client
            .send_email()
            .source(&source)
            .destination(
                Destination::builder()
                    .to_addresses("recipient@example.com")
                    .build(),
            )
            .message(
                Message::builder()
                    .subject(Content::builder().data("Test Subject").build().unwrap())
                    .body(
                        Body::builder()
                            .text(Content::builder().data("Hello, world!").build().unwrap())
                            .build(),
                    )
                    .build(),
            )
            .send()
            .await
            .expect("send_email should succeed");

        assert!(
            !result.message_id().is_empty(),
            "message_id should not be empty"
        );

        // Cleanup.
        let _ = client.delete_identity().identity(&source).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_send_raw_email() {
        let client = ses_client();
        let source = format!("rawsender-{}@example.com", unique_id());

        // Verify the sender identity first.
        client
            .verify_email_identity()
            .email_address(&source)
            .send()
            .await
            .expect("verify should succeed");

        let raw_data = format!(
            "From: {source}\r\nTo: recipient@example.com\r\nSubject: Raw Test\r\n\r\nRaw body."
        );

        let result = client
            .send_raw_email()
            .source(&source)
            .raw_message(
                RawMessage::builder()
                    .data(aws_sdk_ses::primitives::Blob::new(raw_data.as_bytes()))
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("send_raw_email should succeed");

        assert!(
            !result.message_id().is_empty(),
            "message_id should not be empty"
        );

        // Cleanup.
        let _ = client.delete_identity().identity(&source).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_send_quota() {
        let client = ses_client();

        let quota = client
            .get_send_quota()
            .send()
            .await
            .expect("get_send_quota should succeed");

        // Quota values should be non-negative.
        assert!(quota.max24_hour_send() >= 0.0);
        assert!(quota.max_send_rate() >= 0.0);
        assert!(quota.sent_last24_hours() >= 0.0);
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_get_send_statistics() {
        let client = ses_client();

        let stats = client
            .get_send_statistics()
            .send()
            .await
            .expect("get_send_statistics should succeed");

        // Should return a (possibly empty) list of send data points.
        let _ = stats.send_data_points();
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_verify_and_list_verified_email_addresses() {
        let client = ses_client();
        let email = format!("verified-{}@example.com", unique_id());

        // Use the legacy VerifyEmailAddress API.
        client
            .verify_email_address()
            .email_address(&email)
            .send()
            .await
            .expect("verify_email_address should succeed");

        // List verified email addresses.
        let list = client
            .list_verified_email_addresses()
            .send()
            .await
            .expect("list_verified_email_addresses should succeed");
        assert!(
            list.verified_email_addresses().contains(&email),
            "verified emails should contain {email}"
        );

        // Delete verified email address.
        client
            .delete_verified_email_address()
            .email_address(&email)
            .send()
            .await
            .expect("delete_verified_email_address should succeed");

        // Verify it's gone.
        let list = client
            .list_verified_email_addresses()
            .send()
            .await
            .expect("list_verified_email_addresses should succeed");
        assert!(
            !list.verified_email_addresses().contains(&email),
            "verified emails should no longer contain {email}"
        );
    }

    // ---------------------------------------------------------------------------
    // Phase 1: Templates & Configuration Sets
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_get_update_delete_template() {
        let client = ses_client();
        let name = format!("tpl-{}", unique_id());

        // Create template.
        client
            .create_template()
            .template(
                Template::builder()
                    .template_name(&name)
                    .subject_part("Hello {{name}}")
                    .html_part("<h1>Hi {{name}}</h1>")
                    .text_part("Hi {{name}}")
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("create_template should succeed");

        // Get template.
        let got = client
            .get_template()
            .template_name(&name)
            .send()
            .await
            .expect("get_template should succeed");
        let tpl = got.template().expect("template should be present");
        assert_eq!(tpl.template_name(), name.as_str());
        assert_eq!(tpl.subject_part(), Some("Hello {{name}}"));

        // Update template.
        client
            .update_template()
            .template(
                Template::builder()
                    .template_name(&name)
                    .subject_part("Updated {{name}}")
                    .html_part("<h1>Updated {{name}}</h1>")
                    .text_part("Updated {{name}}")
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("update_template should succeed");

        // Verify update.
        let got = client
            .get_template()
            .template_name(&name)
            .send()
            .await
            .expect("get_template should succeed");
        let tpl = got.template().expect("template should be present");
        assert_eq!(tpl.subject_part(), Some("Updated {{name}}"));

        // Delete template.
        client
            .delete_template()
            .template_name(&name)
            .send()
            .await
            .expect("delete_template should succeed");

        // Verify deletion - get should fail.
        let err = client.get_template().template_name(&name).send().await;
        assert!(err.is_err(), "get_template should fail after deletion");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_list_templates() {
        let client = ses_client();
        let name_a = format!("tpl-list-a-{}", unique_id());
        let name_b = format!("tpl-list-b-{}", unique_id());

        // Create two templates.
        for name in [&name_a, &name_b] {
            client
                .create_template()
                .template(
                    Template::builder()
                        .template_name(name)
                        .subject_part("Subject")
                        .build()
                        .unwrap(),
                )
                .send()
                .await
                .expect("create_template should succeed");
        }

        let list = client
            .list_templates()
            .send()
            .await
            .expect("list_templates should succeed");

        let names: Vec<_> = list
            .templates_metadata()
            .iter()
            .filter_map(|m| m.name())
            .collect();
        assert!(names.contains(&name_a.as_str()), "should contain {name_a}");
        assert!(names.contains(&name_b.as_str()), "should contain {name_b}");

        // Cleanup.
        for name in [&name_a, &name_b] {
            let _ = client.delete_template().template_name(name).send().await;
        }
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_send_templated_email() {
        let client = ses_client();
        let source = format!("tplsender-{}@example.com", unique_id());
        let tpl_name = format!("tpl-send-{}", unique_id());

        // Verify sender.
        client
            .verify_email_identity()
            .email_address(&source)
            .send()
            .await
            .expect("verify should succeed");

        // Create template.
        client
            .create_template()
            .template(
                Template::builder()
                    .template_name(&tpl_name)
                    .subject_part("Hello {{name}}")
                    .text_part("Hi {{name}}")
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("create_template should succeed");

        // Send templated email.
        let result = client
            .send_templated_email()
            .source(&source)
            .destination(
                Destination::builder()
                    .to_addresses("recipient@example.com")
                    .build(),
            )
            .template(&tpl_name)
            .template_data(r#"{"name":"World"}"#)
            .send()
            .await
            .expect("send_templated_email should succeed");

        assert!(
            !result.message_id().is_empty(),
            "message_id should not be empty"
        );

        // Cleanup.
        let _ = client
            .delete_template()
            .template_name(&tpl_name)
            .send()
            .await;
        let _ = client.delete_identity().identity(&source).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_describe_list_delete_configuration_set() {
        let client = ses_client();
        let cs_name = format!("cs-{}", unique_id());

        // Create configuration set.
        client
            .create_configuration_set()
            .configuration_set(
                ses::types::ConfigurationSet::builder()
                    .name(&cs_name)
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("create_configuration_set should succeed");

        // Describe configuration set.
        let desc = client
            .describe_configuration_set()
            .configuration_set_name(&cs_name)
            .send()
            .await
            .expect("describe_configuration_set should succeed");
        let cs = desc.configuration_set().expect("should have config set");
        assert_eq!(cs.name(), cs_name.as_str());

        // List configuration sets.
        let list = client
            .list_configuration_sets()
            .send()
            .await
            .expect("list_configuration_sets should succeed");
        let names: Vec<_> = list
            .configuration_sets()
            .iter()
            .map(aws_sdk_ses::types::ConfigurationSet::name)
            .collect();
        assert!(
            names.contains(&cs_name.as_str()),
            "should contain {cs_name}"
        );

        // Delete configuration set.
        client
            .delete_configuration_set()
            .configuration_set_name(&cs_name)
            .send()
            .await
            .expect("delete_configuration_set should succeed");

        // Verify deletion.
        let list = client
            .list_configuration_sets()
            .send()
            .await
            .expect("list_configuration_sets should succeed");
        let names: Vec<_> = list
            .configuration_sets()
            .iter()
            .map(aws_sdk_ses::types::ConfigurationSet::name)
            .collect();
        assert!(
            !names.contains(&cs_name.as_str()),
            "should no longer contain {cs_name}"
        );
    }

    // ---------------------------------------------------------------------------
    // Phase 2: Event Destinations & Receipt Rules
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_delete_configuration_set_event_destination() {
        let client = ses_client();
        let cs_name = format!("cs-evt-{}", unique_id());
        let dest_name = format!("dest-{}", unique_id());

        // Create configuration set.
        client
            .create_configuration_set()
            .configuration_set(
                ses::types::ConfigurationSet::builder()
                    .name(&cs_name)
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("create_configuration_set should succeed");

        // Create event destination.
        client
            .create_configuration_set_event_destination()
            .configuration_set_name(&cs_name)
            .event_destination(
                EventDestination::builder()
                    .name(&dest_name)
                    .enabled(true)
                    .matching_event_types(EventType::Send)
                    .matching_event_types(EventType::Bounce)
                    .sns_destination(
                        ses::types::SnsDestination::builder()
                            .topic_arn("arn:aws:sns:us-east-1:000000000000:test-topic")
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("create event destination should succeed");

        // Describe to verify event destination exists.
        let desc = client
            .describe_configuration_set()
            .configuration_set_name(&cs_name)
            .send()
            .await
            .expect("describe should succeed");
        let dest_names: Vec<_> = desc
            .event_destinations()
            .iter()
            .map(aws_sdk_ses::types::EventDestination::name)
            .collect();
        assert!(
            dest_names.contains(&dest_name.as_str()),
            "should contain event destination {dest_name}"
        );

        // Delete event destination.
        client
            .delete_configuration_set_event_destination()
            .configuration_set_name(&cs_name)
            .event_destination_name(&dest_name)
            .send()
            .await
            .expect("delete event destination should succeed");

        // Cleanup.
        let _ = client
            .delete_configuration_set()
            .configuration_set_name(&cs_name)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_describe_delete_receipt_rule_set() {
        let client = ses_client();
        let rs_name = format!("rs-{}", unique_id());

        // Create receipt rule set.
        client
            .create_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await
            .expect("create_receipt_rule_set should succeed");

        // Describe receipt rule set.
        let desc = client
            .describe_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await
            .expect("describe_receipt_rule_set should succeed");
        let meta = desc.metadata().expect("should have metadata");
        assert_eq!(meta.name(), Some(rs_name.as_str()));

        // Delete receipt rule set.
        client
            .delete_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await
            .expect("delete_receipt_rule_set should succeed");

        // Verify deletion - describe should fail.
        let err = client
            .describe_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await;
        assert!(err.is_err(), "describe should fail after deleting rule set");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_clone_receipt_rule_set() {
        let client = ses_client();
        let original = format!("rs-orig-{}", unique_id());
        let cloned = format!("rs-clone-{}", unique_id());

        // Create original rule set.
        client
            .create_receipt_rule_set()
            .rule_set_name(&original)
            .send()
            .await
            .expect("create original should succeed");

        // Clone rule set.
        client
            .clone_receipt_rule_set()
            .original_rule_set_name(&original)
            .rule_set_name(&cloned)
            .send()
            .await
            .expect("clone_receipt_rule_set should succeed");

        // Describe cloned to verify.
        let desc = client
            .describe_receipt_rule_set()
            .rule_set_name(&cloned)
            .send()
            .await
            .expect("describe cloned should succeed");
        let meta = desc.metadata().expect("should have metadata");
        assert_eq!(meta.name(), Some(cloned.as_str()));

        // Cleanup.
        let _ = client
            .delete_receipt_rule_set()
            .rule_set_name(&original)
            .send()
            .await;
        let _ = client
            .delete_receipt_rule_set()
            .rule_set_name(&cloned)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_create_and_delete_receipt_rule() {
        let client = ses_client();
        let rs_name = format!("rs-rule-{}", unique_id());
        let rule_name = format!("rule-{}", unique_id());

        // Create rule set first.
        client
            .create_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await
            .expect("create rule set should succeed");

        // Create receipt rule.
        client
            .create_receipt_rule()
            .rule_set_name(&rs_name)
            .rule(
                ReceiptRule::builder()
                    .name(&rule_name)
                    .enabled(true)
                    .recipients("test@example.com")
                    .scan_enabled(false)
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("create_receipt_rule should succeed");

        // Describe rule set to verify rule exists.
        let desc = client
            .describe_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await
            .expect("describe should succeed");
        let rule_names: Vec<_> = desc
            .rules()
            .iter()
            .map(aws_sdk_ses::types::ReceiptRule::name)
            .collect();
        assert!(
            rule_names.contains(&rule_name.as_str()),
            "should contain rule {rule_name}"
        );

        // Delete receipt rule.
        client
            .delete_receipt_rule()
            .rule_set_name(&rs_name)
            .rule_name(&rule_name)
            .send()
            .await
            .expect("delete_receipt_rule should succeed");

        // Verify rule is gone.
        let desc = client
            .describe_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await
            .expect("describe should succeed");
        let rule_names: Vec<_> = desc
            .rules()
            .iter()
            .map(aws_sdk_ses::types::ReceiptRule::name)
            .collect();
        assert!(
            !rule_names.contains(&rule_name.as_str()),
            "should no longer contain rule {rule_name}"
        );

        // Cleanup.
        let _ = client
            .delete_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await;
    }

    // ---------------------------------------------------------------------------
    // Phase 3: Notifications, DKIM & Identity Policies
    // ---------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_set_and_get_identity_notification_topic() {
        let client = ses_client();
        let email = format!("notif-{}@example.com", unique_id());
        let topic_arn = "arn:aws:sns:us-east-1:000000000000:ses-notifications";

        // Verify identity first.
        client
            .verify_email_identity()
            .email_address(&email)
            .send()
            .await
            .expect("verify should succeed");

        // Set notification topic for bounces.
        client
            .set_identity_notification_topic()
            .identity(&email)
            .notification_type(NotificationType::Bounce)
            .sns_topic(topic_arn)
            .send()
            .await
            .expect("set_identity_notification_topic should succeed");

        // Get notification attributes.
        let attrs = client
            .get_identity_notification_attributes()
            .identities(&email)
            .send()
            .await
            .expect("get_identity_notification_attributes should succeed");

        let notif_attrs = attrs.notification_attributes();
        assert!(
            notif_attrs.contains_key(&email),
            "should contain notification attributes for {email}"
        );
        let entry = &notif_attrs[&email];
        assert_eq!(
            entry.bounce_topic(),
            topic_arn,
            "bounce topic should be set"
        );

        // Cleanup.
        let _ = client.delete_identity().identity(&email).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_verify_domain_dkim_and_get_attributes() {
        let client = ses_client();
        let domain = format!("dkim-{}.example.com", unique_id());

        // Verify domain identity first.
        client
            .verify_domain_identity()
            .domain(&domain)
            .send()
            .await
            .expect("verify_domain_identity should succeed");

        // Verify DKIM.
        let dkim = client
            .verify_domain_dkim()
            .domain(&domain)
            .send()
            .await
            .expect("verify_domain_dkim should succeed");

        // Should return DKIM tokens.
        assert!(!dkim.dkim_tokens().is_empty(), "should return DKIM tokens");

        // Get DKIM attributes.
        let attrs = client
            .get_identity_dkim_attributes()
            .identities(&domain)
            .send()
            .await
            .expect("get_identity_dkim_attributes should succeed");
        let dkim_attrs = attrs.dkim_attributes();
        assert!(
            dkim_attrs.contains_key(&domain),
            "should contain DKIM attributes for {domain}"
        );

        // Cleanup.
        let _ = client.delete_identity().identity(&domain).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_put_get_list_delete_identity_policy() {
        let client = ses_client();
        let email = format!("policy-{}@example.com", unique_id());
        let policy_name = format!("pol-{}", unique_id());
        let policy_doc = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"ses:SendEmail","Resource":"*"}]}"#;

        // Verify identity first.
        client
            .verify_email_identity()
            .email_address(&email)
            .send()
            .await
            .expect("verify should succeed");

        // Put identity policy.
        client
            .put_identity_policy()
            .identity(&email)
            .policy_name(&policy_name)
            .policy(policy_doc)
            .send()
            .await
            .expect("put_identity_policy should succeed");

        // List identity policies.
        let list = client
            .list_identity_policies()
            .identity(&email)
            .send()
            .await
            .expect("list_identity_policies should succeed");
        assert!(
            list.policy_names().contains(&policy_name),
            "should contain policy {policy_name}"
        );

        // Get identity policies.
        let got = client
            .get_identity_policies()
            .identity(&email)
            .policy_names(&policy_name)
            .send()
            .await
            .expect("get_identity_policies should succeed");
        let policies = got.policies();
        assert!(
            policies.contains_key(&policy_name),
            "should contain policy {policy_name}"
        );
        assert_eq!(
            policies.get(&policy_name).map(String::as_str),
            Some(policy_doc),
            "policy document should match"
        );

        // Delete identity policy.
        client
            .delete_identity_policy()
            .identity(&email)
            .policy_name(&policy_name)
            .send()
            .await
            .expect("delete_identity_policy should succeed");

        // Verify deletion.
        let list = client
            .list_identity_policies()
            .identity(&email)
            .send()
            .await
            .expect("list_identity_policies should succeed");
        assert!(
            !list.policy_names().contains(&policy_name),
            "should no longer contain policy {policy_name}"
        );

        // Cleanup.
        let _ = client.delete_identity().identity(&email).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_filter_list_identities_by_type() {
        let client = ses_client();
        let email = format!("filter-{}@example.com", unique_id());
        let domain = format!("filter-{}.example.com", unique_id());

        // Verify both an email and a domain.
        client
            .verify_email_identity()
            .email_address(&email)
            .send()
            .await
            .expect("verify email should succeed");
        client
            .verify_domain_identity()
            .domain(&domain)
            .send()
            .await
            .expect("verify domain should succeed");

        // List only email identities.
        let email_list = client
            .list_identities()
            .identity_type(ses::types::IdentityType::EmailAddress)
            .send()
            .await
            .expect("list email identities should succeed");
        assert!(
            email_list.identities().contains(&email),
            "email list should contain {email}"
        );
        assert!(
            !email_list.identities().contains(&domain),
            "email list should not contain domain {domain}"
        );

        // List only domain identities.
        let domain_list = client
            .list_identities()
            .identity_type(ses::types::IdentityType::Domain)
            .send()
            .await
            .expect("list domain identities should succeed");
        assert!(
            domain_list.identities().contains(&domain),
            "domain list should contain {domain}"
        );
        assert!(
            !domain_list.identities().contains(&email),
            "domain list should not contain email {email}"
        );

        // Cleanup.
        let _ = client.delete_identity().identity(&email).send().await;
        let _ = client.delete_identity().identity(&domain).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_send_email_with_html_body() {
        let client = ses_client();
        let source = format!("html-{}@example.com", unique_id());

        // Verify the sender identity.
        client
            .verify_email_identity()
            .email_address(&source)
            .send()
            .await
            .expect("verify should succeed");

        let result = client
            .send_email()
            .source(&source)
            .destination(
                Destination::builder()
                    .to_addresses("recipient@example.com")
                    .cc_addresses("cc@example.com")
                    .build(),
            )
            .message(
                Message::builder()
                    .subject(
                        Content::builder()
                            .data("HTML Test")
                            .charset("UTF-8")
                            .build()
                            .unwrap(),
                    )
                    .body(
                        Body::builder()
                            .html(
                                Content::builder()
                                    .data("<html><body><h1>Hello</h1></body></html>")
                                    .charset("UTF-8")
                                    .build()
                                    .unwrap(),
                            )
                            .text(Content::builder().data("Hello").build().unwrap())
                            .build(),
                    )
                    .build(),
            )
            .send()
            .await
            .expect("send_email with HTML should succeed");

        assert_ne!(result.message_id(), "");

        // Cleanup.
        let _ = client.delete_identity().identity(&source).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_handle_multiple_receipt_rules_in_set() {
        let client = ses_client();
        let rs_name = format!("rs-multi-{}", unique_id());
        let rule_a = format!("rule-a-{}", unique_id());
        let rule_b = format!("rule-b-{}", unique_id());

        // Create rule set.
        client
            .create_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await
            .expect("create rule set should succeed");

        // Create first rule.
        client
            .create_receipt_rule()
            .rule_set_name(&rs_name)
            .rule(
                ReceiptRule::builder()
                    .name(&rule_a)
                    .enabled(true)
                    .recipients("a@example.com")
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("create rule_a should succeed");

        // Create second rule.
        client
            .create_receipt_rule()
            .rule_set_name(&rs_name)
            .rule(
                ReceiptRule::builder()
                    .name(&rule_b)
                    .enabled(true)
                    .recipients("b@example.com")
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .expect("create rule_b should succeed");

        // Describe and verify both rules exist.
        let desc = client
            .describe_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await
            .expect("describe should succeed");
        let rule_names: Vec<_> = desc
            .rules()
            .iter()
            .map(aws_sdk_ses::types::ReceiptRule::name)
            .collect();
        assert!(
            rule_names.contains(&rule_a.as_str()),
            "should contain {rule_a}"
        );
        assert!(
            rule_names.contains(&rule_b.as_str()),
            "should contain {rule_b}"
        );

        // Cleanup.
        let _ = client
            .delete_receipt_rule()
            .rule_set_name(&rs_name)
            .rule_name(&rule_a)
            .send()
            .await;
        let _ = client
            .delete_receipt_rule()
            .rule_set_name(&rs_name)
            .rule_name(&rule_b)
            .send()
            .await;
        let _ = client
            .delete_receipt_rule_set()
            .rule_set_name(&rs_name)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_set_multiple_notification_types() {
        let client = ses_client();
        let email = format!("multi-notif-{}@example.com", unique_id());
        let bounce_topic = "arn:aws:sns:us-east-1:000000000000:bounce-topic";
        let complaint_topic = "arn:aws:sns:us-east-1:000000000000:complaint-topic";

        // Verify identity.
        client
            .verify_email_identity()
            .email_address(&email)
            .send()
            .await
            .expect("verify should succeed");

        // Set bounce notification.
        client
            .set_identity_notification_topic()
            .identity(&email)
            .notification_type(NotificationType::Bounce)
            .sns_topic(bounce_topic)
            .send()
            .await
            .expect("set bounce topic should succeed");

        // Set complaint notification.
        client
            .set_identity_notification_topic()
            .identity(&email)
            .notification_type(NotificationType::Complaint)
            .sns_topic(complaint_topic)
            .send()
            .await
            .expect("set complaint topic should succeed");

        // Get and verify both are set.
        let attrs = client
            .get_identity_notification_attributes()
            .identities(&email)
            .send()
            .await
            .expect("get notification attributes should succeed");
        let entry = &attrs.notification_attributes()[&email];
        assert_eq!(entry.bounce_topic(), bounce_topic);
        assert_eq!(entry.complaint_topic(), complaint_topic);

        // Cleanup.
        let _ = client.delete_identity().identity(&email).send().await;
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_should_handle_multiple_identity_policies() {
        let client = ses_client();
        let email = format!("multi-pol-{}@example.com", unique_id());
        let policy_a = format!("pol-a-{}", unique_id());
        let policy_b = format!("pol-b-{}", unique_id());
        let doc_a = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"ses:SendEmail","Resource":"*"}]}"#;
        let doc_b = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Deny","Principal":"*","Action":"ses:SendRawEmail","Resource":"*"}]}"#;

        // Verify identity.
        client
            .verify_email_identity()
            .email_address(&email)
            .send()
            .await
            .expect("verify should succeed");

        // Put two policies.
        client
            .put_identity_policy()
            .identity(&email)
            .policy_name(&policy_a)
            .policy(doc_a)
            .send()
            .await
            .expect("put policy_a should succeed");

        client
            .put_identity_policy()
            .identity(&email)
            .policy_name(&policy_b)
            .policy(doc_b)
            .send()
            .await
            .expect("put policy_b should succeed");

        // List should contain both.
        let list = client
            .list_identity_policies()
            .identity(&email)
            .send()
            .await
            .expect("list should succeed");
        assert!(list.policy_names().contains(&policy_a));
        assert!(list.policy_names().contains(&policy_b));

        // Delete one.
        client
            .delete_identity_policy()
            .identity(&email)
            .policy_name(&policy_a)
            .send()
            .await
            .expect("delete policy_a should succeed");

        // List should only contain policy_b.
        let list = client
            .list_identity_policies()
            .identity(&email)
            .send()
            .await
            .expect("list should succeed");
        assert!(!list.policy_names().contains(&policy_a));
        assert!(list.policy_names().contains(&policy_b));

        // Cleanup.
        let _ = client
            .delete_identity_policy()
            .identity(&email)
            .policy_name(&policy_b)
            .send()
            .await;
        let _ = client.delete_identity().identity(&email).send().await;
    }
}
