//! Main SES provider implementing all operations across all 4 phases.
//!
//! Acts as the central coordinator that owns all stores (identity, email,
//! template, configuration set, receipt rule, statistics) and implements
//! each SES operation as a method.
//!
//! Operation methods take input structs by value (matching the SNS provider
//! pattern) since callers always construct inputs immediately before calling.

use std::sync::Arc;

use rustack_ses_model::{
    error::{SesError, SesErrorCode},
    input::{
        CloneReceiptRuleSetInput, CreateConfigurationSetEventDestinationInput,
        CreateConfigurationSetInput, CreateReceiptRuleInput, CreateReceiptRuleSetInput,
        CreateTemplateInput, DeleteConfigurationSetEventDestinationInput,
        DeleteConfigurationSetInput, DeleteIdentityInput, DeleteIdentityPolicyInput,
        DeleteReceiptRuleInput, DeleteReceiptRuleSetInput, DeleteTemplateInput,
        DeleteVerifiedEmailAddressInput, DescribeActiveReceiptRuleSetInput,
        DescribeConfigurationSetInput, DescribeReceiptRuleSetInput, GetIdentityDkimAttributesInput,
        GetIdentityMailFromDomainAttributesInput, GetIdentityNotificationAttributesInput,
        GetIdentityPoliciesInput, GetIdentityVerificationAttributesInput, GetTemplateInput,
        ListConfigurationSetsInput, ListIdentitiesInput, ListIdentityPoliciesInput,
        ListTemplatesInput, PutIdentityPolicyInput, SendEmailInput, SendRawEmailInput,
        SendTemplatedEmailInput, SetActiveReceiptRuleSetInput,
        SetIdentityFeedbackForwardingEnabledInput, SetIdentityMailFromDomainInput,
        SetIdentityNotificationTopicInput, UpdateConfigurationSetEventDestinationInput,
        UpdateTemplateInput, VerifyDomainDkimInput, VerifyDomainIdentityInput,
        VerifyEmailAddressInput, VerifyEmailIdentityInput,
    },
    output::{
        CloneReceiptRuleSetResponse, CreateConfigurationSetEventDestinationResponse,
        CreateConfigurationSetResponse, CreateReceiptRuleResponse, CreateReceiptRuleSetResponse,
        CreateTemplateResponse, DeleteConfigurationSetEventDestinationResponse,
        DeleteConfigurationSetResponse, DeleteIdentityPolicyResponse, DeleteIdentityResponse,
        DeleteReceiptRuleResponse, DeleteReceiptRuleSetResponse, DeleteTemplateResponse,
        DescribeActiveReceiptRuleSetResponse, DescribeConfigurationSetResponse,
        DescribeReceiptRuleSetResponse, GetIdentityDkimAttributesResponse,
        GetIdentityMailFromDomainAttributesResponse, GetIdentityNotificationAttributesResponse,
        GetIdentityPoliciesResponse, GetIdentityVerificationAttributesResponse,
        GetSendQuotaResponse, GetSendStatisticsResponse, GetTemplateResponse,
        ListConfigurationSetsResponse, ListIdentitiesResponse, ListIdentityPoliciesResponse,
        ListTemplatesResponse, ListVerifiedEmailAddressesResponse, PutIdentityPolicyResponse,
        SendEmailResponse, SendRawEmailResponse, SendTemplatedEmailResponse,
        SetActiveReceiptRuleSetResponse, SetIdentityFeedbackForwardingEnabledResponse,
        SetIdentityMailFromDomainResponse, SetIdentityNotificationTopicResponse,
        UpdateConfigurationSetEventDestinationResponse, UpdateTemplateResponse,
        VerifyDomainDkimResponse, VerifyDomainIdentityResponse, VerifyEmailIdentityResponse,
    },
    types::{ConfigurationSet, IdentityType, MessageTag, ReceiptRuleSetMetadata, SendDataPoint},
};
use tracing::debug;

use crate::{
    config::SesConfig,
    config_set::ConfigurationSetStore,
    identity::IdentityStore,
    receipt_rule::ReceiptRuleSetStore,
    retrospection::{EmailStore, SentEmail, SentEmailBody, SentEmailDestination, SentEmailTag},
    statistics::SendStatistics,
    template::{TemplateStore, render_template},
    validation::validate_tags,
};

