//! Queue attribute management and validation.

use std::collections::HashMap;

use ruststack_sqs_model::error::SqsError;
use ruststack_sqs_model::types::RedrivePolicy;

/// Queue attributes with validated values and defaults.
#[derive(Debug, Clone)]
pub struct QueueAttributes {
    /// Default delay for messages (0-900 seconds, default 0).
    pub delay_seconds: i32,
    /// Maximum message size (1024-262144 bytes, default 262144).
    pub maximum_message_size: i32,
    /// Message retention period (60-1209600 seconds, default 345600 = 4 days).
    pub message_retention_period: i32,
    /// Default wait time for receive operations (0-20 seconds, default 0).
    pub receive_message_wait_time_seconds: i32,
    /// Default visibility timeout (0-43200 seconds, default 30).
    pub visibility_timeout: i32,
    /// Dead-letter queue redrive configuration.
    pub redrive_policy: Option<RedrivePolicy>,
    /// Redrive allow policy JSON.
    pub redrive_allow_policy: Option<String>,
    /// Content-based deduplication (FIFO only, default false).
    pub content_based_deduplication: bool,
    /// Deduplication scope (FIFO only).
    pub deduplication_scope: String,
    /// FIFO throughput limit (FIFO only).
    pub fifo_throughput_limit: String,
    /// IAM policy JSON (stored, not enforced).
    pub policy: Option<String>,
    /// KMS master key ID (stored, not enforced).
    pub kms_master_key_id: Option<String>,
    /// KMS data key reuse period.
    pub kms_data_key_reuse_period_seconds: Option<i32>,
    /// SQS-managed SSE enabled.
    pub sqs_managed_sse_enabled: bool,
}

impl Default for QueueAttributes {
    fn default() -> Self {
        Self {
            delay_seconds: 0,
            maximum_message_size: 262_144,
            message_retention_period: 345_600,
            receive_message_wait_time_seconds: 0,
            visibility_timeout: 30,
            redrive_policy: None,
            redrive_allow_policy: None,
            content_based_deduplication: false,
            deduplication_scope: "queue".to_owned(),
            fifo_throughput_limit: "perQueue".to_owned(),
            policy: None,
            kms_master_key_id: None,
            kms_data_key_reuse_period_seconds: None,
            sqs_managed_sse_enabled: true,
        }
    }
}

impl QueueAttributes {
    /// Build attributes from a string key-value map, applying defaults for
    /// omitted values and validating ranges.
    pub fn from_map(attrs: &HashMap<String, String>, is_fifo: bool) -> Result<Self, SqsError> {
        let mut result = Self::default();

        for (key, value) in attrs {
            match key.as_str() {
                "DelaySeconds" => {
                    result.delay_seconds = parse_i32_range(key, value, 0, 900)?;
                }
                "MaximumMessageSize" => {
                    result.maximum_message_size = parse_i32_range(key, value, 1024, 262_144)?;
                }
                "MessageRetentionPeriod" => {
                    result.message_retention_period = parse_i32_range(key, value, 60, 1_209_600)?;
                }
                "ReceiveMessageWaitTimeSeconds" => {
                    result.receive_message_wait_time_seconds = parse_i32_range(key, value, 0, 20)?;
                }
                "VisibilityTimeout" => {
                    result.visibility_timeout = parse_i32_range(key, value, 0, 43_200)?;
                }
                "RedrivePolicy" => {
                    let policy: RedrivePolicy = serde_json::from_str(value).map_err(|e| {
                        SqsError::invalid_parameter_value(format!(
                            "Invalid value for the parameter RedrivePolicy: {e}"
                        ))
                    })?;
                    if policy.max_receive_count < 1 {
                        return Err(SqsError::invalid_parameter_value(
                            "Value for parameter RedrivePolicy is invalid. Reason: maxReceiveCount must be between 1 and 1000.",
                        ));
                    }
                    result.redrive_policy = Some(policy);
                }
                "RedriveAllowPolicy" => {
                    result.redrive_allow_policy = Some(value.clone());
                }
                "ContentBasedDeduplication" => {
                    if !is_fifo {
                        return Err(SqsError::invalid_parameter_value(
                            "Unknown Attribute ContentBasedDeduplication.",
                        ));
                    }
                    result.content_based_deduplication = value == "true" || value == "1";
                }
                "DeduplicationScope" => {
                    if !is_fifo {
                        return Err(SqsError::invalid_parameter_value(
                            "Unknown Attribute DeduplicationScope.",
                        ));
                    }
                    if value != "queue" && value != "messageGroup" {
                        return Err(SqsError::invalid_parameter_value(format!(
                            "Invalid value for DeduplicationScope: {value}"
                        )));
                    }
                    result.deduplication_scope.clone_from(value);
                }
                "FifoThroughputLimit" => {
                    if !is_fifo {
                        return Err(SqsError::invalid_parameter_value(
                            "Unknown Attribute FifoThroughputLimit.",
                        ));
                    }
                    result.fifo_throughput_limit.clone_from(value);
                }
                "Policy" => {
                    result.policy = Some(value.clone());
                }
                "KmsMasterKeyId" => {
                    result.kms_master_key_id = Some(value.clone());
                }
                "KmsDataKeyReusePeriodSeconds" => {
                    result.kms_data_key_reuse_period_seconds =
                        Some(parse_i32_range(key, value, 60, 86_400)?);
                }
                "SqsManagedSseEnabled" => {
                    result.sqs_managed_sse_enabled = value == "true" || value == "1";
                }
                "FifoQueue" => {
                    // Accepted but validated elsewhere (queue name must end with .fifo).
                }
                _ => {
                    return Err(SqsError::invalid_attribute_name(format!(
                        "Unknown Attribute {key}."
                    )));
                }
            }
        }

        if is_fifo && result.delay_seconds != 0 {
            // FIFO queues do not support per-queue delay.
            // Note: AWS allows setting DelaySeconds on FIFO queues, but it's always 0.
            // We follow the same behavior.
        }

        Ok(result)
    }

