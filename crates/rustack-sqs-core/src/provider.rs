//! Main SQS provider implementing all operations.
//!
//! Acts as the queue manager that owns all queue actors, creating and
//! destroying them as queues are created and deleted.

use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};

use dashmap::{DashMap, mapref::multiple::RefMulti};
use rustack_sqs_model::{
    error::SqsError,
    input::{
        AddPermissionInput, CancelMessageMoveTaskInput, ChangeMessageVisibilityBatchInput,
        ChangeMessageVisibilityInput, CreateQueueInput, DeleteMessageBatchInput,
        DeleteMessageInput, DeleteQueueInput, GetQueueAttributesInput, GetQueueUrlInput,
        ListDeadLetterSourceQueuesInput, ListMessageMoveTasksInput, ListQueueTagsInput,
        ListQueuesInput, PurgeQueueInput, ReceiveMessageInput, RemovePermissionInput,
        SendMessageBatchInput, SendMessageInput, SetQueueAttributesInput,
        StartMessageMoveTaskInput, TagQueueInput, UntagQueueInput,
    },
    output::{
        AddPermissionOutput, CancelMessageMoveTaskOutput, ChangeMessageVisibilityBatchOutput,
        ChangeMessageVisibilityOutput, CreateQueueOutput, DeleteMessageBatchOutput,
        DeleteMessageOutput, DeleteQueueOutput, GetQueueAttributesOutput, GetQueueUrlOutput,
        ListDeadLetterSourceQueuesOutput, ListMessageMoveTasksOutput, ListQueueTagsOutput,
        ListQueuesOutput, PurgeQueueOutput, ReceiveMessageOutput, RemovePermissionOutput,
        SendMessageBatchOutput, SendMessageOutput, SetQueueAttributesOutput,
        StartMessageMoveTaskOutput, TagQueueOutput, UntagQueueOutput,
    },
};
use tokio::sync::mpsc;

use crate::{
    config::SqsConfig,
    message::now_epoch_seconds,
    queue::{
        actor::{QueueActor, QueueHandle, QueueMetadata},
        attributes::QueueAttributes,
        url::{extract_queue_name, queue_arn, queue_url},
    },
};

/// Main SQS provider. Acts as the queue manager that owns all queue actors.
#[derive(Debug)]
pub struct RustackSqs {
    /// Queue registry: queue_name -> QueueHandle.
    queues: DashMap<String, QueueHandle>,
    /// Configuration.
    config: Arc<SqsConfig>,
}

impl RustackSqs {
    /// Create a new SQS provider.
    #[must_use]
    pub fn new(config: SqsConfig) -> Self {
        Self {
            queues: DashMap::new(),
            config: Arc::new(config),
        }
    }

    /// Resolve a queue name from a queue URL.
    fn resolve_queue_name(queue_url_str: &str) -> Result<String, SqsError> {
        extract_queue_name(queue_url_str)
            .map(String::from)
            .ok_or_else(|| {
                SqsError::non_existent_queue(format!(
                    "The specified queue does not exist for this wsdl version. QueueUrl: \
                     {queue_url_str}"
                ))
            })
    }