/// Validate a slice of `MessageTag` values.
///
/// # Errors
///
/// Returns the first validation error encountered for any tag name or value.
fn validate_message_tags(tags: &[MessageTag]) -> Result<(), SesError> {
    let pairs: Vec<(String, String)> = tags
        .iter()
        .map(|t| (t.name.clone(), t.value.clone()))
        .collect();
    validate_tags(&pairs)
}

/// Convert a slice of `MessageTag` into `SentEmailTag` for retrospection storage.
fn convert_tags(tags: &[MessageTag]) -> Vec<SentEmailTag> {
    tags.iter()
        .map(|t| SentEmailTag {
            name: t.name.clone(),
            value: t.value.clone(),
        })
        .collect()
}

/// Main SES provider implementing all operations.
#[derive(Debug)]
pub struct RustackSes {
    /// Identity store.
    pub(crate) identities: Arc<IdentityStore>,
    /// Email store for retrospection.
    pub(crate) emails: Arc<EmailStore>,
    /// Template store.
    pub(crate) templates: Arc<TemplateStore>,
    /// Configuration set store.
    pub(crate) config_sets: Arc<ConfigurationSetStore>,
    /// Receipt rule set store.
    pub(crate) receipt_rules: Arc<ReceiptRuleSetStore>,
    /// Send statistics.
    pub(crate) statistics: Arc<SendStatistics>,
    /// Service configuration.
    pub(crate) config: Arc<SesConfig>,
}

#[allow(clippy::needless_pass_by_value)]
impl RustackSes {
    /// Create a new SES provider with the given configuration.
    #[must_use]
    pub fn new(config: SesConfig) -> Self {
        Self {
            identities: Arc::new(IdentityStore::new()),
            emails: Arc::new(EmailStore::new()),
            templates: Arc::new(TemplateStore::new()),
            config_sets: Arc::new(ConfigurationSetStore::new()),
            receipt_rules: Arc::new(ReceiptRuleSetStore::new()),
            statistics: Arc::new(SendStatistics::new()),
            config: Arc::new(config),
        }
    }

    /// Get a reference to the email store for retrospection.
    #[must_use]
    pub fn email_store(&self) -> &Arc<EmailStore> {
        &self.emails
    }

    /// Get a reference to the configuration.
    #[must_use]
    pub fn config(&self) -> &SesConfig {
        &self.config
    }

    // ---------------------------------------------------------------
    // Phase 0: Core Sending + Identities
    // ---------------------------------------------------------------

    /// Verify an email identity.
    pub fn verify_email_identity(
        &self,
        input: VerifyEmailIdentityInput,
    ) -> Result<VerifyEmailIdentityResponse, SesError> {
        debug!(email = %input.email_address, "verify email identity");
        let _ = self.identities.verify_email(&input.email_address);
        Ok(VerifyEmailIdentityResponse {})
    }

    /// Verify a domain identity.
    pub fn verify_domain_identity(
        &self,
        input: VerifyDomainIdentityInput,
    ) -> Result<VerifyDomainIdentityResponse, SesError> {
        debug!(domain = %input.domain, "verify domain identity");
        let (_, token) = self.identities.verify_domain(&input.domain);
        Ok(VerifyDomainIdentityResponse {
            verification_token: token,
        })
    }

    /// List all identities, optionally filtered by type.
    pub fn list_identities(
        &self,
        input: ListIdentitiesInput,
    ) -> Result<ListIdentitiesResponse, SesError> {
        let identities = self.identities.list(input.identity_type.as_ref());
        Ok(ListIdentitiesResponse {
            identities,
            next_token: None,
        })
    }

    /// Delete an identity.
    pub fn delete_identity(
        &self,
        input: DeleteIdentityInput,
    ) -> Result<DeleteIdentityResponse, SesError> {
        debug!(identity = %input.identity, "delete identity");
        self.identities.delete(&input.identity);
        Ok(DeleteIdentityResponse {})
    }

