//! Main SNS provider implementing Phase 0 and Phase 1 operations.
//!
//! Acts as the topic manager that owns all topic state and
//! coordinates message fan-out to subscribers.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use tracing::{debug, warn};

use ruststack_sns_model::error::{SnsError, SnsErrorCode};
use ruststack_sns_model::input::{
    ConfirmSubscriptionInput, CreateTopicInput, DeleteTopicInput, GetSubscriptionAttributesInput,
    GetTopicAttributesInput, ListSubscriptionsByTopicInput, ListSubscriptionsInput,
    ListTagsForResourceInput, ListTopicsInput, PublishBatchInput, PublishInput,
    SetSubscriptionAttributesInput, SetTopicAttributesInput, SubscribeInput, TagResourceInput,
    UnsubscribeInput, UntagResourceInput,
};
use ruststack_sns_model::output::{
    ConfirmSubscriptionOutput, CreateTopicOutput, DeleteTopicOutput,
    GetSubscriptionAttributesOutput, GetTopicAttributesOutput, ListSubscriptionsByTopicOutput,
    ListSubscriptionsOutput, ListTagsForResourceOutput, ListTopicsOutput, PublishBatchOutput,
    PublishOutput, SetSubscriptionAttributesOutput, SetTopicAttributesOutput, SubscribeOutput,
    TagResourceOutput, UnsubscribeOutput, UntagResourceOutput,
};
use ruststack_sns_model::types::{
    BatchResultErrorEntry, PublishBatchResultEntry, Subscription, Tag, Topic,
};

use crate::config::SnsConfig;
use crate::delivery::{EnvelopeParams, build_sns_envelope};
use crate::filter::{evaluate_filter_policy, resolve_protocol_message};
use crate::publisher::SqsPublisher;
use crate::state::TopicStore;
use crate::subscription::{
    FilterPolicyScope, SubscriptionAttributes, SubscriptionProtocol, SubscriptionRecord,
};
use crate::topic::{TopicAttributes, TopicRecord};

/// Maximum number of topics returned per `ListTopics` page.
const LIST_TOPICS_PAGE_SIZE: usize = 100;

/// Maximum number of subscriptions returned per list page.
const LIST_SUBSCRIPTIONS_PAGE_SIZE: usize = 100;

/// Maximum message size in bytes (256 KiB).
const MAX_MESSAGE_SIZE: usize = 256 * 1024;

/// Maximum number of tags per resource.
const MAX_TAGS_PER_RESOURCE: usize = 50;

/// Maximum number of entries in a `PublishBatch` request.
const MAX_PUBLISH_BATCH_ENTRIES: usize = 10;

/// Main SNS provider.
pub struct RustStackSns {
    /// Topic store.
    topics: TopicStore,
    /// SQS publisher for fan-out delivery.
    sqs_publisher: Arc<dyn SqsPublisher>,
    /// Service configuration.
    config: Arc<SnsConfig>,
}

impl fmt::Debug for RustStackSns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RustStackSns")
            .field("topics", &self.topics)
            .field("sqs_publisher", &"<dyn SqsPublisher>")
            .field("config", &self.config)
            .finish()
    }
}

impl RustStackSns {
    /// Create a new SNS provider.
    #[must_use]
    pub fn new(config: SnsConfig, sqs_publisher: Arc<dyn SqsPublisher>) -> Self {
        Self {
            topics: TopicStore::new(),
            sqs_publisher,
            config: Arc::new(config),
        }
    }

    // ---- Topic Management ----

    /// Handle `CreateTopic`.
    pub fn create_topic(&self, input: CreateTopicInput) -> Result<CreateTopicOutput, SnsError> {
        let name = &input.name;
        validate_topic_name(name)?;

        // Not a file extension - this is an AWS SNS FIFO topic naming convention.
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        let is_fifo = name.ends_with(".fifo");

        // Check FifoTopic attribute consistency.
        if let Some(fifo_attr) = input.attributes.get("FifoTopic") {
            if fifo_attr.eq_ignore_ascii_case("true") && !is_fifo {
                return Err(SnsError::invalid_parameter(
                    "Invalid parameter: FifoTopic Reason: \
                     FIFO topics must have a name ending with '.fifo'",
                ));
            }
            if !fifo_attr.eq_ignore_ascii_case("true") && is_fifo {
                return Err(SnsError::invalid_parameter(
                    "Invalid parameter: FifoTopic Reason: Topic name ending with '.fifo' \
                     must have FifoTopic attribute set to true",
                ));
            }
        }

        let arn = format!(
            "arn:aws:sns:{}:{}:{name}",
            self.config.default_region, self.config.account_id
        );

        // Idempotent: if topic already exists, return its ARN.
        if self.topics.get_topic(&arn).is_some() {
            return Ok(CreateTopicOutput { topic_arn: arn });
        }

        let attributes =
            TopicAttributes::from_input(&input.attributes, is_fifo, &self.config.account_id);

        let tags: HashMap<String, String> =
            input.tags.into_iter().map(|t| (t.key, t.value)).collect();

        let now = chrono::Utc::now().timestamp().unsigned_abs();

        let topic = TopicRecord {
            arn: arn.clone(),
            name: name.clone(),
            is_fifo,
            attributes,
            subscriptions: Vec::new(),
            tags,
            data_protection_policy: input.data_protection_policy,
            created_at: now,
            subscription_counter: 0,
        };

        self.topics.insert_topic(topic);
        debug!(topic_arn = %arn, "created topic");

        Ok(CreateTopicOutput { topic_arn: arn })
    }