    /// Update attributes from a string key-value map.
    pub fn update_from_map(
        &mut self,
        attrs: &HashMap<String, String>,
        is_fifo: bool,
    ) -> Result<(), SqsError> {
        for (key, value) in attrs {
            match key.as_str() {
                "DelaySeconds" => {
                    self.delay_seconds = parse_i32_range(key, value, 0, 900)?;
                }
                "MaximumMessageSize" => {
                    self.maximum_message_size = parse_i32_range(key, value, 1024, 262_144)?;
                }
                "MessageRetentionPeriod" => {
                    self.message_retention_period = parse_i32_range(key, value, 60, 1_209_600)?;
                }
                "ReceiveMessageWaitTimeSeconds" => {
                    self.receive_message_wait_time_seconds = parse_i32_range(key, value, 0, 20)?;
                }
                "VisibilityTimeout" => {
                    self.visibility_timeout = parse_i32_range(key, value, 0, 43_200)?;
                }
                "RedrivePolicy" => {
                    if value.is_empty() {
                        self.redrive_policy = None;
                    } else {
                        let policy: RedrivePolicy = serde_json::from_str(value).map_err(|e| {
                            SqsError::invalid_parameter_value(format!(
                                "Invalid value for the parameter RedrivePolicy: {e}"
                            ))
                        })?;
                        self.redrive_policy = Some(policy);
                    }
                }
                "RedriveAllowPolicy" => {
                    self.redrive_allow_policy = if value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                }
                "ContentBasedDeduplication" => {
                    if !is_fifo {
                        return Err(SqsError::invalid_parameter_value(
                            "Unknown Attribute ContentBasedDeduplication.",
                        ));
                    }
                    self.content_based_deduplication = value == "true" || value == "1";
                }
                "DeduplicationScope" => {
                    if !is_fifo {
                        return Err(SqsError::invalid_parameter_value(
                            "Unknown Attribute DeduplicationScope.",
                        ));
                    }
                    self.deduplication_scope.clone_from(value);
                }
                "FifoThroughputLimit" => {
                    if !is_fifo {
                        return Err(SqsError::invalid_parameter_value(
                            "Unknown Attribute FifoThroughputLimit.",
                        ));
                    }
                    self.fifo_throughput_limit.clone_from(value);
                }
                "Policy" => {
                    self.policy = if value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                }
                "KmsMasterKeyId" => {
                    self.kms_master_key_id = if value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                }
                "KmsDataKeyReusePeriodSeconds" => {
                    self.kms_data_key_reuse_period_seconds =
                        Some(parse_i32_range(key, value, 60, 86_400)?);
                }
                "SqsManagedSseEnabled" => {
                    self.sqs_managed_sse_enabled = value == "true" || value == "1";
                }
                _ => {
                    return Err(SqsError::invalid_attribute_name(format!(
                        "Unknown Attribute {key}."
                    )));
                }
            }
        }
        Ok(())
    }