    /// Get verification attributes for identities.
    pub fn get_identity_verification_attributes(
        &self,
        input: GetIdentityVerificationAttributesInput,
    ) -> Result<GetIdentityVerificationAttributesResponse, SesError> {
        let verification_attributes = self
            .identities
            .get_verification_attributes(&input.identities);
        Ok(GetIdentityVerificationAttributesResponse {
            verification_attributes,
        })
    }

    /// Legacy API: verify an email address.
    pub fn verify_email_address(&self, input: VerifyEmailAddressInput) -> Result<(), SesError> {
        debug!(email = %input.email_address, "verify email address (legacy)");
        let _ = self.identities.verify_email(&input.email_address);
        Ok(())
    }

    /// Legacy API: delete a verified email address.
    pub fn delete_verified_email_address(
        &self,
        input: DeleteVerifiedEmailAddressInput,
    ) -> Result<(), SesError> {
        self.identities.delete(&input.email_address);
        Ok(())
    }

    /// Legacy API: list verified email addresses.
    pub fn list_verified_email_addresses(
        &self,
    ) -> Result<ListVerifiedEmailAddressesResponse, SesError> {
        let verified_email_addresses = self.identities.list(Some(&IdentityType::EmailAddress));
        Ok(ListVerifiedEmailAddressesResponse {
            verified_email_addresses,
        })
    }

    /// Send an email.
    pub fn send_email(&self, input: SendEmailInput) -> Result<SendEmailResponse, SesError> {
        validate_message_tags(&input.tags)?;

        // Optionally validate source is verified
        if self.config.require_verified_identity && !self.identities.is_verified(&input.source) {
            return Err(SesError::with_message(
                SesErrorCode::MessageRejected,
                format!(
                    "Email address is not verified. The following identities failed the check in \
                     region {}: {}",
                    self.config.default_region, input.source
                ),
            ));
        }

        let message_id = uuid::Uuid::new_v4().to_string();
        let sent = SentEmail {
            id: message_id.clone(),
            region: self.config.default_region.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            source: input.source,
            destination: SentEmailDestination {
                to_addresses: input.destination.to_addresses,
                cc_addresses: input.destination.cc_addresses,
                bcc_addresses: input.destination.bcc_addresses,
            },
            subject: Some(input.message.subject.data.clone()),
            body: Some(SentEmailBody {
                text_part: input.message.body.text.as_ref().map(|t| t.data.clone()),
                html_part: input.message.body.html.as_ref().map(|h| h.data.clone()),
            }),
            raw_data: None,
            template: None,
            template_data: None,
            tags: convert_tags(&input.tags),
        };
        self.emails.capture(sent);
        self.statistics.record_send();
        debug!(message_id = %message_id, "email sent");
        Ok(SendEmailResponse { message_id })
    }

    /// Send a raw MIME email.
    pub fn send_raw_email(
        &self,
        input: SendRawEmailInput,
    ) -> Result<SendRawEmailResponse, SesError> {
        validate_message_tags(&input.tags)?;

        let source = input
            .source
            .clone()
            .unwrap_or_else(|| extract_from_raw(&input.raw_message.data));

        if source.is_empty() {
            return Err(SesError::with_message(
                SesErrorCode::MessageRejected,
                "Could not determine source address from raw message.",
            ));
        }

        if self.config.require_verified_identity && !self.identities.is_verified(&source) {
            return Err(SesError::with_message(
                SesErrorCode::MessageRejected,
                format!(
                    "Email address is not verified. The following identities failed the check in \
                     region {}: {source}",
                    self.config.default_region
                ),
            ));
        }

        let message_id = uuid::Uuid::new_v4().to_string();
        let raw_str = String::from_utf8_lossy(&input.raw_message.data).into_owned();
        let sent = SentEmail {
            id: message_id.clone(),
            region: self.config.default_region.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            source,
            destination: SentEmailDestination {
                to_addresses: input.destinations,
                cc_addresses: Vec::new(),
                bcc_addresses: Vec::new(),
            },
            subject: None,
            body: None,
            raw_data: Some(raw_str),
            template: None,
            template_data: None,
            tags: convert_tags(&input.tags),
        };
        self.emails.capture(sent);
        self.statistics.record_send();
        debug!(message_id = %message_id, "raw email sent");
        Ok(SendRawEmailResponse { message_id })
    }

