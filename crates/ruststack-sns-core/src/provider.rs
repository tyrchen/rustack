//! Main SNS provider implementing Phase 0, Phase 1, Phase 2, and Phase 3 operations.
//!
//! Acts as the topic manager that owns all topic state and
//! coordinates message fan-out to subscribers.
//!
//! Phase 3 adds stub implementations for platform application CRUD
//! and SMS operations. These store and retrieve data but do not
//! perform actual push notification or SMS delivery.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use dashmap::DashMap;
use ruststack_sns_model::error::{SnsError, SnsErrorCode};

use ruststack_sns_model::input::{
    AddPermissionInput, CheckIfPhoneNumberIsOptedOutInput, ConfirmSubscriptionInput,
    CreatePlatformApplicationInput, CreatePlatformEndpointInput, CreateSMSSandboxPhoneNumberInput,
    CreateTopicInput, DeleteEndpointInput, DeletePlatformApplicationInput,
    DeleteSMSSandboxPhoneNumberInput, DeleteTopicInput, GetDataProtectionPolicyInput,
    GetEndpointAttributesInput, GetPlatformApplicationAttributesInput, GetSMSAttributesInput,
    GetSMSSandboxAccountStatusInput, GetSubscriptionAttributesInput, GetTopicAttributesInput,
    ListEndpointsByPlatformApplicationInput, ListOriginationNumbersInput,
    ListPhoneNumbersOptedOutInput, ListPlatformApplicationsInput, ListSMSSandboxPhoneNumbersInput,
    ListSubscriptionsByTopicInput, ListSubscriptionsInput, ListTagsForResourceInput,
    ListTopicsInput, OptInPhoneNumberInput, PublishBatchInput, PublishInput,
    PutDataProtectionPolicyInput, RemovePermissionInput, SetEndpointAttributesInput,
    SetPlatformApplicationAttributesInput, SetSMSAttributesInput, SetSubscriptionAttributesInput,
    SetTopicAttributesInput, SubscribeInput, TagResourceInput, UnsubscribeInput,
    UntagResourceInput, VerifySMSSandboxPhoneNumberInput,
};
use ruststack_sns_model::output::{
    AddPermissionOutput, CheckIfPhoneNumberIsOptedOutOutput, ConfirmSubscriptionOutput,
    CreatePlatformApplicationOutput, CreatePlatformEndpointOutput,
    CreateSMSSandboxPhoneNumberOutput, CreateTopicOutput, DeleteEndpointOutput,
    DeletePlatformApplicationOutput, DeleteSMSSandboxPhoneNumberOutput, DeleteTopicOutput,
    GetDataProtectionPolicyOutput, GetEndpointAttributesOutput,
    GetPlatformApplicationAttributesOutput, GetSMSAttributesOutput,
    GetSMSSandboxAccountStatusOutput, GetSubscriptionAttributesOutput, GetTopicAttributesOutput,
    ListEndpointsByPlatformApplicationOutput, ListOriginationNumbersOutput,
    ListPhoneNumbersOptedOutOutput, ListPlatformApplicationsOutput,
    ListSMSSandboxPhoneNumbersOutput, ListSubscriptionsByTopicOutput, ListSubscriptionsOutput,
    ListTagsForResourceOutput, ListTopicsOutput, OptInPhoneNumberOutput, PublishBatchOutput,
    PublishOutput, PutDataProtectionPolicyOutput, RemovePermissionOutput,
    SetEndpointAttributesOutput, SetPlatformApplicationAttributesOutput, SetSMSAttributesOutput,
    SetSubscriptionAttributesOutput, SetTopicAttributesOutput, SubscribeOutput, TagResourceOutput,
    UnsubscribeOutput, UntagResourceOutput, VerifySMSSandboxPhoneNumberOutput,
};
use ruststack_sns_model::types::{
    BatchResultErrorEntry, Endpoint, PlatformApplication, PublishBatchResultEntry,
    SMSSandboxPhoneNumber, Subscription, Tag, Topic,
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

/// Maximum number of platform applications returned per list page.
const LIST_PLATFORM_APPS_PAGE_SIZE: usize = 100;

/// Maximum number of platform endpoints returned per list page.
const LIST_ENDPOINTS_PAGE_SIZE: usize = 100;

/// Internal record for a platform application.
#[derive(Debug, Clone)]
struct PlatformApplicationRecord {
    /// The platform application ARN.
    arn: String,
    /// The application name.
    name: String,
    /// The platform (e.g., `APNS`, `GCM`).
    platform: String,
    /// Application attributes.
    attributes: HashMap<String, String>,
}

/// Internal record for a platform endpoint.
#[derive(Debug, Clone)]
struct PlatformEndpointRecord {
    /// The endpoint ARN.
    arn: String,
    /// The parent platform application ARN.
    platform_application_arn: String,
    /// Endpoint attributes (includes `Enabled`, `Token`, `CustomUserData`).
    attributes: HashMap<String, String>,
}

/// Internal record for an SMS sandbox phone number.
#[derive(Debug, Clone)]
struct SandboxPhoneRecord {
    /// The phone number.
    phone_number: String,
    /// Verification status: `Pending` or `Verified`.
    status: String,
}

/// Main SNS provider.
pub struct RustStackSns {
    /// Topic store.
    topics: TopicStore,
    /// SQS publisher for fan-out delivery.
    sqs_publisher: Arc<dyn SqsPublisher>,
    /// Service configuration.
    config: Arc<SnsConfig>,
    /// Platform applications keyed by ARN.
    platform_apps: DashMap<String, PlatformApplicationRecord>,
    /// Platform endpoints keyed by ARN.
    platform_endpoints: DashMap<String, PlatformEndpointRecord>,
    /// SMS attributes (global settings).
    sms_attributes: parking_lot::RwLock<HashMap<String, String>>,
    /// Phone numbers that have opted out of SMS.
    opted_out_numbers: parking_lot::RwLock<HashSet<String>>,
    /// SMS sandbox phone numbers keyed by phone number.
    sandbox_phones: DashMap<String, SandboxPhoneRecord>,
}

impl fmt::Debug for RustStackSns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RustStackSns")
            .field("topics", &self.topics)
            .field("sqs_publisher", &"<dyn SqsPublisher>")
            .field("config", &self.config)
            .field("platform_apps", &self.platform_apps.len())
            .field("platform_endpoints", &self.platform_endpoints.len())
            .field("sms_attributes", &self.sms_attributes.read().len())
            .field("opted_out_numbers", &self.opted_out_numbers.read().len())
            .field("sandbox_phones", &self.sandbox_phones.len())
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
            platform_apps: DashMap::new(),
            platform_endpoints: DashMap::new(),
            sms_attributes: parking_lot::RwLock::new(HashMap::new()),
            opted_out_numbers: parking_lot::RwLock::new(HashSet::new()),
            sandbox_phones: DashMap::new(),
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
            fifo_sequence_counter: AtomicU64::new(0),
            fifo_dedup_cache: HashMap::new(),
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
        let topic_arn = resolve_publish_target(&input)?.to_owned();
        validate_publish_message(&input)?;

        // Use a mutable reference so we can do FIFO dedup bookkeeping.
        let mut topic = self
            .topics
            .get_topic_mut(&topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        validate_fifo_publish(&input, &topic)?;

        let is_fifo = topic.is_fifo;

        // FIFO deduplication: compute dedup ID if content-based, then check cache.
        let effective_dedup_id = if is_fifo {
            let dedup_id = input.message_deduplication_id.clone().or_else(|| {
                if topic.attributes.content_based_deduplication {
                    Some(compute_sha256(&input.message))
                } else {
                    None
                }
            });

            if let Some(ref id) = dedup_id {
                if topic.check_and_record_dedup(id) {
                    // Duplicate within the 5-minute window -- return success without fan-out.
                    let sequence_number = topic.next_sequence_number();
                    drop(topic);
                    return Ok(PublishOutput {
                        message_id: Some(uuid::Uuid::new_v4().to_string()),
                        sequence_number: Some(sequence_number),
                    });
                }
            }

            dedup_id
        } else {
            None
        };

        let message_id = uuid::Uuid::new_v4().to_string();

        let sequence_number = if is_fifo {
            Some(topic.next_sequence_number())
        } else {
            None
        };

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
        drop(topic);

        // For FIFO fan-out, pass the effective dedup ID to SQS.
        let fan_out_input =
            if effective_dedup_id.is_some() && input.message_deduplication_id.is_none() {
                // Content-based dedup: create a modified input with the computed dedup ID.
                let mut modified = input;
                modified.message_deduplication_id = effective_dedup_id;
                modified
            } else {
                input
            };

        self.fan_out(
            &confirmed_subs,
            &fan_out_input,
            &topic_arn,
            &message_id,
            &region,
            &host,
            port,
            is_fifo,
        )
        .await;

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

        Ok(PublishBatchOutput { successful, failed })
    }

    // ---- Permissions ----

    /// Handle `AddPermission`.
    ///
    /// Builds an IAM-like policy statement and merges it into the topic's
    /// `Policy` attribute. No enforcement is performed.
    pub fn add_permission(
        &self,
        input: &AddPermissionInput,
    ) -> Result<AddPermissionOutput, SnsError> {
        let mut topic = self
            .topics
            .get_topic_mut(&input.topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        let mut policy: serde_json::Value = topic
            .attributes
            .policy
            .as_deref()
            .and_then(|p| serde_json::from_str(p).ok())
            .unwrap_or_else(|| default_policy(&input.topic_arn, &topic.attributes.owner));

        let statement = build_permission_statement(
            &input.label,
            &input.topic_arn,
            &input.aws_account_id,
            &input.action_name,
        );

        if let Some(statements) = policy.get_mut("Statement").and_then(|s| s.as_array_mut()) {
            statements.push(statement);
        }

        topic.attributes.policy = Some(policy.to_string());

        debug!(
            topic_arn = %input.topic_arn,
            label = %input.label,
            "added permission"
        );

        Ok(AddPermissionOutput {})
    }

    /// Handle `RemovePermission`.
    ///
    /// Removes the policy statement with the given label (Sid) from the
    /// topic's `Policy` attribute.
    pub fn remove_permission(
        &self,
        input: &RemovePermissionInput,
    ) -> Result<RemovePermissionOutput, SnsError> {
        let mut topic = self
            .topics
            .get_topic_mut(&input.topic_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        if let Some(ref policy_str) = topic.attributes.policy {
            if let Ok(mut policy) = serde_json::from_str::<serde_json::Value>(policy_str) {
                if let Some(statements) = policy.get_mut("Statement").and_then(|s| s.as_array_mut())
                {
                    statements.retain(|stmt| {
                        stmt.get("Sid").and_then(|s| s.as_str()) != Some(&input.label)
                    });
                }
                topic.attributes.policy = Some(policy.to_string());
            }
        }

        debug!(
            topic_arn = %input.topic_arn,
            label = %input.label,
            "removed permission"
        );

        Ok(RemovePermissionOutput {})
    }

    // ---- Data Protection ----

    /// Handle `GetDataProtectionPolicy`.
    pub fn get_data_protection_policy(
        &self,
        input: &GetDataProtectionPolicyInput,
    ) -> Result<GetDataProtectionPolicyOutput, SnsError> {
        let topic = self
            .topics
            .get_topic(&input.resource_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        Ok(GetDataProtectionPolicyOutput {
            data_protection_policy: topic.data_protection_policy.clone(),
        })
    }

    /// Handle `PutDataProtectionPolicy`.
    pub fn put_data_protection_policy(
        &self,
        input: &PutDataProtectionPolicyInput,
    ) -> Result<PutDataProtectionPolicyOutput, SnsError> {
        let mut topic = self
            .topics
            .get_topic_mut(&input.resource_arn)
            .ok_or_else(|| SnsError::not_found("Topic does not exist"))?;

        topic.data_protection_policy = if input.data_protection_policy.is_empty() {
            None
        } else {
            Some(input.data_protection_policy.clone())
        };

        debug!(
            resource_arn = %input.resource_arn,
            "updated data protection policy"
        );

        Ok(PutDataProtectionPolicyOutput {})
    }

    // ---- Platform Applications ----

    /// Handle `CreatePlatformApplication`.
    ///
    /// Stores a new platform application record. Does not perform
    /// actual platform registration.
    pub fn create_platform_application(
        &self,
        input: CreatePlatformApplicationInput,
    ) -> Result<CreatePlatformApplicationOutput, SnsError> {
        let arn = format!(
            "arn:aws:sns:{}:{}:app/{}/{}",
            self.config.default_region, self.config.account_id, input.platform, input.name
        );

        let record = PlatformApplicationRecord {
            arn: arn.clone(),
            name: input.name,
            platform: input.platform,
            attributes: input.attributes,
        };

        self.platform_apps.insert(arn.clone(), record);
        debug!(platform_application_arn = %arn, "created platform application");

        Ok(CreatePlatformApplicationOutput {
            platform_application_arn: arn,
        })
    }

    /// Handle `DeletePlatformApplication`.
    ///
    /// Removes the platform application and all associated endpoints.
    pub fn delete_platform_application(
        &self,
        input: &DeletePlatformApplicationInput,
    ) -> Result<DeletePlatformApplicationOutput, SnsError> {
        self.platform_apps.remove(&input.platform_application_arn);

        // Remove all endpoints belonging to this platform application.
        let endpoints_to_remove: Vec<String> = self
            .platform_endpoints
            .iter()
            .filter(|e| e.platform_application_arn == input.platform_application_arn)
            .map(|e| e.key().clone())
            .collect();

        for arn in &endpoints_to_remove {
            self.platform_endpoints.remove(arn);
        }

        debug!(
            platform_application_arn = %input.platform_application_arn,
            endpoints_removed = endpoints_to_remove.len(),
            "deleted platform application"
        );

        Ok(DeletePlatformApplicationOutput {})
    }

    /// Handle `GetPlatformApplicationAttributes`.
    pub fn get_platform_application_attributes(
        &self,
        input: &GetPlatformApplicationAttributesInput,
    ) -> Result<GetPlatformApplicationAttributesOutput, SnsError> {
        let app = self
            .platform_apps
            .get(&input.platform_application_arn)
            .ok_or_else(|| SnsError::not_found("PlatformApplication does not exist"))?;

        Ok(GetPlatformApplicationAttributesOutput {
            attributes: app.attributes.clone(),
        })
    }

    /// Handle `SetPlatformApplicationAttributes`.
    pub fn set_platform_application_attributes(
        &self,
        input: SetPlatformApplicationAttributesInput,
    ) -> Result<SetPlatformApplicationAttributesOutput, SnsError> {
        let mut app = self
            .platform_apps
            .get_mut(&input.platform_application_arn)
            .ok_or_else(|| SnsError::not_found("PlatformApplication does not exist"))?;

        for (k, v) in input.attributes {
            app.attributes.insert(k, v);
        }

        debug!(
            platform_application_arn = %input.platform_application_arn,
            "updated platform application attributes"
        );

        Ok(SetPlatformApplicationAttributesOutput {})
    }

    /// Handle `ListPlatformApplications`.
    pub fn list_platform_applications(
        &self,
        input: &ListPlatformApplicationsInput,
    ) -> Result<ListPlatformApplicationsOutput, SnsError> {
        let mut apps: Vec<PlatformApplication> = self
            .platform_apps
            .iter()
            .map(|r| PlatformApplication {
                platform_application_arn: r.arn.clone(),
                attributes: r.attributes.clone(),
            })
            .collect();

        // Sort for deterministic output.
        apps.sort_by(|a, b| a.platform_application_arn.cmp(&b.platform_application_arn));

        let start = input
            .next_token
            .as_ref()
            .and_then(|t| t.parse::<usize>().ok())
            .unwrap_or(0);

        let page = &apps[start.min(apps.len())..];
        let (page, next_token) = if page.len() > LIST_PLATFORM_APPS_PAGE_SIZE {
            (
                &page[..LIST_PLATFORM_APPS_PAGE_SIZE],
                Some((start + LIST_PLATFORM_APPS_PAGE_SIZE).to_string()),
            )
        } else {
            (page, None)
        };

        Ok(ListPlatformApplicationsOutput {
            platform_applications: page.to_vec(),
            next_token,
        })
    }

    // ---- Platform Endpoints ----

    /// Handle `CreatePlatformEndpoint`.
    ///
    /// Creates a new platform endpoint. Does not register with actual push services.
    pub fn create_platform_endpoint(
        &self,
        input: CreatePlatformEndpointInput,
    ) -> Result<CreatePlatformEndpointOutput, SnsError> {
        // Verify the platform application exists.
        let app = self
            .platform_apps
            .get(&input.platform_application_arn)
            .ok_or_else(|| SnsError::not_found("PlatformApplication does not exist"))?;

        let endpoint_id = uuid::Uuid::new_v4();
        let arn = format!(
            "arn:aws:sns:{}:{}:endpoint/{}/{}/{}",
            self.config.default_region, self.config.account_id, app.platform, app.name, endpoint_id
        );

        let mut attributes = input.attributes;
        attributes.insert("Token".to_owned(), input.token.clone());
        attributes
            .entry("Enabled".to_owned())
            .or_insert_with(|| "true".to_owned());
        if let Some(ref custom_data) = input.custom_user_data {
            attributes.insert("CustomUserData".to_owned(), custom_data.clone());
        }

        let record = PlatformEndpointRecord {
            arn: arn.clone(),
            platform_application_arn: input.platform_application_arn,
            attributes,
        };

        self.platform_endpoints.insert(arn.clone(), record);
        debug!(endpoint_arn = %arn, "created platform endpoint");

        Ok(CreatePlatformEndpointOutput { endpoint_arn: arn })
    }

    /// Handle `DeleteEndpoint`.
    pub fn delete_endpoint(
        &self,
        input: &DeleteEndpointInput,
    ) -> Result<DeleteEndpointOutput, SnsError> {
        // Idempotent: no error if endpoint does not exist.
        self.platform_endpoints.remove(&input.endpoint_arn);
        debug!(endpoint_arn = %input.endpoint_arn, "deleted endpoint");
        Ok(DeleteEndpointOutput {})
    }

    /// Handle `GetEndpointAttributes`.
    pub fn get_endpoint_attributes(
        &self,
        input: &GetEndpointAttributesInput,
    ) -> Result<GetEndpointAttributesOutput, SnsError> {
        let endpoint = self
            .platform_endpoints
            .get(&input.endpoint_arn)
            .ok_or_else(|| SnsError::not_found("Endpoint does not exist"))?;

        Ok(GetEndpointAttributesOutput {
            attributes: endpoint.attributes.clone(),
        })
    }

    /// Handle `SetEndpointAttributes`.
    pub fn set_endpoint_attributes(
        &self,
        input: SetEndpointAttributesInput,
    ) -> Result<SetEndpointAttributesOutput, SnsError> {
        let mut endpoint = self
            .platform_endpoints
            .get_mut(&input.endpoint_arn)
            .ok_or_else(|| SnsError::not_found("Endpoint does not exist"))?;

        for (k, v) in input.attributes {
            endpoint.attributes.insert(k, v);
        }

        debug!(endpoint_arn = %input.endpoint_arn, "updated endpoint attributes");
        Ok(SetEndpointAttributesOutput {})
    }

    /// Handle `ListEndpointsByPlatformApplication`.
    pub fn list_endpoints_by_platform_application(
        &self,
        input: &ListEndpointsByPlatformApplicationInput,
    ) -> Result<ListEndpointsByPlatformApplicationOutput, SnsError> {
        // Verify the platform application exists.
        if !self
            .platform_apps
            .contains_key(&input.platform_application_arn)
        {
            return Err(SnsError::not_found("PlatformApplication does not exist"));
        }

        let mut endpoints: Vec<Endpoint> = self
            .platform_endpoints
            .iter()
            .filter(|e| e.platform_application_arn == input.platform_application_arn)
            .map(|e| Endpoint {
                endpoint_arn: e.arn.clone(),
                attributes: e.attributes.clone(),
            })
            .collect();

        // Sort for deterministic output.
        endpoints.sort_by(|a, b| a.endpoint_arn.cmp(&b.endpoint_arn));

        let start = input
            .next_token
            .as_ref()
            .and_then(|t| t.parse::<usize>().ok())
            .unwrap_or(0);

        let page = &endpoints[start.min(endpoints.len())..];
        let (page, next_token) = if page.len() > LIST_ENDPOINTS_PAGE_SIZE {
            (
                &page[..LIST_ENDPOINTS_PAGE_SIZE],
                Some((start + LIST_ENDPOINTS_PAGE_SIZE).to_string()),
            )
        } else {
            (page, None)
        };

        Ok(ListEndpointsByPlatformApplicationOutput {
            endpoints: page.to_vec(),
            next_token,
        })
    }

    // ---- SMS Operations ----

    /// Handle `CheckIfPhoneNumberIsOptedOut`.
    pub fn check_if_phone_number_is_opted_out(
        &self,
        input: &CheckIfPhoneNumberIsOptedOutInput,
    ) -> Result<CheckIfPhoneNumberIsOptedOutOutput, SnsError> {
        let opted_out = self.opted_out_numbers.read();
        let is_opted_out = opted_out.contains(&input.phone_number);
        Ok(CheckIfPhoneNumberIsOptedOutOutput { is_opted_out })
    }

    /// Handle `GetSMSAttributes`.
    pub fn get_sms_attributes(
        &self,
        input: &GetSMSAttributesInput,
    ) -> Result<GetSMSAttributesOutput, SnsError> {
        let all_attrs = self.sms_attributes.read();

        let attributes = if input.attributes.is_empty() {
            all_attrs.clone()
        } else {
            input
                .attributes
                .iter()
                .filter_map(|k| all_attrs.get(k).map(|v| (k.clone(), v.clone())))
                .collect()
        };

        Ok(GetSMSAttributesOutput { attributes })
    }

    /// Handle `SetSMSAttributes`.
    pub fn set_sms_attributes(
        &self,
        input: SetSMSAttributesInput,
    ) -> Result<SetSMSAttributesOutput, SnsError> {
        let mut attrs = self.sms_attributes.write();
        for (k, v) in input.attributes {
            attrs.insert(k, v);
        }
        debug!("updated SMS attributes");
        Ok(SetSMSAttributesOutput {})
    }

    /// Handle `ListPhoneNumbersOptedOut`.
    pub fn list_phone_numbers_opted_out(
        &self,
        _input: &ListPhoneNumbersOptedOutInput,
    ) -> Result<ListPhoneNumbersOptedOutOutput, SnsError> {
        let opted_out = self.opted_out_numbers.read();
        let mut phone_numbers: Vec<String> = opted_out.iter().cloned().collect();
        phone_numbers.sort();

        Ok(ListPhoneNumbersOptedOutOutput {
            phone_numbers,
            next_token: None,
        })
    }

    /// Handle `OptInPhoneNumber`.
    ///
    /// Removes the phone number from the opted-out list.
    pub fn opt_in_phone_number(
        &self,
        input: &OptInPhoneNumberInput,
    ) -> Result<OptInPhoneNumberOutput, SnsError> {
        let mut opted_out = self.opted_out_numbers.write();
        opted_out.remove(&input.phone_number);
        debug!(phone_number = %input.phone_number, "opted in phone number");
        Ok(OptInPhoneNumberOutput {})
    }

    /// Handle `GetSMSSandboxAccountStatus`.
    ///
    /// Always returns `is_in_sandbox = true` for local development.
    pub fn get_sms_sandbox_account_status(
        &self,
        _input: &GetSMSSandboxAccountStatusInput,
    ) -> Result<GetSMSSandboxAccountStatusOutput, SnsError> {
        Ok(GetSMSSandboxAccountStatusOutput {
            is_in_sandbox: true,
        })
    }

    /// Handle `CreateSMSSandboxPhoneNumber`.
    pub fn create_sms_sandbox_phone_number(
        &self,
        input: &CreateSMSSandboxPhoneNumberInput,
    ) -> Result<CreateSMSSandboxPhoneNumberOutput, SnsError> {
        let record = SandboxPhoneRecord {
            phone_number: input.phone_number.clone(),
            status: "Pending".to_owned(),
        };

        self.sandbox_phones
            .insert(input.phone_number.clone(), record);
        debug!(phone_number = %input.phone_number, "created SMS sandbox phone number");

        Ok(CreateSMSSandboxPhoneNumberOutput {})
    }

    /// Handle `DeleteSMSSandboxPhoneNumber`.
    pub fn delete_sms_sandbox_phone_number(
        &self,
        input: &DeleteSMSSandboxPhoneNumberInput,
    ) -> Result<DeleteSMSSandboxPhoneNumberOutput, SnsError> {
        if self.sandbox_phones.remove(&input.phone_number).is_none() {
            return Err(SnsError::not_found(
                "The sandbox phone number does not exist",
            ));
        }
        debug!(phone_number = %input.phone_number, "deleted SMS sandbox phone number");
        Ok(DeleteSMSSandboxPhoneNumberOutput {})
    }

    /// Handle `VerifySMSSandboxPhoneNumber`.
    ///
    /// For local development, accepts any OTP and marks the phone as `Verified`.
    pub fn verify_sms_sandbox_phone_number(
        &self,
        input: &VerifySMSSandboxPhoneNumberInput,
    ) -> Result<VerifySMSSandboxPhoneNumberOutput, SnsError> {
        let mut phone = self
            .sandbox_phones
            .get_mut(&input.phone_number)
            .ok_or_else(|| SnsError::not_found("The sandbox phone number does not exist"))?;

        "Verified".clone_into(&mut phone.status);
        debug!(phone_number = %input.phone_number, "verified SMS sandbox phone number");

        Ok(VerifySMSSandboxPhoneNumberOutput {})
    }

    /// Handle `ListSMSSandboxPhoneNumbers`.
    pub fn list_sms_sandbox_phone_numbers(
        &self,
        _input: &ListSMSSandboxPhoneNumbersInput,
    ) -> Result<ListSMSSandboxPhoneNumbersOutput, SnsError> {
        let mut phone_numbers: Vec<SMSSandboxPhoneNumber> = self
            .sandbox_phones
            .iter()
            .map(|r| SMSSandboxPhoneNumber {
                phone_number: r.phone_number.clone(),
                status: r.status.clone(),
            })
            .collect();

        phone_numbers.sort_by(|a, b| a.phone_number.cmp(&b.phone_number));

        Ok(ListSMSSandboxPhoneNumbersOutput {
            phone_numbers,
            next_token: None,
        })
    }

    /// Handle `ListOriginationNumbers`.
    ///
    /// Returns an empty list (stub -- no origination numbers in local dev).
    pub fn list_origination_numbers(
        &self,
        _input: &ListOriginationNumbersInput,
    ) -> Result<ListOriginationNumbersOutput, SnsError> {
        Ok(ListOriginationNumbersOutput {
            phone_numbers: Vec::new(),
            next_token: None,
        })
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

/// Compute the SHA-256 hex digest of a message for content-based deduplication.
fn compute_sha256(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    // Format as lowercase hex string.
    result
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// Build a default SNS policy document.
fn default_policy(topic_arn: &str, owner: &str) -> serde_json::Value {
    serde_json::json!({
        "Version": "2012-10-17",
        "Id": format!("{topic_arn}/__default_policy_ID"),
        "Statement": [
            {
                "Sid": "__default_statement_ID",
                "Effect": "Allow",
                "Principal": { "AWS": "*" },
                "Action": [
                    "SNS:GetTopicAttributes",
                    "SNS:SetTopicAttributes",
                    "SNS:AddPermission",
                    "SNS:RemovePermission",
                    "SNS:DeleteTopic",
                    "SNS:Subscribe",
                    "SNS:ListSubscriptionsByTopic",
                    "SNS:Publish"
                ],
                "Resource": topic_arn,
                "Condition": {
                    "StringEquals": {
                        "AWS:SourceOwner": owner
                    }
                }
            }
        ]
    })
}

/// Build a permission statement for `AddPermission`.
fn build_permission_statement(
    label: &str,
    topic_arn: &str,
    account_ids: &[String],
    actions: &[String],
) -> serde_json::Value {
    let principals: Vec<String> = account_ids
        .iter()
        .map(|id| format!("arn:aws:iam::{id}:root"))
        .collect();

    let sns_actions: Vec<String> = actions.iter().map(|a| format!("SNS:{a}")).collect();

    serde_json::json!({
        "Sid": label,
        "Effect": "Allow",
        "Principal": { "AWS": principals },
        "Action": sns_actions,
        "Resource": topic_arn,
    })
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

    // ---- Phase 2 Tests ----

    #[tokio::test]
    async fn test_should_publish_to_fifo_topic_with_dedup_id() {
        let provider = make_provider();
        let mut attrs = HashMap::new();
        attrs.insert("FifoTopic".to_owned(), "true".to_owned());
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "fifo-pub.fifo".to_owned(),
                attributes: attrs,
                ..Default::default()
            })
            .unwrap();

        let output = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn),
                message: "Hello FIFO".to_owned(),
                message_group_id: Some("group1".to_owned()),
                message_deduplication_id: Some("dedup1".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(output.message_id.is_some());
        assert!(output.sequence_number.is_some());
    }

    #[tokio::test]
    async fn test_should_reject_fifo_publish_without_group_id() {
        let provider = make_provider();
        let mut attrs = HashMap::new();
        attrs.insert("FifoTopic".to_owned(), "true".to_owned());
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "fifo-nogroup.fifo".to_owned(),
                attributes: attrs,
                ..Default::default()
            })
            .unwrap();

        let err = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn),
                message: "Hello".to_owned(),
                message_deduplication_id: Some("dedup1".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(err.message.contains("MessageGroupId"));
    }

    #[tokio::test]
    async fn test_should_auto_generate_dedup_id_for_content_based_dedup() {
        let provider = make_provider();
        let mut attrs = HashMap::new();
        attrs.insert("FifoTopic".to_owned(), "true".to_owned());
        attrs.insert("ContentBasedDeduplication".to_owned(), "true".to_owned());
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "fifo-cbd.fifo".to_owned(),
                attributes: attrs,
                ..Default::default()
            })
            .unwrap();

        // Should succeed without explicit dedup ID.
        let output = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn),
                message: "Hello CBD".to_owned(),
                message_group_id: Some("group1".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(output.message_id.is_some());
        assert!(output.sequence_number.is_some());
    }

    #[tokio::test]
    async fn test_should_deduplicate_fifo_messages() {
        let provider = make_provider();
        let mut attrs = HashMap::new();
        attrs.insert("FifoTopic".to_owned(), "true".to_owned());
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "fifo-dedup.fifo".to_owned(),
                attributes: attrs,
                ..Default::default()
            })
            .unwrap();

        // First publish should succeed with sequence 0.
        let o1 = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn.clone()),
                message: "Msg".to_owned(),
                message_group_id: Some("g1".to_owned()),
                message_deduplication_id: Some("same-dedup".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();

        // Second publish with same dedup ID should be deduplicated.
        let o2 = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn),
                message: "Msg".to_owned(),
                message_group_id: Some("g1".to_owned()),
                message_deduplication_id: Some("same-dedup".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();

        // Both should succeed (deduplicated messages return success).
        assert!(o1.message_id.is_some());
        assert!(o2.message_id.is_some());
        // But they should have different message IDs.
        assert_ne!(o1.message_id, o2.message_id);
        // Both should have sequence numbers.
        assert!(o1.sequence_number.is_some());
        assert!(o2.sequence_number.is_some());
    }

    #[tokio::test]
    async fn test_should_assign_incrementing_sequence_numbers() {
        let provider = make_provider();
        let mut attrs = HashMap::new();
        attrs.insert("FifoTopic".to_owned(), "true".to_owned());
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "fifo-seq.fifo".to_owned(),
                attributes: attrs,
                ..Default::default()
            })
            .unwrap();

        let o1 = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn.clone()),
                message: "Msg1".to_owned(),
                message_group_id: Some("g1".to_owned()),
                message_deduplication_id: Some("d1".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();

        let o2 = provider
            .publish(PublishInput {
                topic_arn: Some(topic.topic_arn),
                message: "Msg2".to_owned(),
                message_group_id: Some("g1".to_owned()),
                message_deduplication_id: Some("d2".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();

        let seq1: u64 = o1
            .sequence_number
            .as_ref()
            .map_or(0, |s| s.parse().unwrap_or(0));
        let seq2: u64 = o2
            .sequence_number
            .as_ref()
            .map_or(0, |s| s.parse().unwrap_or(0));
        assert!(seq2 > seq1, "sequence numbers should be incrementing");
    }

    #[test]
    fn test_should_add_and_remove_permission() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "perm-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        // Add a permission.
        provider
            .add_permission(&AddPermissionInput {
                topic_arn: topic.topic_arn.clone(),
                label: "my-perm".to_owned(),
                aws_account_id: vec!["123456789012".to_owned()],
                action_name: vec!["Publish".to_owned(), "Subscribe".to_owned()],
            })
            .unwrap();

        // Verify policy was stored.
        let attrs = provider
            .get_topic_attributes(&GetTopicAttributesInput {
                topic_arn: topic.topic_arn.clone(),
            })
            .unwrap();
        let policy_str = attrs.attributes.get("Policy").expect("Policy should exist");
        let policy: serde_json::Value = serde_json::from_str(policy_str).unwrap();
        let stmts = policy["Statement"].as_array().unwrap();
        assert!(stmts.iter().any(|s| s["Sid"] == "my-perm"));

        // Remove it.
        provider
            .remove_permission(&RemovePermissionInput {
                topic_arn: topic.topic_arn.clone(),
                label: "my-perm".to_owned(),
            })
            .unwrap();

        // Verify it was removed.
        let attrs = provider
            .get_topic_attributes(&GetTopicAttributesInput {
                topic_arn: topic.topic_arn,
            })
            .unwrap();
        let policy_str = attrs.attributes.get("Policy").expect("Policy should exist");
        let policy: serde_json::Value = serde_json::from_str(policy_str).unwrap();
        let stmts = policy["Statement"].as_array().unwrap();
        assert!(!stmts.iter().any(|s| s["Sid"] == "my-perm"));
    }

    #[test]
    fn test_should_add_permission_to_nonexistent_topic_fails() {
        let provider = make_provider();
        let err = provider
            .add_permission(&AddPermissionInput {
                topic_arn: "arn:aws:sns:us-east-1:000000000000:nope".to_owned(),
                label: "perm".to_owned(),
                aws_account_id: vec!["111111111111".to_owned()],
                action_name: vec!["Publish".to_owned()],
            })
            .unwrap_err();
        assert_eq!(err.code, SnsErrorCode::NotFound);
    }

    #[test]
    fn test_should_put_and_get_data_protection_policy() {
        let provider = make_provider();
        let topic = provider
            .create_topic(CreateTopicInput {
                name: "dpp-topic".to_owned(),
                ..Default::default()
            })
            .unwrap();

        // Initially empty.
        let output = provider
            .get_data_protection_policy(&GetDataProtectionPolicyInput {
                resource_arn: topic.topic_arn.clone(),
            })
            .unwrap();
        assert!(output.data_protection_policy.is_none());

        // Put a policy.
        let policy_json = r#"{"Name":"data-protection","Statement":[]}"#;
        provider
            .put_data_protection_policy(&PutDataProtectionPolicyInput {
                resource_arn: topic.topic_arn.clone(),
                data_protection_policy: policy_json.to_owned(),
            })
            .unwrap();

        // Get it back.
        let output = provider
            .get_data_protection_policy(&GetDataProtectionPolicyInput {
                resource_arn: topic.topic_arn.clone(),
            })
            .unwrap();
        assert_eq!(output.data_protection_policy.as_deref(), Some(policy_json));

        // Clear it.
        provider
            .put_data_protection_policy(&PutDataProtectionPolicyInput {
                resource_arn: topic.topic_arn.clone(),
                data_protection_policy: String::new(),
            })
            .unwrap();
        let output = provider
            .get_data_protection_policy(&GetDataProtectionPolicyInput {
                resource_arn: topic.topic_arn,
            })
            .unwrap();
        assert!(output.data_protection_policy.is_none());
    }

    #[test]
    fn test_should_compute_sha256() {
        let hash = compute_sha256("hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    // ---- Phase 3 Tests: Platform Applications ----

    #[test]
    fn test_should_create_platform_application() {
        let provider = make_provider();
        let mut attrs = HashMap::new();
        attrs.insert("PlatformCredential".to_owned(), "server-api-key".to_owned());

        let output = provider
            .create_platform_application(CreatePlatformApplicationInput {
                name: "MyApp".to_owned(),
                platform: "GCM".to_owned(),
                attributes: attrs,
            })
            .unwrap();

        assert!(output.platform_application_arn.contains("app/GCM/MyApp"));
        assert!(output.platform_application_arn.starts_with("arn:aws:sns:"));
    }

    #[test]
    fn test_should_delete_platform_application() {
        let provider = make_provider();
        let output = provider
            .create_platform_application(CreatePlatformApplicationInput {
                name: "ToDelete".to_owned(),
                platform: "APNS".to_owned(),
                attributes: HashMap::new(),
            })
            .unwrap();

        provider
            .delete_platform_application(&DeletePlatformApplicationInput {
                platform_application_arn: output.platform_application_arn.clone(),
            })
            .unwrap();

        // Getting attributes should fail after deletion.
        let err = provider
            .get_platform_application_attributes(&GetPlatformApplicationAttributesInput {
                platform_application_arn: output.platform_application_arn,
            })
            .unwrap_err();
        assert_eq!(err.code, SnsErrorCode::NotFound);
    }

    #[test]
    fn test_should_get_and_set_platform_application_attributes() {
        let provider = make_provider();
        let mut initial_attrs = HashMap::new();
        initial_attrs.insert("Key1".to_owned(), "Value1".to_owned());

        let app = provider
            .create_platform_application(CreatePlatformApplicationInput {
                name: "AttrApp".to_owned(),
                platform: "GCM".to_owned(),
                attributes: initial_attrs,
            })
            .unwrap();

        // Get attributes.
        let output = provider
            .get_platform_application_attributes(&GetPlatformApplicationAttributesInput {
                platform_application_arn: app.platform_application_arn.clone(),
            })
            .unwrap();
        assert_eq!(output.attributes.get("Key1").unwrap(), "Value1");

        // Set new attribute.
        let mut new_attrs = HashMap::new();
        new_attrs.insert("Key2".to_owned(), "Value2".to_owned());
        provider
            .set_platform_application_attributes(SetPlatformApplicationAttributesInput {
                platform_application_arn: app.platform_application_arn.clone(),
                attributes: new_attrs,
            })
            .unwrap();

        // Verify both attributes are present.
        let output = provider
            .get_platform_application_attributes(&GetPlatformApplicationAttributesInput {
                platform_application_arn: app.platform_application_arn,
            })
            .unwrap();
        assert_eq!(output.attributes.get("Key1").unwrap(), "Value1");
        assert_eq!(output.attributes.get("Key2").unwrap(), "Value2");
    }

    #[test]
    fn test_should_list_platform_applications() {
        let provider = make_provider();

        for name in &["App1", "App2", "App3"] {
            provider
                .create_platform_application(CreatePlatformApplicationInput {
                    name: (*name).to_owned(),
                    platform: "GCM".to_owned(),
                    attributes: HashMap::new(),
                })
                .unwrap();
        }

        let output = provider
            .list_platform_applications(&ListPlatformApplicationsInput { next_token: None })
            .unwrap();
        assert_eq!(output.platform_applications.len(), 3);
        assert!(output.next_token.is_none());
    }

    // ---- Phase 3 Tests: Platform Endpoints ----

    #[test]
    fn test_should_create_platform_endpoint() {
        let provider = make_provider();
        let app = provider
            .create_platform_application(CreatePlatformApplicationInput {
                name: "EndpointApp".to_owned(),
                platform: "GCM".to_owned(),
                attributes: HashMap::new(),
            })
            .unwrap();

        let output = provider
            .create_platform_endpoint(CreatePlatformEndpointInput {
                platform_application_arn: app.platform_application_arn,
                token: "device-token-123".to_owned(),
                custom_user_data: Some("user-data".to_owned()),
                attributes: HashMap::new(),
            })
            .unwrap();

        assert!(output.endpoint_arn.contains("endpoint/GCM/EndpointApp/"));

        // Verify attributes include Token and Enabled.
        let attrs = provider
            .get_endpoint_attributes(&GetEndpointAttributesInput {
                endpoint_arn: output.endpoint_arn,
            })
            .unwrap();
        assert_eq!(attrs.attributes.get("Token").unwrap(), "device-token-123");
        assert_eq!(attrs.attributes.get("Enabled").unwrap(), "true");
        assert_eq!(attrs.attributes.get("CustomUserData").unwrap(), "user-data");
    }

    #[test]
    fn test_should_create_endpoint_fails_for_nonexistent_app() {
        let provider = make_provider();
        let err = provider
            .create_platform_endpoint(CreatePlatformEndpointInput {
                platform_application_arn: "arn:aws:sns:us-east-1:000000000000:app/GCM/NoApp"
                    .to_owned(),
                token: "token".to_owned(),
                custom_user_data: None,
                attributes: HashMap::new(),
            })
            .unwrap_err();
        assert_eq!(err.code, SnsErrorCode::NotFound);
    }

    #[test]
    fn test_should_delete_endpoint() {
        let provider = make_provider();
        let app = provider
            .create_platform_application(CreatePlatformApplicationInput {
                name: "DelApp".to_owned(),
                platform: "APNS".to_owned(),
                attributes: HashMap::new(),
            })
            .unwrap();

        let ep = provider
            .create_platform_endpoint(CreatePlatformEndpointInput {
                platform_application_arn: app.platform_application_arn,
                token: "tok".to_owned(),
                custom_user_data: None,
                attributes: HashMap::new(),
            })
            .unwrap();

        provider
            .delete_endpoint(&DeleteEndpointInput {
                endpoint_arn: ep.endpoint_arn.clone(),
            })
            .unwrap();

        // Getting attributes should fail after deletion.
        let err = provider
            .get_endpoint_attributes(&GetEndpointAttributesInput {
                endpoint_arn: ep.endpoint_arn,
            })
            .unwrap_err();
        assert_eq!(err.code, SnsErrorCode::NotFound);
    }

    #[test]
    fn test_should_set_endpoint_attributes() {
        let provider = make_provider();
        let app = provider
            .create_platform_application(CreatePlatformApplicationInput {
                name: "SetAttrApp".to_owned(),
                platform: "GCM".to_owned(),
                attributes: HashMap::new(),
            })
            .unwrap();

        let ep = provider
            .create_platform_endpoint(CreatePlatformEndpointInput {
                platform_application_arn: app.platform_application_arn,
                token: "original-token".to_owned(),
                custom_user_data: None,
                attributes: HashMap::new(),
            })
            .unwrap();

        let mut new_attrs = HashMap::new();
        new_attrs.insert("Enabled".to_owned(), "false".to_owned());
        provider
            .set_endpoint_attributes(SetEndpointAttributesInput {
                endpoint_arn: ep.endpoint_arn.clone(),
                attributes: new_attrs,
            })
            .unwrap();

        let output = provider
            .get_endpoint_attributes(&GetEndpointAttributesInput {
                endpoint_arn: ep.endpoint_arn,
            })
            .unwrap();
        assert_eq!(output.attributes.get("Enabled").unwrap(), "false");
    }

    #[test]
    fn test_should_list_endpoints_by_platform_application() {
        let provider = make_provider();
        let app = provider
            .create_platform_application(CreatePlatformApplicationInput {
                name: "ListEpApp".to_owned(),
                platform: "GCM".to_owned(),
                attributes: HashMap::new(),
            })
            .unwrap();

        for i in 0..3 {
            provider
                .create_platform_endpoint(CreatePlatformEndpointInput {
                    platform_application_arn: app.platform_application_arn.clone(),
                    token: format!("token-{i}"),
                    custom_user_data: None,
                    attributes: HashMap::new(),
                })
                .unwrap();
        }

        let output = provider
            .list_endpoints_by_platform_application(&ListEndpointsByPlatformApplicationInput {
                platform_application_arn: app.platform_application_arn,
                next_token: None,
            })
            .unwrap();
        assert_eq!(output.endpoints.len(), 3);
        assert!(output.next_token.is_none());
    }

    #[test]
    fn test_should_delete_platform_app_removes_endpoints() {
        let provider = make_provider();
        let app = provider
            .create_platform_application(CreatePlatformApplicationInput {
                name: "CascadeApp".to_owned(),
                platform: "GCM".to_owned(),
                attributes: HashMap::new(),
            })
            .unwrap();

        let ep = provider
            .create_platform_endpoint(CreatePlatformEndpointInput {
                platform_application_arn: app.platform_application_arn.clone(),
                token: "tok".to_owned(),
                custom_user_data: None,
                attributes: HashMap::new(),
            })
            .unwrap();

        provider
            .delete_platform_application(&DeletePlatformApplicationInput {
                platform_application_arn: app.platform_application_arn,
            })
            .unwrap();

        // Endpoint should also be gone.
        let err = provider
            .get_endpoint_attributes(&GetEndpointAttributesInput {
                endpoint_arn: ep.endpoint_arn,
            })
            .unwrap_err();
        assert_eq!(err.code, SnsErrorCode::NotFound);
    }

    // ---- Phase 3 Tests: SMS ----

    #[test]
    fn test_should_get_and_set_sms_attributes() {
        let provider = make_provider();

        // Initially empty.
        let output = provider
            .get_sms_attributes(&GetSMSAttributesInput {
                attributes: Vec::new(),
            })
            .unwrap();
        assert!(output.attributes.is_empty());

        // Set attributes.
        let mut attrs = HashMap::new();
        attrs.insert("DefaultSMSType".to_owned(), "Transactional".to_owned());
        attrs.insert("MonthlySpendLimit".to_owned(), "100".to_owned());
        provider
            .set_sms_attributes(SetSMSAttributesInput { attributes: attrs })
            .unwrap();

        // Get all.
        let output = provider
            .get_sms_attributes(&GetSMSAttributesInput {
                attributes: Vec::new(),
            })
            .unwrap();
        assert_eq!(output.attributes.len(), 2);
        assert_eq!(
            output.attributes.get("DefaultSMSType").unwrap(),
            "Transactional"
        );

        // Get specific.
        let output = provider
            .get_sms_attributes(&GetSMSAttributesInput {
                attributes: vec!["DefaultSMSType".to_owned()],
            })
            .unwrap();
        assert_eq!(output.attributes.len(), 1);
    }

    #[test]
    fn test_should_check_phone_number_opted_out() {
        let provider = make_provider();

        // Not opted out by default.
        let output = provider
            .check_if_phone_number_is_opted_out(&CheckIfPhoneNumberIsOptedOutInput {
                phone_number: "+15551234567".to_owned(),
            })
            .unwrap();
        assert!(!output.is_opted_out);
    }

    #[test]
    fn test_should_opt_in_phone_number() {
        let provider = make_provider();

        // Manually insert an opted-out number.
        {
            let mut opted_out = provider.opted_out_numbers.write();
            opted_out.insert("+15559999999".to_owned());
        }

        // Verify it's opted out.
        let output = provider
            .check_if_phone_number_is_opted_out(&CheckIfPhoneNumberIsOptedOutInput {
                phone_number: "+15559999999".to_owned(),
            })
            .unwrap();
        assert!(output.is_opted_out);

        // Opt it back in.
        provider
            .opt_in_phone_number(&OptInPhoneNumberInput {
                phone_number: "+15559999999".to_owned(),
            })
            .unwrap();

        // Verify no longer opted out.
        let output = provider
            .check_if_phone_number_is_opted_out(&CheckIfPhoneNumberIsOptedOutInput {
                phone_number: "+15559999999".to_owned(),
            })
            .unwrap();
        assert!(!output.is_opted_out);
    }

    #[test]
    fn test_should_list_phone_numbers_opted_out() {
        let provider = make_provider();

        {
            let mut opted_out = provider.opted_out_numbers.write();
            opted_out.insert("+15551111111".to_owned());
            opted_out.insert("+15552222222".to_owned());
        }

        let output = provider
            .list_phone_numbers_opted_out(&ListPhoneNumbersOptedOutInput { next_token: None })
            .unwrap();
        assert_eq!(output.phone_numbers.len(), 2);
        // Should be sorted.
        assert!(output.phone_numbers[0] < output.phone_numbers[1]);
    }

    #[test]
    fn test_should_get_sms_sandbox_account_status() {
        let provider = make_provider();
        let output = provider
            .get_sms_sandbox_account_status(&GetSMSSandboxAccountStatusInput {})
            .unwrap();
        assert!(output.is_in_sandbox);
    }

    #[test]
    fn test_should_create_and_verify_sms_sandbox_phone_number() {
        let provider = make_provider();

        // Create a sandbox phone number.
        provider
            .create_sms_sandbox_phone_number(&CreateSMSSandboxPhoneNumberInput {
                phone_number: "+15553334444".to_owned(),
                language_code: None,
            })
            .unwrap();

        // List and verify it's pending.
        let output = provider
            .list_sms_sandbox_phone_numbers(&ListSMSSandboxPhoneNumbersInput { next_token: None })
            .unwrap();
        assert_eq!(output.phone_numbers.len(), 1);
        assert_eq!(output.phone_numbers[0].phone_number, "+15553334444");
        assert_eq!(output.phone_numbers[0].status, "Pending");

        // Verify the phone number (any OTP accepted in local dev).
        provider
            .verify_sms_sandbox_phone_number(&VerifySMSSandboxPhoneNumberInput {
                phone_number: "+15553334444".to_owned(),
                one_time_password: "123456".to_owned(),
            })
            .unwrap();

        // List and verify it's now verified.
        let output = provider
            .list_sms_sandbox_phone_numbers(&ListSMSSandboxPhoneNumbersInput { next_token: None })
            .unwrap();
        assert_eq!(output.phone_numbers[0].status, "Verified");
    }

    #[test]
    fn test_should_delete_sms_sandbox_phone_number() {
        let provider = make_provider();

        provider
            .create_sms_sandbox_phone_number(&CreateSMSSandboxPhoneNumberInput {
                phone_number: "+15555556666".to_owned(),
                language_code: None,
            })
            .unwrap();

        provider
            .delete_sms_sandbox_phone_number(&DeleteSMSSandboxPhoneNumberInput {
                phone_number: "+15555556666".to_owned(),
            })
            .unwrap();

        let output = provider
            .list_sms_sandbox_phone_numbers(&ListSMSSandboxPhoneNumbersInput { next_token: None })
            .unwrap();
        assert!(output.phone_numbers.is_empty());
    }

    #[test]
    fn test_should_delete_nonexistent_sandbox_phone_returns_error() {
        let provider = make_provider();
        let err = provider
            .delete_sms_sandbox_phone_number(&DeleteSMSSandboxPhoneNumberInput {
                phone_number: "+15550000000".to_owned(),
            })
            .unwrap_err();
        assert_eq!(err.code, SnsErrorCode::NotFound);
    }

    #[test]
    fn test_should_verify_nonexistent_sandbox_phone_returns_error() {
        let provider = make_provider();
        let err = provider
            .verify_sms_sandbox_phone_number(&VerifySMSSandboxPhoneNumberInput {
                phone_number: "+15550000000".to_owned(),
                one_time_password: "123456".to_owned(),
            })
            .unwrap_err();
        assert_eq!(err.code, SnsErrorCode::NotFound);
    }

    #[test]
    fn test_should_list_origination_numbers_empty() {
        let provider = make_provider();
        let output = provider
            .list_origination_numbers(&ListOriginationNumbersInput {
                next_token: None,
                max_results: None,
            })
            .unwrap();
        assert!(output.phone_numbers.is_empty());
        assert!(output.next_token.is_none());
    }
}