    /// Handle `DeleteTopic`.
    pub fn delete_topic(&self, input: &DeleteTopicInput) -> Result<DeleteTopicOutput, SnsError> {
        // Idempotent: no error if topic does not exist.
        if let Some(topic) = self.topics.remove_topic(&input.topic_arn) {
            debug!(
                topic_arn = %input.topic_arn,
                subs = topic.subscriptions.len(),
                "deleted topic"
            );
        }
        Ok(DeleteTopicOutput {})
    }

    /// Handle `GetTopicAttributes`.
    pub fn get_topic_attributes(
        &self,
        input: &GetTopicAttributesInput,
    ) -> Result<GetTopicAttributesOutput, SnsError> {
        let topic = self
            .topics
            .get_topic(&input.topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        let attributes = topic.attributes.to_map(&topic);
        Ok(GetTopicAttributesOutput { attributes })
    }

    /// Handle `SetTopicAttributes`.
    pub fn set_topic_attributes(
        &self,
        input: SetTopicAttributesInput,
    ) -> Result<SetTopicAttributesOutput, SnsError> {
        let mut topic = self
            .topics
            .get_topic_mut(&input.topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        let value = input.attribute_value.unwrap_or_default();

        match input.attribute_name.as_str() {
            "DisplayName" => topic.attributes.display_name = value,
            "Policy" => topic.attributes.policy = Some(value),
            "DeliveryPolicy" => {
                topic.attributes.delivery_policy = Some(value.clone());
                topic.attributes.effective_delivery_policy = Some(value);
            }
            "KmsMasterKeyId" => topic.attributes.kms_master_key_id = Some(value),
            "SignatureVersion" => topic.attributes.signature_version = value,
            "ContentBasedDeduplication" => {
                if topic.is_fifo {
                    topic.attributes.content_based_deduplication =
                        value.eq_ignore_ascii_case("true");
                } else {
                    return Err(SnsError::invalid_parameter(
                        "Invalid parameter: ContentBasedDeduplication Reason: \
                         Content-based deduplication can only be set for FIFO topics",
                    ));
                }
            }
            other => {
                return Err(SnsError::invalid_parameter(format!(
                    "Invalid parameter: AttributeName Reason: Unknown attribute {other}"
                )));
            }
        }

        Ok(SetTopicAttributesOutput {})
    }

    /// Handle `ListTopics`.
    pub fn list_topics(&self, input: &ListTopicsInput) -> Result<ListTopicsOutput, SnsError> {
        let mut sorted = self.topics.list_topics();
        sorted.sort();

        let start = input
            .next_token
            .as_ref()
            .and_then(|t| t.parse::<usize>().ok())
            .unwrap_or(0);

        let page = &sorted[start.min(sorted.len())..];
        let (page, next_token) = if page.len() > LIST_TOPICS_PAGE_SIZE {
            (
                &page[..LIST_TOPICS_PAGE_SIZE],
                Some((start + LIST_TOPICS_PAGE_SIZE).to_string()),
            )
        } else {
            (page, None)
        };

        let topics = page
            .iter()
            .map(|arn| Topic {
                topic_arn: arn.clone(),
            })
            .collect();

        Ok(ListTopicsOutput { topics, next_token })
    }

    // ---- Subscription Management ----

    /// Handle `Subscribe`.
    pub fn subscribe(&self, input: SubscribeInput) -> Result<SubscribeOutput, SnsError> {
        let mut topic = self
            .topics
            .get_topic_mut(&input.topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        let protocol = SubscriptionProtocol::parse(&input.protocol)?;

        let endpoint = input.endpoint.unwrap_or_default();
        if endpoint.is_empty() {
            return Err(SnsError::invalid_parameter(
                "Invalid parameter: Endpoint Reason: Endpoint is required",
            ));
        }

        // Idempotent: check for duplicate (same protocol + endpoint).
        for existing in &topic.subscriptions {
            if existing.protocol == protocol && existing.endpoint == endpoint {
                let sub_arn = if existing.confirmed || input.return_subscription_arn {
                    Some(existing.arn.clone())
                } else {
                    Some("PendingConfirmation".to_owned())
                };
                return Ok(SubscribeOutput {
                    subscription_arn: sub_arn,
                });
            }
        }

        // Parse subscription attributes.
        let sub_attrs = SubscriptionAttributes::from_input(&input.attributes)?;

        topic.subscription_counter += 1;
        let sub_id = uuid::Uuid::new_v4();
        let sub_arn = format!("{}:{sub_id}", input.topic_arn);

        let confirmed = protocol.is_auto_confirmed();
        let confirmation_token = if confirmed {
            None
        } else {
            Some(uuid::Uuid::new_v4().to_string())
        };

        let sub = SubscriptionRecord {
            arn: sub_arn.clone(),
            topic_arn: input.topic_arn.clone(),
            protocol,
            endpoint,
            owner: topic.attributes.owner.clone(),
            confirmed,
            confirmation_token,
            attributes: sub_attrs,
        };

        self.topics
            .add_subscription_index(&sub_arn, &input.topic_arn);
        topic.subscriptions.push(sub);

        let result_arn = if confirmed || input.return_subscription_arn {
            Some(sub_arn)
        } else {
            Some("PendingConfirmation".to_owned())
        };

        debug!(
            topic_arn = %input.topic_arn,
            subscription_arn = ?result_arn,
            "created subscription"
        );

        Ok(SubscribeOutput {
            subscription_arn: result_arn,
        })
    }

    /// Handle `Unsubscribe`.
    pub fn unsubscribe(&self, input: &UnsubscribeInput) -> Result<UnsubscribeOutput, SnsError> {
        let topic_arn = self
            .topics
            .find_topic_for_subscription(&input.subscription_arn);

        if let Some(topic_arn) = topic_arn {
            if let Some(mut topic) = self.topics.get_topic_mut(&topic_arn) {
                topic
                    .subscriptions
                    .retain(|s| s.arn != input.subscription_arn);
            }
            self.topics
                .remove_subscription_index(&input.subscription_arn);
            debug!(subscription_arn = %input.subscription_arn, "unsubscribed");
        }
        // Idempotent: no error if subscription does not exist.
        Ok(UnsubscribeOutput {})
    }

    /// Handle `ConfirmSubscription`.
    ///
    /// For local development, accepts any non-empty token and marks the
    /// subscription as confirmed.
    pub fn confirm_subscription(
        &self,
        input: &ConfirmSubscriptionInput,
    ) -> Result<ConfirmSubscriptionOutput, SnsError> {
        let mut topic = self
            .topics
            .get_topic_mut(&input.topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        if input.token.is_empty() {
            return Err(SnsError::invalid_parameter(
                "Invalid parameter: Token Reason: Token is required",
            ));
        }

        // Find subscription with matching token or any unconfirmed subscription
        // (local dev mode is lenient about tokens).
        // First try exact token match, then fall back to any unconfirmed sub.
        let sub_idx = topic
            .subscriptions
            .iter()
            .position(|s| {
                !s.confirmed
                    && s.confirmation_token
                        .as_ref()
                        .is_some_and(|t| t == &input.token)
            })
            .or_else(|| {
                // Fallback: accept any token for any unconfirmed sub (local dev).
                topic.subscriptions.iter().position(|s| !s.confirmed)
            })
            .ok_or_else(|| {
                SnsError::not_found("No pending subscription found matching the token")
            })?;

        let sub = &mut topic.subscriptions[sub_idx];

        sub.confirmed = true;
        sub.confirmation_token = None;
        let sub_arn = sub.arn.clone();

        debug!(
            topic_arn = %input.topic_arn,
            subscription_arn = %sub_arn,
            "confirmed subscription"
        );

        Ok(ConfirmSubscriptionOutput {
            subscription_arn: Some(sub_arn),
        })
    }

    /// Handle `GetSubscriptionAttributes`.
    pub fn get_subscription_attributes(
        &self,
        input: &GetSubscriptionAttributesInput,
    ) -> Result<GetSubscriptionAttributesOutput, SnsError> {
        let topic_arn = self
            .topics
            .find_topic_for_subscription(&input.subscription_arn)
            .ok_or_else(|| SnsError::not_found("Subscription does not exist"))?;

        let topic = self
            .topics
            .get_topic(&topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        let sub = topic
            .subscriptions
            .iter()
            .find(|s| s.arn == input.subscription_arn)
            .ok_or_else(|| SnsError::not_found("Subscription does not exist"))?;

        let attributes = sub.attributes.to_map(sub);
        Ok(GetSubscriptionAttributesOutput { attributes })
    }

    /// Handle `SetSubscriptionAttributes`.
    pub fn set_subscription_attributes(
        &self,
        input: SetSubscriptionAttributesInput,
    ) -> Result<SetSubscriptionAttributesOutput, SnsError> {
        let topic_arn = self
            .topics
            .find_topic_for_subscription(&input.subscription_arn)
            .ok_or_else(|| SnsError::not_found("Subscription does not exist"))?;

        let mut topic = self
            .topics
            .get_topic_mut(&topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        let sub = topic
            .subscriptions
            .iter_mut()
            .find(|s| s.arn == input.subscription_arn)
            .ok_or_else(|| SnsError::not_found("Subscription does not exist"))?;

        let value = input.attribute_value.unwrap_or_default();
        apply_subscription_attribute(sub, &input.attribute_name, value)?;

        Ok(SetSubscriptionAttributesOutput {})
    }

    /// Handle `ListSubscriptions`.
    pub fn list_subscriptions(
        &self,
        input: &ListSubscriptionsInput,
    ) -> Result<ListSubscriptionsOutput, SnsError> {
        let all_subs = self.collect_all_subscriptions();

        let start = input
            .next_token
            .as_ref()
            .and_then(|t| t.parse::<usize>().ok())
            .unwrap_or(0);

        let page = &all_subs[start.min(all_subs.len())..];
        let (page, next_token) = if page.len() > LIST_SUBSCRIPTIONS_PAGE_SIZE {
            (
                &page[..LIST_SUBSCRIPTIONS_PAGE_SIZE],
                Some((start + LIST_SUBSCRIPTIONS_PAGE_SIZE).to_string()),
            )
        } else {
            (page, None)
        };

        Ok(ListSubscriptionsOutput {
            subscriptions: page.to_vec(),
            next_token,
        })
    }

    /// Handle `ListSubscriptionsByTopic`.
    pub fn list_subscriptions_by_topic(
        &self,
        input: &ListSubscriptionsByTopicInput,
    ) -> Result<ListSubscriptionsByTopicOutput, SnsError> {
        let topic = self
            .topics
            .get_topic(&input.topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        let all_subs: Vec<Subscription> = topic
            .subscriptions
            .iter()
            .map(sub_record_to_summary)
            .collect();

        let start = input
            .next_token
            .as_ref()
            .and_then(|t| t.parse::<usize>().ok())
            .unwrap_or(0);

        let page = &all_subs[start.min(all_subs.len())..];
        let (page, next_token) = if page.len() > LIST_SUBSCRIPTIONS_PAGE_SIZE {
            (
                &page[..LIST_SUBSCRIPTIONS_PAGE_SIZE],
                Some((start + LIST_SUBSCRIPTIONS_PAGE_SIZE).to_string()),
            )
        } else {
            (page, None)
        };

        Ok(ListSubscriptionsByTopicOutput {
            subscriptions: page.to_vec(),
            next_token,
        })
    }

    // ---- Tagging ----

    /// Handle `TagResource`.
    pub fn tag_resource(&self, input: &TagResourceInput) -> Result<TagResourceOutput, SnsError> {
        let mut topic = self
            .topics
            .get_topic_mut(&input.resource_arn)
            .ok_or_else(|| SnsError::not_found("Resource does not exist"))?;

        // Apply new tags (upsert).
        for tag in &input.tags {
            topic.tags.insert(tag.key.clone(), tag.value.clone());
        }

        // Validate tag count after merge.
        if topic.tags.len() > MAX_TAGS_PER_RESOURCE {
            // Rollback: remove the tags we just added if they were new.
            for tag in &input.tags {
                // Only remove if it would exceed the limit.
                if topic.tags.len() > MAX_TAGS_PER_RESOURCE {
                    topic.tags.remove(&tag.key);
                }
            }
            return Err(SnsError::new(
                SnsErrorCode::TagLimitExceeded,
                format!("Tag limit exceeded: maximum {MAX_TAGS_PER_RESOURCE} tags per resource"),
            ));
        }

        debug!(
            resource_arn = %input.resource_arn,
            tags_added = input.tags.len(),
            "tagged resource"
        );

        Ok(TagResourceOutput {})
    }

    /// Handle `UntagResource`.
    pub fn untag_resource(
        &self,
        input: &UntagResourceInput,
    ) -> Result<UntagResourceOutput, SnsError> {
        let mut topic = self
            .topics
            .get_topic_mut(&input.resource_arn)
            .ok_or_else(|| SnsError::not_found("Resource does not exist"))?;

        for key in &input.tag_keys {
            topic.tags.remove(key);
        }

        debug!(
            resource_arn = %input.resource_arn,
            tags_removed = input.tag_keys.len(),
            "untagged resource"
        );

        Ok(UntagResourceOutput {})
    }

    /// Handle `ListTagsForResource`.
    pub fn list_tags_for_resource(
        &self,
        input: &ListTagsForResourceInput,
    ) -> Result<ListTagsForResourceOutput, SnsError> {
        let topic = self
            .topics
            .get_topic(&input.resource_arn)
            .ok_or_else(|| SnsError::not_found("Resource does not exist"))?;

        let mut tags: Vec<Tag> = topic
            .tags
            .iter()
            .map(|(k, v)| Tag {
                key: k.clone(),
                value: v.clone(),
            })
            .collect();

        // Sort for deterministic output.
        tags.sort_by(|a, b| a.key.cmp(&b.key));

        Ok(ListTagsForResourceOutput { tags })
    }

    // ---- Publishing ----

    /// Handle `Publish`.
    pub async fn publish(&self, input: PublishInput) -> Result<PublishOutput, SnsError> {
        let topic_arn = resolve_publish_target(&input)?;
        validate_publish_message(&input)?;

        let topic = self
            .topics
            .get_topic(topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        validate_fifo_publish(&input, &topic)?;

        let message_id = uuid::Uuid::new_v4().to_string();

        // Collect confirmed subscriptions for fan-out, applying filter policies.
        let confirmed_subs: Vec<SubscriptionRecord> = topic
            .subscriptions
            .iter()
            .filter(|s| s.confirmed)
            .filter(|s| matches_filter_policy(s, &input.message_attributes, &input.message))
            .cloned()
            .collect();

        // Drop the topic reference before async fan-out to avoid holding
        // the DashMap lock across an await point.
        let region = self.config.default_region.clone();
        let host = self.config.host.clone();
        let port = self.config.port;
        let is_fifo = topic.is_fifo;
        drop(topic);

        self.fan_out(
            &confirmed_subs,
            &input,
            topic_arn,
            &message_id,
            &region,
            &host,
            port,
            is_fifo,
        )
        .await;

        let sequence_number = if is_fifo {
            Some(
                chrono::Utc::now()
                    .timestamp_nanos_opt()
                    .unwrap_or(0)
                    .to_string(),
            )
        } else {
            None
        };

        Ok(PublishOutput {
            message_id: Some(message_id),
            sequence_number,
        })
    }

    /// Handle `PublishBatch`.
    ///
    /// Publishes up to 10 messages to a topic in a single request.
    /// Each entry is processed independently; failures for one entry
    /// do not affect others.
    pub async fn publish_batch(
        &self,
        input: PublishBatchInput,
    ) -> Result<PublishBatchOutput, SnsError> {
        // Validate batch constraints.
        if input.publish_batch_request_entries.is_empty() {
            return Err(SnsError::new(
                SnsErrorCode::EmptyBatchRequest,
                "The batch request doesn't contain any entries",
            ));
        }

        if input.publish_batch_request_entries.len() > MAX_PUBLISH_BATCH_ENTRIES {
            return Err(SnsError::new(
                SnsErrorCode::TooManyEntriesInBatchRequest,
                format!(
                    "The batch request contains more entries than permissible (max {MAX_PUBLISH_BATCH_ENTRIES})"
                ),
            ));
        }

        // Check for duplicate IDs.
        let mut seen_ids = HashSet::with_capacity(input.publish_batch_request_entries.len());
        for entry in &input.publish_batch_request_entries {
            if !seen_ids.insert(&entry.id) {
                return Err(SnsError::new(
                    SnsErrorCode::BatchEntryIdsNotDistinct,
                    format!(
                        "Two or more batch entries in the request have the same Id: {}",
                        entry.id
                    ),
                ));
            }
        }

        // Verify topic exists.
        let topic = self
            .topics
            .get_topic(&input.topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        let is_fifo = topic.is_fifo;
        drop(topic);

        let mut successful = Vec::with_capacity(input.publish_batch_request_entries.len());
        let mut failed = Vec::new();

        for entry in &input.publish_batch_request_entries {
            let publish_input = PublishInput {
                topic_arn: Some(input.topic_arn.clone()),
                target_arn: None,
                phone_number: None,
                message: entry.message.clone(),
                subject: entry.subject.clone(),
                message_structure: entry.message_structure.clone(),
                message_attributes: entry.message_attributes.clone(),
                message_group_id: entry.message_group_id.clone(),
                message_deduplication_id: entry.message_deduplication_id.clone(),
            };

            match self.publish(publish_input).await {
                Ok(output) => {
                    successful.push(PublishBatchResultEntry {
                        id: entry.id.clone(),
                        message_id: output.message_id.unwrap_or_default(),
                        sequence_number: output.sequence_number,
                    });
                }
                Err(e) => {
                    failed.push(BatchResultErrorEntry {
                        id: entry.id.clone(),
                        code: e.code.code().to_owned(),
                        message: e.message,
                        sender_fault: e.code.fault() == "Sender",
                    });
                }
            }
        }

        let sequence_number = if is_fifo {
            Some(
                chrono::Utc::now()
                    .timestamp_nanos_opt()
                    .unwrap_or(0)
                    .to_string(),
            )
        } else {
            None
        };

        // Override sequence numbers for FIFO topics.
        if sequence_number.is_some() {
            for entry in &mut successful {
                if entry.sequence_number.is_none() {
                    entry.sequence_number.clone_from(&sequence_number);
                }
            }
        }

        Ok(PublishBatchOutput { successful, failed })
    }

    // ---- Internal Helpers ----

    /// Collect all subscriptions across all topics.
    fn collect_all_subscriptions(&self) -> Vec<Subscription> {
        let mut all_subs = Vec::new();
        let mut sorted_arns = self.topics.list_topics();
        sorted_arns.sort();

        for arn in &sorted_arns {
            if let Some(topic) = self.topics.get_topic(arn) {
                for sub in &topic.subscriptions {
                    all_subs.push(sub_record_to_summary(sub));
                }
            }
        }
        all_subs
    }

    /// Fan-out message delivery to all confirmed subscribers.
    #[allow(clippy::too_many_arguments)]
    async fn fan_out(
        &self,
        subs: &[SubscriptionRecord],
        input: &PublishInput,
        topic_arn: &str,
        message_id: &str,
        region: &str,
        host: &str,
        port: u16,
        is_fifo: bool,
    ) {
        for sub in subs {
            match sub.protocol {
                SubscriptionProtocol::Sqs => {
                    self.deliver_to_sqs(
                        sub, input, topic_arn, message_id, region, host, port, is_fifo,
                    )
                    .await;
                }
                _ => {
                    debug!(
                        protocol = %sub.protocol,
                        endpoint = %sub.endpoint,
                        "skipping delivery for unsupported protocol"
                    );
                }
            }
        }
    }

    /// Deliver a single message to an SQS subscriber.
    #[allow(clippy::too_many_arguments)]
    async fn deliver_to_sqs(
        &self,
        sub: &SubscriptionRecord,
        input: &PublishInput,
        topic_arn: &str,
        message_id: &str,
        region: &str,
        host: &str,
        port: u16,
        is_fifo: bool,
    ) {
        // Resolve the effective message for this subscriber's protocol.
        let effective_message =
            resolve_effective_message(&input.message, input.message_structure.as_deref(), sub);

        let body = if sub.attributes.raw_message_delivery {
            effective_message
        } else {
            let params = EnvelopeParams {
                message_id,
                topic_arn,
                subject: input.subject.as_deref(),
                message: &effective_message,
                message_attributes: &input.message_attributes,
                region,
                host,
                port,
            };
            match build_sns_envelope(&params) {
                Ok(envelope) => envelope,
                Err(e) => {
                    warn!(
                        subscription_arn = %sub.arn,
                        error = %e,
                        "failed to serialize SNS envelope"
                    );
                    return;
                }
            }
        };

        let group_id = if is_fifo {
            input.message_group_id.as_deref()
        } else {
            None
        };
        let dedup_id = if is_fifo {
            input.message_deduplication_id.as_deref()
        } else {
            None
        };

        if let Err(e) = self
            .sqs_publisher
            .send_message(&sub.endpoint, &body, group_id, dedup_id)
            .await
        {
            warn!(
                subscription_arn = %sub.arn,
                endpoint = %sub.endpoint,
                error = %e,
                "failed to deliver message to SQS"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Convert a `SubscriptionRecord` to a `Subscription` summary.
fn sub_record_to_summary(sub: &SubscriptionRecord) -> Subscription {
    Subscription {
        subscription_arn: sub.arn.clone(),
        owner: sub.owner.clone(),
        protocol: sub.protocol.as_str().to_owned(),
        endpoint: sub.endpoint.clone(),
        topic_arn: sub.topic_arn.clone(),
    }
}

/// Check whether a subscription's filter policy matches the message.
///
/// Returns `true` if the subscription has no filter policy or the message matches.
fn matches_filter_policy<S: ::std::hash::BuildHasher>(
    sub: &SubscriptionRecord,
    message_attributes: &HashMap<String, ruststack_sns_model::types::MessageAttributeValue, S>,
    message_body: &str,
) -> bool {
    let Some(ref filter_json) = sub.attributes.filter_policy else {
        return true;
    };

    match evaluate_filter_policy(
        filter_json,
        &sub.attributes.filter_policy_scope,
        message_attributes,
        message_body,
    ) {
        Ok(matches) => matches,
        Err(e) => {
            warn!(
                subscription_arn = %sub.arn,
                error = %e,
                "failed to evaluate filter policy, skipping subscriber"
            );
            false
        }
    }
}

/// Resolve the effective message for a subscriber based on `MessageStructure`.
///
/// When `message_structure == "json"`, resolves the protocol-specific message.
/// Otherwise returns the original message.
fn resolve_effective_message(
    message: &str,
    message_structure: Option<&str>,
    sub: &SubscriptionRecord,
) -> String {
    if message_structure == Some("json") {
        let protocol = sub.protocol.as_str();
        match resolve_protocol_message(message, protocol) {
            Ok(resolved) => resolved,
            Err(e) => {
                warn!(
                    subscription_arn = %sub.arn,
                    error = %e,
                    "failed to resolve protocol-specific message, using default"
                );
                // Fallback: try to get default, or use original message.
                resolve_protocol_message(message, "default").unwrap_or_else(|_| message.to_owned())
            }
        }
    } else {
        message.to_owned()
    }
}

/// Apply a single attribute update to a subscription.
fn apply_subscription_attribute(
    sub: &mut SubscriptionRecord,
    attribute_name: &str,
    value: String,
) -> Result<(), SnsError> {
    match attribute_name {
        "FilterPolicy" => {
            if value.is_empty() {
                sub.attributes.filter_policy = None;
            } else {
                // Validate JSON.
                if serde_json::from_str::<serde_json::Value>(&value).is_err() {
                    return Err(SnsError::invalid_parameter(
                        "Invalid parameter: FilterPolicy Reason: failed to parse JSON",
                    ));
                }
                sub.attributes.filter_policy = Some(value);
            }
        }
        "FilterPolicyScope" => {
            sub.attributes.filter_policy_scope = FilterPolicyScope::parse(&value);
        }
        "RawMessageDelivery" => {
            sub.attributes.raw_message_delivery = value.eq_ignore_ascii_case("true");
        }
        "RedrivePolicy" => {
            sub.attributes.redrive_policy = if value.is_empty() { None } else { Some(value) };
        }
        "DeliveryPolicy" => {
            sub.attributes.delivery_policy = if value.is_empty() { None } else { Some(value) };
        }
        "SubscriptionRoleArn" => {
            sub.attributes.subscription_role_arn =
                if value.is_empty() { None } else { Some(value) };
        }
        other => {
            return Err(SnsError::invalid_parameter(format!(
                "Invalid parameter: AttributeName Reason: Unknown attribute {other}"
            )));
        }
    }
    Ok(())
}

/// Resolve the target topic ARN from a Publish input.
fn resolve_publish_target(input: &PublishInput) -> Result<&str, SnsError> {
    input
        .topic_arn
        .as_deref()
        .or(input.target_arn.as_deref())
        .ok_or_else(|| {
            SnsError::invalid_parameter(
                "Invalid parameter: TopicArn or TargetArn Reason: one is required",
            )
        })
}

/// Validate the publish message content.
fn validate_publish_message(input: &PublishInput) -> Result<(), SnsError> {
    if input.message.len() > MAX_MESSAGE_SIZE {
        return Err(SnsError::invalid_parameter(format!(
            "Invalid parameter: Message Reason: \
             Message must be shorter than {MAX_MESSAGE_SIZE} bytes"
        )));
    }

    if input.message_structure.as_deref() == Some("json") {
        match serde_json::from_str::<serde_json::Value>(&input.message) {
            Ok(val) => {
                if val.get("default").is_none() {
                    return Err(SnsError::invalid_parameter(
                        "Invalid parameter: Message Reason: \
                         When MessageStructure is 'json', the message must contain a 'default' key",
                    ));
                }
            }
            Err(_) => {
                return Err(SnsError::invalid_parameter(
                    "Invalid parameter: Message Reason: \
                     When MessageStructure is 'json', the message must be valid JSON",
                ));
            }
        }
    }

    Ok(())
}

/// Validate FIFO-specific publish requirements.
fn validate_fifo_publish(
    input: &PublishInput,
    topic: &crate::topic::TopicRecord,
) -> Result<(), SnsError> {
    if topic.is_fifo {
        if input.message_group_id.is_none() {
            return Err(SnsError::invalid_parameter(
                "Invalid parameter: MessageGroupId Reason: \
                 The MessageGroupId parameter is required for FIFO topics",
            ));
        }
        if input.message_deduplication_id.is_none() && !topic.attributes.content_based_deduplication
        {
            return Err(SnsError::invalid_parameter(
                "Invalid parameter: MessageDeduplicationId Reason: \
                 The topic does not have ContentBasedDeduplication enabled, \
                 so MessageDeduplicationId is required",
            ));
        }
    }
    Ok(())
}

/// Validate a topic name according to AWS rules.
///
/// Topic names must be 1-256 characters and contain only alphanumeric
/// characters, hyphens, underscores, or end with `.fifo`.
fn validate_topic_name(name: &str) -> Result<(), SnsError> {
    if name.is_empty() || name.len() > 256 {
        return Err(SnsError::invalid_parameter(
            "Invalid parameter: Name Reason: \
             Topic name must be between 1 and 256 characters",
        ));
    }

    // Strip .fifo suffix for character validation.
    // Not a file extension - this is an AWS SNS FIFO topic naming convention.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    let base_name = name.strip_suffix(".fifo").unwrap_or(name);

    let valid = base_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return Err(SnsError::invalid_parameter(
            "Invalid parameter: Name Reason: \
             Topic name can only contain alphanumeric characters, \
             hyphens (-), and underscores (_)",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publisher::NoopSqsPublisher;

    fn make_provider() -> RustStackSns {
        RustStackSns::new(SnsConfig::default(), Arc::new(NoopSqsPublisher))
    }

    #[test]
    fn test_should_validate_valid_topic_name() {
        assert!(validate_topic_name("my-topic_123").is_ok());
        assert!(validate_topic_name("my-topic.fifo").is_ok());
    }

    #[test]
    fn test_should_reject_empty_topic_name() {
        assert!(validate_topic_name("").is_err());
    }

    #[test]
    fn test_should_reject_topic_name_with_special_chars() {
        assert!(validate_topic_name("my topic").is_err());
        assert!(validate_topic_name("my@topic").is_err());
    }

    #[test]
    fn test_should_create_topic() {
        let provider = make_provider();
        let output = provider
            .create_topic(CreateTopicInput {
                name: "test-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();
        assert!(output.topic_arn.contains("test-topic"));
    }

    #[test]
    fn test_should_create_topic_idempotent() {
        let provider = make_provider();
        let o1 = provider
            .create_topic(CreateTopicInput {
                name: "test-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();
        let o2 = provider
            .create_topic(CreateTopicInput {
                name: "test-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(o1.topic_arn, o2.topic_arn);
    }

    #[test]
    fn test_should_delete_topic() {
        let provider = make_provider();
        let output = provider
            .create_topic(CreateTopicInput {
                name: "to-delete".to_owned(),
                ..Default::default()
            })
            .unwrap();
        assert!(
            provider
                .delete_topic(&DeleteTopicInput {
                    topic_arn: output.topic_arn.clone(),
                })
                .is_ok()
        );
        assert!(
            provider
                .get_topic_attributes(&GetTopicAttributesInput {
                    topic_arn: output.topic_arn,
                })
                .is_err()
        );
    }

    #[test]
    fn test_should_delete_topic_idempotent() {
        let provider = make_provider();
        assert!(
            provider
                .delete_topic(&DeleteTopicInput {
                    topic_arn: "arn:aws:sns:us-east-1:000000000000:nonexistent".to_owned(),
                })
                .is_ok()
        );
    }

    #[test]
    fn test_should_list_topics() {
        let provider = make_provider();
        for i in 0..5 {
            provider
                .create_topic(CreateTopicInput {
                    name: format!("topic-{i}"),
                    ..Default::default()
                })
                .unwrap();
        }
        let output = provider
            .list_topics(&ListTopicsInput { next_token: None })
            .unwrap();
        assert_eq!(output.topics.len(), 5);
        assert!(output.next_token.is_none());
    }

    #[test]
    fn test_should_subscribe_and_list() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "sub-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        let sub = provider
            .subscribe(SubscribeInput {
                topic_arn: topic.topic_arn.clone(),
                protocol: "sqs".to_owned(),
                endpoint: Some("arn:aws:sqs:us-east-1:000000000000:test-queue".to_owned()),
                ..Default::default()
            })
            .unwrap();
        assert!(sub.subscription_arn.is_some());
        assert_ne!(sub.subscription_arn.as_deref(), Some("PendingConfirmation"));

        let list = provider
            .list_subscriptions_by_topic(&ListSubscriptionsByTopicInput {
                topic_arn: topic.topic_arn,
                next_token: None,
            })
            .unwrap();
        assert_eq!(list.subscriptions.len(), 1);
    }

    #[test]
    fn test_should_unsubscribe() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "unsub-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        let sub = provider
            .subscribe(SubscribeInput {
                topic_arn: topic.topic_arn.clone(),
                protocol: "sqs".to_owned(),
                endpoint: Some("arn:aws:sqs:us-east-1:000000000000:q".to_owned()),
                ..Default::default()
            })
            .unwrap();

        provider
            .unsubscribe(&UnsubscribeInput {
                subscription_arn: sub.subscription_arn.unwrap(),
            })
            .unwrap();

        let list = provider
            .list_subscriptions_by_topic(&ListSubscriptionsByTopicInput {
                topic_arn: topic.topic_arn,
                next_token: None,
            })
            .unwrap();
        assert!(list.subscriptions.is_empty());
    }

    #[tokio::test]
    async fn test_should_publish_message() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "pub-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        let output = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn),
                message: "Hello, world!".to_owned(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(output.message_id.is_some());
    }

    #[tokio::test]
    async fn test_should_reject_oversized_message() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "size-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        let big_message = "x".repeat(MAX_MESSAGE_SIZE + 1);
        let err = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn),
                message: big_message,
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(err.message.contains("shorter than"));
    }

    #[tokio::test]
    async fn test_should_reject_json_message_without_default() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "json-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        let err = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn),
                message: r#"{"sqs": "hello"}"#.to_owned(),
                message_structure: Some("json".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(err.message.contains("default"));
    }

    #[test]
    fn test_should_create_fifo_topic() {
        let provider = make_provider();
        let output = provider
            .create_topic(CreateTopicInput {
                name: "test.fifo".to_owned(),
                ..Default::default()
            })
            .unwrap();
        assert!(output.topic_arn.contains("test.fifo"));

        let attrs = provider
            .get_topic_attributes(&GetTopicAttributesInput {
                topic_arn: output.topic_arn,
            })
            .unwrap();
        assert_eq!(attrs.attributes.get("FifoTopic").unwrap(), "true");
    }

    // ---- Phase 1 Tests ----

    #[test]
    fn test_should_tag_and_list_tags() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "tag-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        provider
            .tag_resource(&TagResourceInput {
                resource_arn: topic.topic_arn.clone(),
                tags: vec![
                    Tag {
                        key: "env".to_owned(),
                        value: "prod".to_owned(),
                    },
                    Tag {
                        key: "team".to_owned(),
                        value: "platform".to_owned(),
                    },
                ],
            })
            .unwrap();

        let tags = provider
            .list_tags_for_resource(&ListTagsForResourceInput {
                resource_arn: topic.topic_arn,
            })
            .unwrap();
        assert_eq!(tags.tags.len(), 2);
    }

    #[test]
    fn test_should_untag_resource() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "untag-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        provider
            .tag_resource(&TagResourceInput {
                resource_arn: topic.topic_arn.clone(),
                tags: vec![
                    Tag {
                        key: "env".to_owned(),
                        value: "prod".to_owned(),
                    },
                    Tag {
                        key: "team".to_owned(),
                        value: "platform".to_owned(),
                    },
                ],
            })
            .unwrap();

        provider
            .untag_resource(&UntagResourceInput {
                resource_arn: topic.topic_arn.clone(),
                tag_keys: vec!["env".to_owned()],
            })
            .unwrap();

        let tags = provider
            .list_tags_for_resource(&ListTagsForResourceInput {
                resource_arn: topic.topic_arn,
            })
            .unwrap();
        assert_eq!(tags.tags.len(), 1);
        assert_eq!(tags.tags[0].key, "team");
    }

    #[test]
    fn test_should_confirm_subscription() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "confirm-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        // HTTP requires confirmation.
        let sub = provider
            .subscribe(SubscribeInput {
                topic_arn: topic.topic_arn.clone(),
                protocol: "http".to_owned(),
                endpoint: Some("http://example.com/webhook".to_owned()),
                return_subscription_arn: true,
                ..Default::default()
            })
            .unwrap();

        let sub_arn = sub.subscription_arn.unwrap();

        // Get subscription and verify it's pending.
        let attrs = provider
            .get_subscription_attributes(&GetSubscriptionAttributesInput {
                subscription_arn: sub_arn.clone(),
            })
            .unwrap();
        assert_eq!(attrs.attributes.get("PendingConfirmation").unwrap(), "true");

        // Confirm with a token.
        let confirm_output = provider
            .confirm_subscription(&ConfirmSubscriptionInput {
                topic_arn: topic.topic_arn.clone(),
                token: "any-valid-token".to_owned(),
                authenticate_on_unsubscribe: None,
            })
            .unwrap();
        assert_eq!(
            confirm_output.subscription_arn.as_deref(),
            Some(sub_arn.as_str())
        );

        // Verify it's now confirmed.
        let attrs = provider
            .get_subscription_attributes(&GetSubscriptionAttributesInput {
                subscription_arn: sub_arn,
            })
            .unwrap();
        assert_eq!(
            attrs.attributes.get("PendingConfirmation").unwrap(),
            "false"
        );
    }

    #[tokio::test]
    async fn test_should_publish_batch() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "batch-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        let output = provider
            .publish_batch(PublishBatchInput {
                topic_arn: topic.topic_arn,
                publish_batch_request_entries: vec![
                    ruststack_sns_model::types::PublishBatchRequestEntry {
                        id: "e1".to_owned(),
                        message: "msg1".to_owned(),
                        ..Default::default()
                    },
                    ruststack_sns_model::types::PublishBatchRequestEntry {
                        id: "e2".to_owned(),
                        message: "msg2".to_owned(),
                        ..Default::default()
                    },
                ],
            })
            .await
            .unwrap();

        assert_eq!(output.successful.len(), 2);
        assert!(output.failed.is_empty());
        assert_eq!(output.successful[0].id, "e1");
        assert_eq!(output.successful[1].id, "e2");
    }

    #[tokio::test]
    async fn test_should_reject_empty_batch() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "empty-batch-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        let err = provider
            .publish_batch(PublishBatchInput {
                topic_arn: topic.topic_arn,
                publish_batch_request_entries: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, SnsErrorCode::EmptyBatchRequest);
    }

    #[tokio::test]
    async fn test_should_reject_duplicate_batch_ids() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "dup-batch-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        let err = provider
            .publish_batch(PublishBatchInput {
                topic_arn: topic.topic_arn,
                publish_batch_request_entries: vec![
                    ruststack_sns_model::types::PublishBatchRequestEntry {
                        id: "same".to_owned(),
                        message: "msg1".to_owned(),
                        ..Default::default()
                    },
                    ruststack_sns_model::types::PublishBatchRequestEntry {
                        id: "same".to_owned(),
                        message: "msg2".to_owned(),
                        ..Default::default()
                    },
                ],
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, SnsErrorCode::BatchEntryIdsNotDistinct);
    }

    #[tokio::test]
    async fn test_should_filter_messages_by_attribute() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "filter-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        // Subscribe with a filter policy.
        let mut attrs = HashMap::new();
        attrs.insert(
            "FilterPolicy".to_owned(),
            r#"{"color": ["red"]}"#.to_owned(),
        );
        provider
            .subscribe(SubscribeInput {
                topic_arn: topic.topic_arn.clone(),
                protocol: "sqs".to_owned(),
                endpoint: Some("arn:aws:sqs:us-east-1:000000000000:filtered-queue".to_owned()),
                attributes: attrs,
                ..Default::default()
            })
            .unwrap();

        // Publish with matching attribute - should succeed (no error).
        let mut message_attrs = HashMap::new();
        message_attrs.insert(
            "color".to_owned(),
            ruststack_sns_model::types::MessageAttributeValue {
                data_type: "String".to_owned(),
                string_value: Some("red".to_owned()),
                binary_value: None,
            },
        );
        let output = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn.clone()),
                message: "Hello!".to_owned(),
                message_attributes: message_attrs,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(output.message_id.is_some());

        // Publish with non-matching attribute - should also succeed (message just gets filtered).
        let mut non_matching_attrs = HashMap::new();
        non_matching_attrs.insert(
            "color".to_owned(),
            ruststack_sns_model::types::MessageAttributeValue {
                data_type: "String".to_owned(),
                string_value: Some("green".to_owned()),
                binary_value: None,
            },
        );
        let output = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn),
                message: "Hello!".to_owned(),
                message_attributes: non_matching_attrs,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(output.message_id.is_some());
    }
}