    /// Get send quota.
    #[allow(clippy::cast_precision_loss)]
    pub fn get_send_quota(&self) -> Result<GetSendQuotaResponse, SesError> {
        Ok(GetSendQuotaResponse {
            max24_hour_send: Some(self.config.max_24_hour_send),
            max_send_rate: Some(self.config.max_send_rate),
            sent_last24_hours: Some(self.emails.total_sent() as f64),
        })
    }

    /// Get send statistics.
    #[allow(clippy::cast_possible_wrap)]
    pub fn get_send_statistics(&self) -> Result<GetSendStatisticsResponse, SesError> {
        let stats = self.statistics.get_stats();
        let data_point = SendDataPoint {
            delivery_attempts: Some(stats.delivery_attempts as i64),
            bounces: Some(stats.bounce_count as i64),
            complaints: Some(stats.complaint_count as i64),
            rejects: Some(stats.reject_count as i64),
            timestamp: Some(chrono::Utc::now()),
        };
        Ok(GetSendStatisticsResponse {
            send_data_points: vec![data_point],
        })
    }

    // ---------------------------------------------------------------
    // Phase 1: Templates + Configuration Sets
    // ---------------------------------------------------------------

    /// Create a template.
    pub fn create_template(
        &self,
        input: CreateTemplateInput,
    ) -> Result<CreateTemplateResponse, SesError> {
        debug!(template = %input.template.template_name, "create template");
        self.templates.create(input.template)?;
        Ok(CreateTemplateResponse {})
    }

    /// Get a template.
    pub fn get_template(&self, input: GetTemplateInput) -> Result<GetTemplateResponse, SesError> {
        let template = self.templates.get(&input.template_name)?;
        Ok(GetTemplateResponse {
            template: Some(template),
        })
    }

    /// Update a template.
    pub fn update_template(
        &self,
        input: UpdateTemplateInput,
    ) -> Result<UpdateTemplateResponse, SesError> {
        debug!(template = %input.template.template_name, "update template");
        self.templates.update(input.template)?;
        Ok(UpdateTemplateResponse {})
    }

    /// Delete a template.
    pub fn delete_template(
        &self,
        input: DeleteTemplateInput,
    ) -> Result<DeleteTemplateResponse, SesError> {
        debug!(template = %input.template_name, "delete template");
        self.templates.delete(&input.template_name);
        Ok(DeleteTemplateResponse {})
    }

    /// List templates.
    pub fn list_templates(
        &self,
        _input: ListTemplatesInput,
    ) -> Result<ListTemplatesResponse, SesError> {
        let templates_metadata = self.templates.list();
        Ok(ListTemplatesResponse {
            templates_metadata,
            next_token: None,
        })
    }

    /// Send a templated email.
    pub fn send_templated_email(
        &self,
        input: SendTemplatedEmailInput,
    ) -> Result<SendTemplatedEmailResponse, SesError> {
        validate_message_tags(&input.tags)?;

        if self.config.require_verified_identity && !self.identities.is_verified(&input.source) {
            return Err(SesError::with_message(
                SesErrorCode::MessageRejected,
                format!(
                    "Email address is not verified. The following identities failed the check in \
                     region {}: {}",
                    self.config.default_region, input.source
                ),
            ));
        }

        let template = self.templates.get(&input.template)?;
        let rendered_subject = template
            .subject_part
            .as_deref()
            .map(|s| render_template(s, &input.template_data))
            .transpose()?;
        let rendered_text = template
            .text_part
            .as_deref()
            .map(|s| render_template(s, &input.template_data))
            .transpose()?;
        let rendered_html = template
            .html_part
            .as_deref()
            .map(|s| render_template(s, &input.template_data))
            .transpose()?;

        let message_id = uuid::Uuid::new_v4().to_string();
        let sent = SentEmail {
            id: message_id.clone(),
            region: self.config.default_region.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            source: input.source,
            destination: SentEmailDestination {
                to_addresses: input.destination.to_addresses,
                cc_addresses: input.destination.cc_addresses,
                bcc_addresses: input.destination.bcc_addresses,
            },
            subject: rendered_subject,
            body: Some(SentEmailBody {
                text_part: rendered_text,
                html_part: rendered_html,
            }),
            raw_data: None,
            template: Some(input.template),
            template_data: Some(input.template_data),
            tags: convert_tags(&input.tags),
        };
        self.emails.capture(sent);
        self.statistics.record_send();
        debug!(message_id = %message_id, "templated email sent");
        Ok(SendTemplatedEmailResponse { message_id })
    }