    /// Convert attributes to a string key-value map for `GetQueueAttributes`.
    ///
    /// If `requested` contains `"All"`, returns all attributes.
    /// Otherwise, returns only the requested attributes.
    #[must_use]
    #[allow(clippy::too_many_lines)] // One branch per AWS attribute; straightforward map construction.
    pub fn to_map(
        &self,
        requested: &[String],
        is_fifo: bool,
        arn: &str,
        created_at: u64,
        last_modified_at: u64,
        counts: (u32, u32, u32),
    ) -> HashMap<String, String> {
        let all = requested.is_empty() || requested.iter().any(|n| n == "All");
        let mut map = HashMap::new();

        let want = |name: &str| all || requested.iter().any(|n| n == name);

        if want("DelaySeconds") {
            map.insert("DelaySeconds".to_owned(), self.delay_seconds.to_string());
        }
        if want("MaximumMessageSize") {
            map.insert(
                "MaximumMessageSize".to_owned(),
                self.maximum_message_size.to_string(),
            );
        }
        if want("MessageRetentionPeriod") {
            map.insert(
                "MessageRetentionPeriod".to_owned(),
                self.message_retention_period.to_string(),
            );
        }
        if want("ReceiveMessageWaitTimeSeconds") {
            map.insert(
                "ReceiveMessageWaitTimeSeconds".to_owned(),
                self.receive_message_wait_time_seconds.to_string(),
            );
        }
        if want("VisibilityTimeout") {
            map.insert(
                "VisibilityTimeout".to_owned(),
                self.visibility_timeout.to_string(),
            );
        }
        if want("QueueArn") {
            map.insert("QueueArn".to_owned(), arn.to_owned());
        }
        if want("CreatedTimestamp") {
            map.insert("CreatedTimestamp".to_owned(), created_at.to_string());
        }
        if want("LastModifiedTimestamp") {
            map.insert(
                "LastModifiedTimestamp".to_owned(),
                last_modified_at.to_string(),
            );
        }
        if want("ApproximateNumberOfMessages") {
            map.insert(
                "ApproximateNumberOfMessages".to_owned(),
                counts.0.to_string(),
            );
        }
        if want("ApproximateNumberOfMessagesNotVisible") {
            map.insert(
                "ApproximateNumberOfMessagesNotVisible".to_owned(),
                counts.1.to_string(),
            );
        }
        if want("ApproximateNumberOfMessagesDelayed") {
            map.insert(
                "ApproximateNumberOfMessagesDelayed".to_owned(),
                counts.2.to_string(),
            );
        }
        if want("RedrivePolicy") {
            if let Some(ref policy) = self.redrive_policy {
                if let Ok(json) = serde_json::to_string(policy) {
                    map.insert("RedrivePolicy".to_owned(), json);
                }
            }
        }
        if want("RedriveAllowPolicy") {
            if let Some(ref policy) = self.redrive_allow_policy {
                map.insert("RedriveAllowPolicy".to_owned(), policy.clone());
            }
        }
        if want("Policy") {
            if let Some(ref policy) = self.policy {
                map.insert("Policy".to_owned(), policy.clone());
            }
        }
        if is_fifo {
            if want("FifoQueue") {
                map.insert("FifoQueue".to_owned(), "true".to_owned());
            }
            if want("ContentBasedDeduplication") {
                map.insert(
                    "ContentBasedDeduplication".to_owned(),
                    self.content_based_deduplication.to_string(),
                );
            }
            if want("DeduplicationScope") {
                map.insert(
                    "DeduplicationScope".to_owned(),
                    self.deduplication_scope.clone(),
                );
            }
            if want("FifoThroughputLimit") {
                map.insert(
                    "FifoThroughputLimit".to_owned(),
                    self.fifo_throughput_limit.clone(),
                );
            }
        }
        if want("SqsManagedSseEnabled") {
            map.insert(
                "SqsManagedSseEnabled".to_owned(),
                self.sqs_managed_sse_enabled.to_string(),
            );
        }
        if want("KmsMasterKeyId") {
            if let Some(ref kid) = self.kms_master_key_id {
                map.insert("KmsMasterKeyId".to_owned(), kid.clone());
            }
        }
        if want("KmsDataKeyReusePeriodSeconds") {
            if let Some(period) = self.kms_data_key_reuse_period_seconds {
                map.insert(
                    "KmsDataKeyReusePeriodSeconds".to_owned(),
                    period.to_string(),
                );
            }
        }

        map
    }
}

/// Parse an integer attribute value with range validation.
fn parse_i32_range(key: &str, value: &str, min: i32, max: i32) -> Result<i32, SqsError> {
    let n: i32 = value.parse().map_err(|_| {
        SqsError::invalid_parameter_value(format!("Invalid value for the parameter {key}."))
    })?;
    if n < min || n > max {
        return Err(SqsError::invalid_parameter_value(format!(
            "Invalid value for the parameter {key}. Reason: Must be between {min} and {max}."
        )));
    }
    Ok(n)
}