    /// Get a reference to a queue handle by URL.
    fn get_queue(
        &self,
        queue_url_str: &str,
    ) -> Result<dashmap::mapref::one::Ref<'_, String, QueueHandle>, SqsError> {
        let name = Self::resolve_queue_name(queue_url_str)?;
        self.queues.get(&name).ok_or_else(|| {
            SqsError::non_existent_queue(
                "The specified queue does not exist for this wsdl version.",
            )
        })
    }

    // ---- Queue Management Operations ----

    /// Handle `CreateQueue`.
    pub async fn create_queue(
        &self,
        input: CreateQueueInput,
    ) -> Result<CreateQueueOutput, SqsError> {
        let queue_name = &input.queue_name;

        // Validate queue name: 1-80 chars, alphanumeric + hyphens + underscores.
        validate_queue_name(queue_name)?;

        // Not a file extension - this is an AWS SQS FIFO queue naming convention.
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        let is_fifo = queue_name.ends_with(".fifo");

        // Check if FifoQueue attribute is consistent with name.
        if let Some(fifo_attr) = input.attributes.get("FifoQueue") {
            if fifo_attr == "true" && !is_fifo {
                return Err(SqsError::invalid_parameter_value(
                    "The queue name must end with the .fifo suffix for FIFO queues.",
                ));
            }
            if fifo_attr != "true" && is_fifo {
                return Err(SqsError::invalid_parameter_value(
                    "The queue name for a FIFO queue must end with the .fifo suffix.",
                ));
            }
        }

        // Idempotent create: if queue exists with same attributes, return existing URL.
        // If attributes differ, return QueueAlreadyExists.
        if let Some(existing) = self.queues.get(queue_name) {
            if !input.attributes.is_empty() {
                let existing_attrs = existing
                    .get_attributes(vec!["All".to_owned()])
                    .await
                    .unwrap_or_default();

                // Check each requested attribute against the existing queue's attributes.
                for (key, value) in &input.attributes {
                    // Skip FifoQueue since it's validated by name convention.
                    if key == "FifoQueue" {
                        continue;
                    }
                    if let Some(existing_value) = existing_attrs.get(key) {
                        if existing_value != value {
                            return Err(SqsError::queue_already_exists(format!(
                                "A queue already exists with the same name and a different value \
                                 for attribute {key}."
                            )));
                        }
                    }
                }
            }
            return Ok(CreateQueueOutput {
                queue_url: Some(existing.metadata.url.clone()),
            });
        }

        let attributes = QueueAttributes::from_map(&input.attributes, is_fifo)?;
        let url = queue_url(
            &self.config.host,
            self.config.port,
            &self.config.account_id,
            queue_name,
        );
        let arn = queue_arn(
            &self.config.default_region,
            &self.config.account_id,
            queue_name,
        );

        // Spawn queue actor.
        let (sender, receiver) = mpsc::channel(256);
        let now = now_epoch_seconds();

        let actor = QueueActor::new(
            queue_name.clone(),
            arn.clone(),
            is_fifo,
            attributes,
            receiver,
            input.tags,
            self.config.account_id.clone(),
            now,
        );
        let task = tokio::spawn(actor.run());

        let handle = QueueHandle {
            sender,
            metadata: QueueMetadata {
                name: queue_name.clone(),
                url: url.clone(),
                arn,
                is_fifo,
                created_at: now,
            },
            task,
            shutdown: Arc::new(AtomicBool::new(false)),
        };

        self.queues.insert(queue_name.clone(), handle);
        tracing::info!(queue = %queue_name, "created SQS queue");

        Ok(CreateQueueOutput {
            queue_url: Some(url),
        })
    }

    /// Handle `DeleteQueue`.
    pub async fn delete_queue(
        &self,
        input: DeleteQueueInput,
    ) -> Result<DeleteQueueOutput, SqsError> {
        let name = Self::resolve_queue_name(&input.queue_url)?;
        if let Some((_, handle)) = self.queues.remove(&name) {
            handle.shutdown().await;
            tracing::info!(queue = %name, "deleted SQS queue");
        }
        // AWS doesn't error if queue doesn't exist for delete.
        Ok(DeleteQueueOutput {})
    }

    /// Handle `GetQueueUrl`.
    #[allow(clippy::unused_async)] // Must be async to match the handler trait interface.
    pub async fn get_queue_url(
        &self,
        input: GetQueueUrlInput,
    ) -> Result<GetQueueUrlOutput, SqsError> {
        let handle = self.queues.get(&input.queue_name).ok_or_else(|| {
            SqsError::non_existent_queue(
                "The specified queue does not exist for this wsdl version.",
            )
        })?;
        Ok(GetQueueUrlOutput {
            queue_url: Some(handle.metadata.url.clone()),
        })
    }

    /// Handle `ListQueues`.
    #[allow(clippy::unused_async)] // Must be async to match the handler trait interface.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    // max_results clamped to 1..=1000, always positive and fits in usize.
    pub async fn list_queues(&self, input: ListQueuesInput) -> Result<ListQueuesOutput, SqsError> {
        let max_results = input.max_results.unwrap_or(1000).clamp(1, 1000) as usize;

        let mut urls: Vec<String> = self
            .queues
            .iter()
            .filter(|entry: &RefMulti<'_, String, QueueHandle>| {
                if let Some(ref prefix) = input.queue_name_prefix {
                    entry.key().starts_with(prefix.as_str())
                } else {
                    true
                }
            })
            .map(|entry: RefMulti<'_, String, QueueHandle>| entry.value().metadata.url.clone())
            .collect();

        urls.sort();

        // Simple pagination using NextToken as offset index.
        let start = input
            .next_token
            .as_ref()
            .and_then(|t| t.parse::<usize>().ok())
            .unwrap_or(0);

        let page: Vec<String> = urls.into_iter().skip(start).take(max_results).collect();
        let next_token = if start + max_results < self.queues.len() {
            Some((start + max_results).to_string())
        } else {
            None
        };

        Ok(ListQueuesOutput {
            queue_urls: page,
            next_token,
        })
    }

    /// Handle `GetQueueAttributes`.
    pub async fn get_queue_attributes(
        &self,
        input: GetQueueAttributesInput,
    ) -> Result<GetQueueAttributesOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        let attrs = handle.get_attributes(input.attribute_names).await?;
        Ok(GetQueueAttributesOutput { attributes: attrs })
    }

    /// Handle `SetQueueAttributes`.
    pub async fn set_queue_attributes(
        &self,
        input: SetQueueAttributesInput,
    ) -> Result<SetQueueAttributesOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        handle.set_attributes(input.attributes).await?;
        Ok(SetQueueAttributesOutput {})
    }

    // ---- Message Operations ----

    /// Handle `SendMessage`.
    pub async fn send_message(
        &self,
        input: SendMessageInput,
    ) -> Result<SendMessageOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        handle.send_message(input).await
    }

    /// Handle `ReceiveMessage`.
    pub async fn receive_message(
        &self,
        input: ReceiveMessageInput,
    ) -> Result<ReceiveMessageOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        handle.receive_message(input).await
    }

    /// Handle `DeleteMessage`.
    pub async fn delete_message(
        &self,
        input: DeleteMessageInput,
    ) -> Result<DeleteMessageOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        handle.delete_message(input.receipt_handle).await?;
        Ok(DeleteMessageOutput {})
    }

    /// Handle `PurgeQueue`.
    pub async fn purge_queue(&self, input: PurgeQueueInput) -> Result<PurgeQueueOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        handle.purge().await?;
        Ok(PurgeQueueOutput {})
    }

    /// Handle `ChangeMessageVisibility`.
    pub async fn change_message_visibility(
        &self,
        input: ChangeMessageVisibilityInput,
    ) -> Result<ChangeMessageVisibilityOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        handle
            .change_visibility(input.receipt_handle, input.visibility_timeout)
            .await?;
        Ok(ChangeMessageVisibilityOutput {})
    }

    // ---- Batch Operations ----

    /// Handle `SendMessageBatch`.
    pub async fn send_message_batch(
        &self,
        input: SendMessageBatchInput,
    ) -> Result<SendMessageBatchOutput, SqsError> {
        validate_batch_entries(
            &input
                .entries
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
        )?;

        let handle = self.get_queue(&input.queue_url)?;
        let mut successful = Vec::new();
        let mut failed = Vec::new();

        for entry in input.entries {
            let send_input = SendMessageInput {
                queue_url: input.queue_url.clone(),
                message_body: entry.message_body,
                delay_seconds: entry.delay_seconds,
                message_deduplication_id: entry.message_deduplication_id,
                message_group_id: entry.message_group_id,
                message_attributes: entry.message_attributes,
                message_system_attributes: entry.message_system_attributes,
            };
            match handle.send_message(send_input).await {
                Ok(output) => {
                    successful.push(rustack_sqs_model::types::SendMessageBatchResultEntry {
                        id: entry.id,
                        message_id: output.message_id.unwrap_or_default(),
                        md5_of_message_body: output.md5_of_message_body.unwrap_or_default(),
                        md5_of_message_attributes: output.md5_of_message_attributes,
                        sequence_number: output.sequence_number,
                    });
                }
                Err(err) => {
                    failed.push(rustack_sqs_model::types::BatchResultErrorEntry {
                        id: entry.id,
                        sender_fault: err.code.is_sender_fault(),
                        code: err.code.error_type().to_owned(),
                        message: Some(err.message),
                    });
                }
            }
        }

        Ok(SendMessageBatchOutput { successful, failed })
    }

    /// Handle `DeleteMessageBatch`.
    pub async fn delete_message_batch(
        &self,
        input: DeleteMessageBatchInput,
    ) -> Result<DeleteMessageBatchOutput, SqsError> {
        validate_batch_entries(
            &input
                .entries
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
        )?;

        let handle = self.get_queue(&input.queue_url)?;
        let mut successful = Vec::new();
        let mut failed = Vec::new();

        for entry in input.entries {
            match handle.delete_message(entry.receipt_handle).await {
                Ok(()) => {
                    successful.push(rustack_sqs_model::types::DeleteMessageBatchResultEntry {
                        id: entry.id,
                    });
                }
                Err(err) => {
                    failed.push(rustack_sqs_model::types::BatchResultErrorEntry {
                        id: entry.id,
                        sender_fault: err.code.is_sender_fault(),
                        code: err.code.error_type().to_owned(),
                        message: Some(err.message),
                    });
                }
            }
        }

        Ok(DeleteMessageBatchOutput { successful, failed })
    }

    /// Handle `ChangeMessageVisibilityBatch`.
    pub async fn change_message_visibility_batch(
        &self,
        input: ChangeMessageVisibilityBatchInput,
    ) -> Result<ChangeMessageVisibilityBatchOutput, SqsError> {
        validate_batch_entries(
            &input
                .entries
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
        )?;

        let handle = self.get_queue(&input.queue_url)?;
        let mut successful = Vec::new();
        let mut failed = Vec::new();

        for entry in input.entries {
            match handle
                .change_visibility(entry.receipt_handle, entry.visibility_timeout)
                .await
            {
                Ok(()) => {
                    successful.push(
                        rustack_sqs_model::types::ChangeMessageVisibilityBatchResultEntry {
                            id: entry.id,
                        },
                    );
                }
                Err(err) => {
                    failed.push(rustack_sqs_model::types::BatchResultErrorEntry {
                        id: entry.id,
                        sender_fault: err.code.is_sender_fault(),
                        code: err.code.error_type().to_owned(),
                        message: Some(err.message),
                    });
                }
            }
        }

        Ok(ChangeMessageVisibilityBatchOutput { successful, failed })
    }

    // ---- Tag Operations ----

    /// Handle `TagQueue`.
    pub async fn tag_queue(&self, input: TagQueueInput) -> Result<TagQueueOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        handle.set_tags(input.tags).await?;
        Ok(TagQueueOutput {})
    }

    /// Handle `UntagQueue`.
    pub async fn untag_queue(&self, input: UntagQueueInput) -> Result<UntagQueueOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        handle.remove_tags(input.tag_keys).await?;
        Ok(UntagQueueOutput {})
    }

    /// Handle `ListQueueTags`.
    pub async fn list_queue_tags(
        &self,
        input: ListQueueTagsInput,
    ) -> Result<ListQueueTagsOutput, SqsError> {
        let handle = self.get_queue(&input.queue_url)?;
        let tags = handle.get_tags().await?;
        Ok(ListQueueTagsOutput { tags })
    }

    // ---- DLQ Operations ----

    /// Handle `ListDeadLetterSourceQueues`.
    pub async fn list_dead_letter_source_queues(
        &self,
        input: ListDeadLetterSourceQueuesInput,
    ) -> Result<ListDeadLetterSourceQueuesOutput, SqsError> {
        let target_handle = self.get_queue(&input.queue_url)?;
        let target_arn = target_handle.metadata.arn.clone();
        drop(target_handle);

        let mut source_urls = Vec::new();
        for entry in &self.queues {
            let attrs: Result<HashMap<String, String>, SqsError> = entry
                .value()
                .get_attributes(vec!["RedrivePolicy".to_owned()])
                .await;
            if let Ok(attrs) = attrs {
                if let Some(policy_json) = attrs.get("RedrivePolicy") {
                    if let Ok(policy) =
                        serde_json::from_str::<rustack_sqs_model::types::RedrivePolicy>(policy_json)
                    {
                        if policy.dead_letter_target_arn == target_arn {
                            source_urls.push(entry.value().metadata.url.clone());
                        }
                    }
                }
            }
        }

        Ok(ListDeadLetterSourceQueuesOutput {
            queue_urls: source_urls,
            next_token: None,
        })
    }

    // ---- Permission Operations (store only, no enforcement) ----

    /// Handle `AddPermission`.
    pub async fn add_permission(
        &self,
        input: AddPermissionInput,
    ) -> Result<AddPermissionOutput, SqsError> {
        // Accept and store in Policy attribute, but do not enforce.
        let handle = self.get_queue(&input.queue_url)?;
        let mut attrs: HashMap<String, String> =
            handle.get_attributes(vec!["Policy".to_owned()]).await?;
        let _policy = attrs.remove("Policy").unwrap_or_else(|| "{}".to_owned());
        // Simplified: we accept the call but don't modify the policy in detail.
        Ok(AddPermissionOutput {})
    }

    /// Handle `RemovePermission`.
    #[allow(clippy::unused_async)] // Must be async to match the handler trait interface.
    pub async fn remove_permission(
        &self,
        input: RemovePermissionInput,
    ) -> Result<RemovePermissionOutput, SqsError> {
        let _handle = self.get_queue(&input.queue_url)?;
        Ok(RemovePermissionOutput {})
    }

    // ---- Message Move Task Operations (stubs) ----

    /// Handle `StartMessageMoveTask`.
    #[allow(clippy::unused_async)] // Must be async to match the handler trait interface.
    pub async fn start_message_move_task(
        &self,
        _input: StartMessageMoveTaskInput,
    ) -> Result<StartMessageMoveTaskOutput, SqsError> {
        Err(SqsError::new(
            rustack_sqs_model::error::SqsErrorCode::UnsupportedOperation,
            "StartMessageMoveTask is not yet implemented.",
        ))
    }

    /// Handle `CancelMessageMoveTask`.
    #[allow(clippy::unused_async)] // Must be async to match the handler trait interface.
    pub async fn cancel_message_move_task(
        &self,
        _input: CancelMessageMoveTaskInput,
    ) -> Result<CancelMessageMoveTaskOutput, SqsError> {
        Err(SqsError::new(
            rustack_sqs_model::error::SqsErrorCode::UnsupportedOperation,
            "CancelMessageMoveTask is not yet implemented.",
        ))
    }

    /// Handle `ListMessageMoveTasks`.
    #[allow(clippy::unused_async)] // Must be async to match the handler trait interface.
    pub async fn list_message_move_tasks(
        &self,
        _input: ListMessageMoveTasksInput,
    ) -> Result<ListMessageMoveTasksOutput, SqsError> {
        Ok(ListMessageMoveTasksOutput {
            results: Vec::new(),
        })
    }
}

/// Validate a queue name (1-80 chars, alphanumeric + hyphens + underscores + dots).
fn validate_queue_name(name: &str) -> Result<(), SqsError> {
    if name.is_empty() || name.len() > 80 {
        return Err(SqsError::invalid_parameter_value(
            "Queue name must be between 1 and 80 characters long.",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(SqsError::invalid_parameter_value(
            "Queue name can only include alphanumeric characters, hyphens, or underscores.",
        ));
    }
    Ok(())
}

/// Validate batch entry IDs.
fn validate_batch_entries(ids: &[&str]) -> Result<(), SqsError> {
    if ids.is_empty() {
        return Err(SqsError::empty_batch_request());
    }
    if ids.len() > 10 {
        return Err(SqsError::too_many_entries_in_batch());
    }
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            return Err(SqsError::batch_entry_ids_not_distinct());
        }
    }
    Ok(())
}