    /// Create a configuration set.
    pub fn create_configuration_set(
        &self,
        input: CreateConfigurationSetInput,
    ) -> Result<CreateConfigurationSetResponse, SesError> {
        debug!(name = %input.configuration_set.name, "create configuration set");
        self.config_sets.create(&input.configuration_set.name)?;
        Ok(CreateConfigurationSetResponse {})
    }

    /// Delete a configuration set.
    pub fn delete_configuration_set(
        &self,
        input: DeleteConfigurationSetInput,
    ) -> Result<DeleteConfigurationSetResponse, SesError> {
        debug!(name = %input.configuration_set_name, "delete configuration set");
        self.config_sets.delete(&input.configuration_set_name)?;
        Ok(DeleteConfigurationSetResponse {})
    }

    /// Describe a configuration set.
    pub fn describe_configuration_set(
        &self,
        input: DescribeConfigurationSetInput,
    ) -> Result<DescribeConfigurationSetResponse, SesError> {
        let record = self.config_sets.describe(&input.configuration_set_name)?;
        Ok(DescribeConfigurationSetResponse {
            configuration_set: Some(ConfigurationSet { name: record.name }),
            event_destinations: record.event_destinations,
            reputation_options: None,
            tracking_options: None,
            delivery_options: None,
        })
    }

    /// List configuration sets.
    pub fn list_configuration_sets(
        &self,
        _input: ListConfigurationSetsInput,
    ) -> Result<ListConfigurationSetsResponse, SesError> {
        let names = self.config_sets.list();
        let configuration_sets = names
            .into_iter()
            .map(|name| ConfigurationSet { name })
            .collect();
        Ok(ListConfigurationSetsResponse {
            configuration_sets,
            next_token: None,
        })
    }

    // ---------------------------------------------------------------
    // Phase 2: Event Destinations + Receipt Rules
    // ---------------------------------------------------------------

    /// Create a configuration set event destination.
    pub fn create_configuration_set_event_destination(
        &self,
        input: CreateConfigurationSetEventDestinationInput,
    ) -> Result<CreateConfigurationSetEventDestinationResponse, SesError> {
        self.config_sets
            .add_event_destination(&input.configuration_set_name, input.event_destination)?;
        Ok(CreateConfigurationSetEventDestinationResponse {})
    }

    /// Update a configuration set event destination.
    pub fn update_configuration_set_event_destination(
        &self,
        input: UpdateConfigurationSetEventDestinationInput,
    ) -> Result<UpdateConfigurationSetEventDestinationResponse, SesError> {
        self.config_sets
            .update_event_destination(&input.configuration_set_name, input.event_destination)?;
        Ok(UpdateConfigurationSetEventDestinationResponse {})
    }

    /// Delete a configuration set event destination.
    pub fn delete_configuration_set_event_destination(
        &self,
        input: DeleteConfigurationSetEventDestinationInput,
    ) -> Result<DeleteConfigurationSetEventDestinationResponse, SesError> {
        self.config_sets.delete_event_destination(
            &input.configuration_set_name,
            &input.event_destination_name,
        )?;
        Ok(DeleteConfigurationSetEventDestinationResponse {})
    }

    /// Create a receipt rule set.
    pub fn create_receipt_rule_set(
        &self,
        input: CreateReceiptRuleSetInput,
    ) -> Result<CreateReceiptRuleSetResponse, SesError> {
        debug!(name = %input.rule_set_name, "create receipt rule set");
        self.receipt_rules.create_rule_set(&input.rule_set_name)?;
        Ok(CreateReceiptRuleSetResponse {})
    }

    /// Delete a receipt rule set.
    pub fn delete_receipt_rule_set(
        &self,
        input: DeleteReceiptRuleSetInput,
    ) -> Result<DeleteReceiptRuleSetResponse, SesError> {
        self.receipt_rules.delete_rule_set(&input.rule_set_name)?;
        Ok(DeleteReceiptRuleSetResponse {})
    }

    /// Create a receipt rule.
    pub fn create_receipt_rule(
        &self,
        input: CreateReceiptRuleInput,
    ) -> Result<CreateReceiptRuleResponse, SesError> {
        self.receipt_rules
            .create_rule(&input.rule_set_name, input.rule, input.after.as_deref())?;
        Ok(CreateReceiptRuleResponse {})
    }

    /// Delete a receipt rule.
    pub fn delete_receipt_rule(
        &self,
        input: DeleteReceiptRuleInput,
    ) -> Result<DeleteReceiptRuleResponse, SesError> {
        self.receipt_rules
            .delete_rule(&input.rule_set_name, &input.rule_name)?;
        Ok(DeleteReceiptRuleResponse {})
    }

    /// Describe a receipt rule set.
    pub fn describe_receipt_rule_set(
        &self,
        input: DescribeReceiptRuleSetInput,
    ) -> Result<DescribeReceiptRuleSetResponse, SesError> {
        let record = self.receipt_rules.describe_rule_set(&input.rule_set_name)?;
        Ok(DescribeReceiptRuleSetResponse {
            metadata: Some(ReceiptRuleSetMetadata {
                name: Some(record.name),
                created_timestamp: Some(record.created_timestamp),
            }),
            rules: record.rules,
        })
    }

    /// Clone a receipt rule set.
    pub fn clone_receipt_rule_set(
        &self,
        input: CloneReceiptRuleSetInput,
    ) -> Result<CloneReceiptRuleSetResponse, SesError> {
        self.receipt_rules
            .clone_rule_set(&input.original_rule_set_name, &input.rule_set_name)?;
        Ok(CloneReceiptRuleSetResponse {})
    }

    /// Describe the active receipt rule set.
    pub fn describe_active_receipt_rule_set(
        &self,
        _input: DescribeActiveReceiptRuleSetInput,
    ) -> Result<DescribeActiveReceiptRuleSetResponse, SesError> {
        if let Some((metadata, rules)) = self.receipt_rules.get_active_rule_set() {
            Ok(DescribeActiveReceiptRuleSetResponse {
                metadata: Some(metadata),
                rules,
            })
        } else {
            Ok(DescribeActiveReceiptRuleSetResponse {
                metadata: None,
                rules: Vec::new(),
            })
        }
    }

    /// Set the active receipt rule set.
    pub fn set_active_receipt_rule_set(
        &self,
        input: SetActiveReceiptRuleSetInput,
    ) -> Result<SetActiveReceiptRuleSetResponse, SesError> {
        self.receipt_rules
            .set_active_rule_set(input.rule_set_name.as_deref())?;
        Ok(SetActiveReceiptRuleSetResponse {})
    }

    // ---------------------------------------------------------------
    // Phase 3: Identity Configuration + Sending Authorization
    // ---------------------------------------------------------------

    /// Set identity notification topic.
    pub fn set_identity_notification_topic(
        &self,
        input: SetIdentityNotificationTopicInput,
    ) -> Result<SetIdentityNotificationTopicResponse, SesError> {
        self.identities.set_notification_topic(
            &input.identity,
            &input.notification_type,
            input.sns_topic,
        );
        Ok(SetIdentityNotificationTopicResponse {})
    }

    /// Set identity feedback forwarding enabled.
    pub fn set_identity_feedback_forwarding_enabled(
        &self,
        input: SetIdentityFeedbackForwardingEnabledInput,
    ) -> Result<SetIdentityFeedbackForwardingEnabledResponse, SesError> {
        self.identities
            .set_feedback_forwarding_enabled(&input.identity, input.forwarding_enabled);
        Ok(SetIdentityFeedbackForwardingEnabledResponse {})
    }

    /// Get identity notification attributes.
    pub fn get_identity_notification_attributes(
        &self,
        input: GetIdentityNotificationAttributesInput,
    ) -> Result<GetIdentityNotificationAttributesResponse, SesError> {
        let notification_attributes = self
            .identities
            .get_notification_attributes(&input.identities);
        Ok(GetIdentityNotificationAttributesResponse {
            notification_attributes,
        })
    }

    /// Verify domain DKIM.
    pub fn verify_domain_dkim(
        &self,
        input: VerifyDomainDkimInput,
    ) -> Result<VerifyDomainDkimResponse, SesError> {
        let dkim_tokens = self.identities.verify_domain_dkim(&input.domain);
        Ok(VerifyDomainDkimResponse { dkim_tokens })
    }

    /// Get identity DKIM attributes.
    pub fn get_identity_dkim_attributes(
        &self,
        input: GetIdentityDkimAttributesInput,
    ) -> Result<GetIdentityDkimAttributesResponse, SesError> {
        let dkim_attributes = self.identities.get_dkim_attributes(&input.identities);
        Ok(GetIdentityDkimAttributesResponse { dkim_attributes })
    }

    /// Set identity mail-from domain.
    pub fn set_identity_mail_from_domain(
        &self,
        input: SetIdentityMailFromDomainInput,
    ) -> Result<SetIdentityMailFromDomainResponse, SesError> {
        self.identities.set_mail_from_domain(
            &input.identity,
            input.mail_from_domain,
            input.behavior_on_mx_failure,
        );
        Ok(SetIdentityMailFromDomainResponse {})
    }

    /// Get identity mail-from domain attributes.
    pub fn get_identity_mail_from_domain_attributes(
        &self,
        input: GetIdentityMailFromDomainAttributesInput,
    ) -> Result<GetIdentityMailFromDomainAttributesResponse, SesError> {
        let mail_from_domain_attributes = self
            .identities
            .get_mail_from_domain_attributes(&input.identities);
        Ok(GetIdentityMailFromDomainAttributesResponse {
            mail_from_domain_attributes,
        })
    }

    /// Get identity policies.
    pub fn get_identity_policies(
        &self,
        input: GetIdentityPoliciesInput,
    ) -> Result<GetIdentityPoliciesResponse, SesError> {
        let policies = self
            .identities
            .get_policies(&input.identity, &input.policy_names);
        Ok(GetIdentityPoliciesResponse { policies })
    }

    /// Put (create or update) an identity policy.
    pub fn put_identity_policy(
        &self,
        input: PutIdentityPolicyInput,
    ) -> Result<PutIdentityPolicyResponse, SesError> {
        self.identities
            .put_policy(&input.identity, &input.policy_name, &input.policy);
        Ok(PutIdentityPolicyResponse {})
    }

    /// Delete an identity policy.
    pub fn delete_identity_policy(
        &self,
        input: DeleteIdentityPolicyInput,
    ) -> Result<DeleteIdentityPolicyResponse, SesError> {
        self.identities
            .delete_policy(&input.identity, &input.policy_name);
        Ok(DeleteIdentityPolicyResponse {})
    }

    /// List identity policy names.
    pub fn list_identity_policies(
        &self,
        input: ListIdentityPoliciesInput,
    ) -> Result<ListIdentityPoliciesResponse, SesError> {
        let policy_names = self.identities.list_policy_names(&input.identity);
        Ok(ListIdentityPoliciesResponse { policy_names })
    }
}

/// Extract the `From:` address from raw MIME data.
///
/// Uses case-insensitive matching for the header name per RFC 2822.
fn extract_from_raw(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    for line in text.lines() {
        if line.len() >= 5 && line[..5].eq_ignore_ascii_case("from:") {
            let addr = line[5..].trim();
            // Handle "Name <email>" format
            if let Some(start) = addr.find('<') {
                if let Some(end) = addr.find('>') {
                    return addr[start + 1..end].to_owned();
                }
            }
            return addr.to_owned();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use rustack_ses_model::types::{Body, Content, Destination, Message, Template};

    use super::*;

    fn make_provider() -> RustackSes {
        RustackSes::new(SesConfig::default())
    }

    #[test]
    fn test_should_verify_and_list_email() {
        let p = make_provider();
        p.verify_email_identity(VerifyEmailIdentityInput {
            email_address: "test@example.com".to_owned(),
        })
        .unwrap_or_default();
        let resp = p
            .list_identities(ListIdentitiesInput::default())
            .unwrap_or_default();
        assert!(resp.identities.contains(&"test@example.com".to_owned()));
    }

    #[test]
    fn test_should_verify_domain_and_return_token() {
        let p = make_provider();
        let resp = p
            .verify_domain_identity(VerifyDomainIdentityInput {
                domain: "example.com".to_owned(),
            })
            .unwrap_or_default();
        assert_ne!(resp.verification_token, "");
    }

    #[test]
    fn test_should_send_email_and_capture() {
        let p = make_provider();
        let resp = p
            .send_email(SendEmailInput {
                source: "sender@example.com".to_owned(),
                destination: Destination {
                    to_addresses: vec!["recipient@example.com".to_owned()],
                    ..Destination::default()
                },
                message: Message {
                    subject: Content {
                        data: "Test".to_owned(),
                        ..Content::default()
                    },
                    body: Body {
                        text: Some(Content {
                            data: "Hello".to_owned(),
                            ..Content::default()
                        }),
                        ..Body::default()
                    },
                },
                ..SendEmailInput::default()
            })
            .unwrap_or_default();
        assert_ne!(resp.message_id, "");
        let emails = p.emails.query(None, None);
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].subject.as_deref(), Some("Test"));
    }

    #[test]
    fn test_should_reject_unverified_in_strict_mode() {
        let p = RustackSes::new(SesConfig {
            require_verified_identity: true,
            ..SesConfig::default()
        });
        let result = p.send_email(SendEmailInput {
            source: "unverified@example.com".to_owned(),
            destination: Destination {
                to_addresses: vec!["r@e.com".to_owned()],
                ..Destination::default()
            },
            message: Message {
                subject: Content {
                    data: "Test".to_owned(),
                    ..Content::default()
                },
                body: Body::default(),
            },
            ..SendEmailInput::default()
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_should_send_templated_email() {
        let p = make_provider();
        p.create_template(CreateTemplateInput {
            template: Template {
                template_name: "welcome".to_owned(),
                subject_part: Some("Hello {{name}}".to_owned()),
                text_part: Some("Welcome {{name}}!".to_owned()),
                html_part: None,
            },
        })
        .unwrap_or_default();
        let resp = p
            .send_templated_email(SendTemplatedEmailInput {
                source: "s@e.com".to_owned(),
                destination: Destination {
                    to_addresses: vec!["r@e.com".to_owned()],
                    ..Destination::default()
                },
                template: "welcome".to_owned(),
                template_data: r#"{"name":"World"}"#.to_owned(),
                ..SendTemplatedEmailInput::default()
            })
            .unwrap_or_default();
        assert_ne!(resp.message_id, "");
        let emails = p.emails.query(None, None);
        assert_eq!(emails[0].subject.as_deref(), Some("Hello World"));
    }

    #[test]
    fn test_should_get_send_quota() {
        let p = make_provider();
        let resp = p.get_send_quota().unwrap_or_default();
        assert!((resp.max24_hour_send.unwrap_or_default() - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_should_extract_from_raw() {
        assert_eq!(
            extract_from_raw(b"From: sender@example.com\r\nSubject: Test"),
            "sender@example.com"
        );
        assert_eq!(
            extract_from_raw(b"From: John Doe <john@example.com>\r\n"),
            "john@example.com"
        );
        assert_eq!(extract_from_raw(b"Subject: No From"), "");
    }

    #[test]
    fn test_should_extract_from_raw_case_insensitive() {
        assert_eq!(
            extract_from_raw(b"from: lower@example.com\r\nSubject: Test"),
            "lower@example.com"
        );
        assert_eq!(
            extract_from_raw(b"FROM: UPPER@example.com\r\nSubject: Test"),
            "UPPER@example.com"
        );
        assert_eq!(
            extract_from_raw(b"fRoM: mixed@example.com\r\nSubject: Test"),
            "mixed@example.com"
        );
    }
}
