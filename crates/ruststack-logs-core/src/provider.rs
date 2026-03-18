//! CloudWatch Logs provider implementing all Phase 0 through Phase 3 operations.
//!
//! The provider uses `DashMap` for concurrent access to log groups, keeping
//! the design simple without an actor model.

use std::collections::HashMap;

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;

use ruststack_logs_model::error::{LogsError, LogsErrorCode};
use ruststack_logs_model::input::{
    AssociateKmsKeyInput, CreateLogGroupInput, CreateLogStreamInput, DeleteDestinationInput,
    DeleteLogGroupInput, DeleteLogStreamInput, DeleteMetricFilterInput, DeleteQueryDefinitionInput,
    DeleteResourcePolicyInput, DeleteRetentionPolicyInput, DeleteSubscriptionFilterInput,
    DescribeDestinationsInput, DescribeLogGroupsInput, DescribeLogStreamsInput,
    DescribeMetricFiltersInput, DescribeQueriesInput, DescribeQueryDefinitionsInput,
    DescribeResourcePoliciesInput, DescribeSubscriptionFiltersInput, DisassociateKmsKeyInput,
    FilterLogEventsInput, GetLogEventsInput, GetQueryResultsInput, ListTagsForResourceInput,
    ListTagsLogGroupInput, PutDestinationInput, PutDestinationPolicyInput, PutLogEventsInput,
    PutMetricFilterInput, PutQueryDefinitionInput, PutResourcePolicyInput, PutRetentionPolicyInput,
    PutSubscriptionFilterInput, StartQueryInput, StopQueryInput, TagLogGroupInput,
    TagResourceInput, TestMetricFilterInput, UntagLogGroupInput, UntagResourceInput,
};
use ruststack_logs_model::output::{
    DescribeDestinationsResponse, DescribeLogGroupsResponse, DescribeLogStreamsResponse,
    DescribeMetricFiltersResponse, DescribeQueriesResponse, DescribeQueryDefinitionsResponse,
    DescribeResourcePoliciesResponse, DescribeSubscriptionFiltersResponse, FilterLogEventsResponse,
    GetLogEventsResponse, GetQueryResultsResponse, ListTagsForResourceResponse,
    ListTagsLogGroupResponse, PutDestinationResponse, PutLogEventsResponse,
    PutQueryDefinitionResponse, PutResourcePolicyResponse, StartQueryResponse, StopQueryResponse,
    TestMetricFilterResponse,
};
use ruststack_logs_model::types::{
    Destination, FilteredLogEvent, LogGroup, LogStream, MetricFilter, MetricFilterMatchRecord,
    OutputLogEvent, QueryDefinition, QueryStatistics, QueryStatus, ResourcePolicy,
    SearchedLogStream, SubscriptionFilter,
};

use crate::config::LogsConfig;

/// Maximum number of events per `PutLogEvents` call.
const MAX_PUT_LOG_EVENTS: usize = 10_000;

/// Maximum batch size in bytes for `PutLogEvents`.
const MAX_BATCH_SIZE_BYTES: usize = 1_048_576;

/// Maximum age for log events (14 days in milliseconds).
const MAX_EVENT_AGE_MS: i64 = 14 * 24 * 60 * 60 * 1000;

/// Maximum future tolerance for log events (2 hours in milliseconds).
const MAX_FUTURE_MS: i64 = 2 * 60 * 60 * 1000;

/// Default page size for list/describe operations.
const DEFAULT_PAGE_SIZE: usize = 50;

/// Resolve the page size from an optional limit, clamping to `DEFAULT_PAGE_SIZE`.
fn resolve_page_size(limit: Option<i32>) -> usize {
    limit.map_or(DEFAULT_PAGE_SIZE, |l| {
        usize::try_from(l.max(0))
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .min(DEFAULT_PAGE_SIZE)
    })
}

// ---------------------------------------------------------------------------
// Internal state types
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct LogGroupRecord {
    name: String,
    arn: String,
    creation_time: i64,
    retention_in_days: Option<i32>,
    kms_key_id: Option<String>,
    tags: HashMap<String, String>,
    streams: HashMap<String, LogStreamRecord>,
    metric_filters: HashMap<String, MetricFilterRecord>,
    subscription_filters: HashMap<String, SubscriptionFilterRecord>,
    stored_bytes: i64,
}

#[derive(Debug)]
struct LogStreamRecord {
    name: String,
    arn: String,
    creation_time: i64,
    first_event_timestamp: Option<i64>,
    last_event_timestamp: Option<i64>,
    last_ingestion_time: Option<i64>,
    upload_sequence_token: String,
    events: Vec<StoredLogEvent>,
}

#[derive(Debug)]
struct StoredLogEvent {
    timestamp: i64,
    message: String,
    ingestion_time: i64,
}

#[derive(Debug)]
struct MetricFilterRecord {
    name: String,
    filter_pattern: String,
    log_group_name: String,
    metric_transformations: serde_json::Value,
    creation_time: i64,
}

#[derive(Debug)]
struct SubscriptionFilterRecord {
    name: String,
    filter_pattern: String,
    log_group_name: String,
    destination_arn: String,
    role_arn: Option<String>,
    distribution: Option<String>,
    creation_time: i64,
}

#[derive(Debug)]
struct ResourcePolicyRecord {
    name: String,
    policy_document: String,
    last_updated_time: i64,
}

#[derive(Debug)]
struct DestinationRecord {
    name: String,
    target_arn: String,
    role_arn: String,
    access_policy: Option<String>,
    arn: String,
    creation_time: i64,
}

#[derive(Debug)]
struct QueryDefinitionRecord {
    query_definition_id: String,
    name: String,
    query_string: String,
    log_group_names: Vec<String>,
    last_modified: i64,
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// CloudWatch Logs provider with `DashMap`-based in-memory storage.
#[derive(Debug)]
pub struct RustStackLogs {
    config: LogsConfig,
    groups: DashMap<String, LogGroupRecord>,
    resource_policies: DashMap<String, ResourcePolicyRecord>,
    destinations: DashMap<String, DestinationRecord>,
    query_definitions: DashMap<String, QueryDefinitionRecord>,
}

impl RustStackLogs {
    /// Create a new provider with the given configuration.
    #[must_use]
    pub fn new(config: LogsConfig) -> Self {
        Self {
            config,
            groups: DashMap::new(),
            resource_policies: DashMap::new(),
            destinations: DashMap::new(),
            query_definitions: DashMap::new(),
        }
    }

    fn log_group_arn(&self, name: &str) -> String {
        format!(
            "arn:aws:logs:{}:{}:log-group:{}",
            self.config.default_region, self.config.account_id, name,
        )
    }

    fn log_stream_arn(&self, group_name: &str, stream_name: &str) -> String {
        format!(
            "arn:aws:logs:{}:{}:log-group:{}:log-stream:{}",
            self.config.default_region, self.config.account_id, group_name, stream_name,
        )
    }

    fn destination_arn(&self, name: &str) -> String {
        format!(
            "arn:aws:logs:{}:{}:destination:{}",
            self.config.default_region, self.config.account_id, name,
        )
    }

    fn now_millis() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    fn validate_log_group_name(name: &str) -> Result<(), LogsError> {
        if name.is_empty() || name.len() > 512 {
            return Err(LogsError::with_message(
                LogsErrorCode::InvalidParameterException,
                format!(
                    "Log group name must be between 1 and 512 characters, got {}",
                    name.len()
                ),
            ));
        }
        // Pattern: [\.\-_/#A-Za-z0-9]+
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/#".contains(c))
        {
            return Err(LogsError::with_message(
                LogsErrorCode::InvalidParameterException,
                format!("Log group name does not match pattern [.\\-_/#A-Za-z0-9]+: {name}"),
            ));
        }
        Ok(())
    }

    fn validate_log_stream_name(name: &str) -> Result<(), LogsError> {
        if name.is_empty() || name.len() > 512 {
            return Err(LogsError::with_message(
                LogsErrorCode::InvalidParameterException,
                format!(
                    "Log stream name must be between 1 and 512 characters, got {}",
                    name.len()
                ),
            ));
        }
        if name.contains(':') || name.contains('*') {
            return Err(LogsError::with_message(
                LogsErrorCode::InvalidParameterException,
                format!("Log stream name must not contain ':' or '*': {name}"),
            ));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Phase 0: Log Group Management
    // -----------------------------------------------------------------------

    pub fn handle_create_log_group(
        &self,
        input: &CreateLogGroupInput,
    ) -> Result<serde_json::Value, LogsError> {
        Self::validate_log_group_name(&input.log_group_name)?;

        match self.groups.entry(input.log_group_name.clone()) {
            Entry::Occupied(_) => Err(LogsError::with_message(
                LogsErrorCode::ResourceAlreadyExistsException,
                format!(
                    "The specified log group already exists: {}",
                    input.log_group_name
                ),
            )),
            Entry::Vacant(entry) => {
                let arn = self.log_group_arn(&input.log_group_name);
                entry.insert(LogGroupRecord {
                    name: input.log_group_name.clone(),
                    arn,
                    creation_time: Self::now_millis(),
                    retention_in_days: None,
                    kms_key_id: input.kms_key_id.clone(),
                    tags: input.tags.clone(),
                    streams: HashMap::new(),
                    metric_filters: HashMap::new(),
                    subscription_filters: HashMap::new(),
                    stored_bytes: 0,
                });
                Ok(serde_json::json!({}))
            }
        }
    }

    pub fn handle_delete_log_group(
        &self,
        input: &DeleteLogGroupInput,
    ) -> Result<serde_json::Value, LogsError> {
        self.groups.remove(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;
        Ok(serde_json::json!({}))
    }

    pub fn handle_describe_log_groups(
        &self,
        input: &DescribeLogGroupsInput,
    ) -> Result<DescribeLogGroupsResponse, LogsError> {
        let page_size = resolve_page_size(input.limit);
        let mut groups: Vec<LogGroup> = Vec::new();

        for entry in &self.groups {
            let g = entry.value();

            // Filter by prefix
            if let Some(ref prefix) = input.log_group_name_prefix {
                if !g.name.starts_with(prefix.as_str()) {
                    continue;
                }
            }

            // Filter by pattern (simple substring match)
            if let Some(ref pattern) = input.log_group_name_pattern {
                if !g.name.contains(pattern.as_str()) {
                    continue;
                }
            }

            groups.push(LogGroup {
                log_group_name: Some(g.name.clone()),
                log_group_arn: Some(format!("{}:*", g.arn)),
                arn: Some(format!("{}:*", g.arn)),
                creation_time: Some(g.creation_time),
                retention_in_days: g.retention_in_days,
                kms_key_id: g.kms_key_id.clone(),
                metric_filter_count: Some(
                    i32::try_from(g.metric_filters.len()).unwrap_or(i32::MAX),
                ),
                stored_bytes: Some(g.stored_bytes),
                ..LogGroup::default()
            });
        }

        groups.sort_by(|a, b| a.log_group_name.cmp(&b.log_group_name));

        // Pagination
        let start = input
            .next_token
            .as_ref()
            .and_then(|t| t.parse::<usize>().ok())
            .unwrap_or(0);

        let page: Vec<LogGroup> = groups.into_iter().skip(start).take(page_size).collect();
        let next_token = if page.len() == page_size {
            Some((start + page_size).to_string())
        } else {
            None
        };

        Ok(DescribeLogGroupsResponse {
            log_groups: page,
            next_token,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 0: Log Stream Management
    // -----------------------------------------------------------------------

    pub fn handle_create_log_stream(
        &self,
        input: &CreateLogStreamInput,
    ) -> Result<serde_json::Value, LogsError> {
        Self::validate_log_stream_name(&input.log_stream_name)?;

        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;

        if group.streams.contains_key(&input.log_stream_name) {
            return Err(LogsError::with_message(
                LogsErrorCode::ResourceAlreadyExistsException,
                format!(
                    "The specified log stream already exists: {}",
                    input.log_stream_name,
                ),
            ));
        }

        let arn = self.log_stream_arn(&input.log_group_name, &input.log_stream_name);
        group.streams.insert(
            input.log_stream_name.clone(),
            LogStreamRecord {
                name: input.log_stream_name.clone(),
                arn,
                creation_time: Self::now_millis(),
                first_event_timestamp: None,
                last_event_timestamp: None,
                last_ingestion_time: None,
                upload_sequence_token: uuid::Uuid::new_v4().to_string(),
                events: Vec::new(),
            },
        );

        Ok(serde_json::json!({}))
    }

    pub fn handle_delete_log_stream(
        &self,
        input: &DeleteLogStreamInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;

        group
            .streams
            .remove(&input.log_stream_name)
            .ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::ResourceNotFoundException,
                    format!(
                        "The specified log stream does not exist: {}",
                        input.log_stream_name,
                    ),
                )
            })?;

        Ok(serde_json::json!({}))
    }

    pub fn handle_describe_log_streams(
        &self,
        input: &DescribeLogStreamsInput,
    ) -> Result<DescribeLogStreamsResponse, LogsError> {
        let log_group_name = input
            .log_group_name
            .as_deref()
            .or(input.log_group_identifier.as_deref())
            .ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::InvalidParameterException,
                    "Either logGroupName or logGroupIdentifier must be specified",
                )
            })?;

        let group = self.groups.get(log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!("The specified log group does not exist: {log_group_name}"),
            )
        })?;

        let page_size = resolve_page_size(input.limit);
        let mut streams: Vec<LogStream> = group
            .streams
            .values()
            .filter(|s| {
                if let Some(ref prefix) = input.log_stream_name_prefix {
                    s.name.starts_with(prefix.as_str())
                } else {
                    true
                }
            })
            .map(|s| LogStream {
                log_stream_name: Some(s.name.clone()),
                arn: Some(s.arn.clone()),
                creation_time: Some(s.creation_time),
                first_event_timestamp: s.first_event_timestamp,
                last_event_timestamp: s.last_event_timestamp,
                last_ingestion_time: s.last_ingestion_time,
                upload_sequence_token: Some(s.upload_sequence_token.clone()),
                stored_bytes: Some(
                    i64::try_from(s.events.iter().map(|e| e.message.len()).sum::<usize>())
                        .unwrap_or(0),
                ),
            })
            .collect();

        // Sort by order
        let descending = input.descending.unwrap_or(false);
        match input.order_by {
            Some(ruststack_logs_model::types::OrderBy::LastEventTime) => {
                streams.sort_by(|a, b| {
                    let ta = a.last_event_timestamp.unwrap_or(0);
                    let tb = b.last_event_timestamp.unwrap_or(0);
                    if descending { tb.cmp(&ta) } else { ta.cmp(&tb) }
                });
            }
            _ => {
                streams.sort_by(|a, b| {
                    let cmp = a.log_stream_name.cmp(&b.log_stream_name);
                    if descending { cmp.reverse() } else { cmp }
                });
            }
        }

        let start = input
            .next_token
            .as_ref()
            .and_then(|t| t.parse::<usize>().ok())
            .unwrap_or(0);

        let page: Vec<LogStream> = streams.into_iter().skip(start).take(page_size).collect();
        let next_token = if page.len() == page_size {
            Some((start + page_size).to_string())
        } else {
            None
        };

        Ok(DescribeLogStreamsResponse {
            log_streams: page,
            next_token,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 0: Log Events
    // -----------------------------------------------------------------------

    pub fn handle_put_log_events(
        &self,
        input: &PutLogEventsInput,
    ) -> Result<PutLogEventsResponse, LogsError> {
        if input.log_events.len() > MAX_PUT_LOG_EVENTS {
            return Err(LogsError::with_message(
                LogsErrorCode::InvalidParameterException,
                format!(
                    "Log events in a single PutLogEvents request cannot exceed {MAX_PUT_LOG_EVENTS}"
                ),
            ));
        }

        // Check batch size
        let total_size: usize = input
            .log_events
            .iter()
            .map(|e| e.message.len() + 26) // 26 bytes overhead per event
            .sum();
        if total_size > MAX_BATCH_SIZE_BYTES {
            return Err(LogsError::with_message(
                LogsErrorCode::InvalidParameterException,
                "The batch of log events in a single PutLogEvents request cannot exceed 1 MB",
            ));
        }

        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;

        let stream = group
            .streams
            .get_mut(&input.log_stream_name)
            .ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::ResourceNotFoundException,
                    format!(
                        "The specified log stream does not exist: {}",
                        input.log_stream_name,
                    ),
                )
            })?;

        let now = Self::now_millis();

        // Validate timestamps
        for event in &input.log_events {
            if now - event.timestamp > MAX_EVENT_AGE_MS {
                return Err(LogsError::with_message(
                    LogsErrorCode::InvalidParameterException,
                    "Log event timestamp is too old (more than 14 days ago)",
                ));
            }
            if event.timestamp - now > MAX_FUTURE_MS {
                return Err(LogsError::with_message(
                    LogsErrorCode::InvalidParameterException,
                    "Log event timestamp is too far in the future (more than 2 hours)",
                ));
            }
        }

        // Sort events by timestamp and append
        let mut sorted_events: Vec<&ruststack_logs_model::types::InputLogEvent> =
            input.log_events.iter().collect();
        sorted_events.sort_by_key(|e| e.timestamp);

        let ingestion_time = now;
        for event in &sorted_events {
            stream.events.push(StoredLogEvent {
                timestamp: event.timestamp,
                message: event.message.clone(),
                ingestion_time,
            });
        }

        // Update stream metadata
        if let Some(first) = sorted_events.first() {
            if stream.first_event_timestamp.is_none()
                || first.timestamp < stream.first_event_timestamp.unwrap_or(i64::MAX)
            {
                stream.first_event_timestamp = Some(first.timestamp);
            }
        }
        if let Some(last) = sorted_events.last() {
            stream.last_event_timestamp = Some(last.timestamp);
        }
        stream.last_ingestion_time = Some(ingestion_time);

        // Update sequence token
        let new_token = uuid::Uuid::new_v4().to_string();
        stream.upload_sequence_token.clone_from(&new_token);

        // Update stored bytes
        let added_bytes: i64 = sorted_events
            .iter()
            .map(|e| i64::try_from(e.message.len()).unwrap_or(0))
            .sum();
        group.stored_bytes += added_bytes;

        Ok(PutLogEventsResponse {
            next_sequence_token: Some(new_token),
            rejected_log_events_info: None,
            rejected_entity_info: None,
        })
    }

    pub fn handle_get_log_events(
        &self,
        input: &GetLogEventsInput,
    ) -> Result<GetLogEventsResponse, LogsError> {
        let log_group_name = input
            .log_group_name
            .as_deref()
            .or(input.log_group_identifier.as_deref())
            .ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::InvalidParameterException,
                    "Either logGroupName or logGroupIdentifier must be specified",
                )
            })?;

        let group = self.groups.get(log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!("The specified log group does not exist: {log_group_name}"),
            )
        })?;

        let stream = group.streams.get(&input.log_stream_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log stream does not exist: {}",
                    input.log_stream_name,
                ),
            )
        })?;

        let limit = input
            .limit
            .map_or(10_000, |l| usize::try_from(l.max(1)).unwrap_or(10_000));

        let start_time = input.start_time.unwrap_or(0);
        let end_time = input.end_time.unwrap_or(i64::MAX);
        let start_from_head = input.start_from_head.unwrap_or(false);

        let filtered: Vec<&StoredLogEvent> = stream
            .events
            .iter()
            .filter(|e| e.timestamp >= start_time && e.timestamp <= end_time)
            .collect();

        let events: Vec<OutputLogEvent> = if start_from_head {
            filtered
                .iter()
                .take(limit)
                .map(|e| OutputLogEvent {
                    timestamp: Some(e.timestamp),
                    message: Some(e.message.clone()),
                    ingestion_time: Some(e.ingestion_time),
                })
                .collect()
        } else {
            // From the tail (most recent first)
            let skip = if filtered.len() > limit {
                filtered.len() - limit
            } else {
                0
            };
            filtered
                .iter()
                .skip(skip)
                .map(|e| OutputLogEvent {
                    timestamp: Some(e.timestamp),
                    message: Some(e.message.clone()),
                    ingestion_time: Some(e.ingestion_time),
                })
                .collect()
        };

        // Tokens for forward/backward pagination
        let forward_token = format!("f/{}", uuid::Uuid::new_v4());
        let backward_token = format!("b/{}", uuid::Uuid::new_v4());

        Ok(GetLogEventsResponse {
            events,
            next_forward_token: Some(forward_token),
            next_backward_token: Some(backward_token),
        })
    }

    pub fn handle_filter_log_events(
        &self,
        input: &FilterLogEventsInput,
    ) -> Result<FilterLogEventsResponse, LogsError> {
        let log_group_name = input
            .log_group_name
            .as_deref()
            .or(input.log_group_identifier.as_deref())
            .ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::InvalidParameterException,
                    "Either logGroupName or logGroupIdentifier must be specified",
                )
            })?;

        let group = self.groups.get(log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!("The specified log group does not exist: {log_group_name}"),
            )
        })?;

        let start_time = input.start_time.unwrap_or(0);
        let end_time = input.end_time.unwrap_or(i64::MAX);
        let limit = resolve_page_size(input.limit);
        let filter_pattern = input.filter_pattern.as_deref().unwrap_or("");

        let mut events: Vec<FilteredLogEvent> = Vec::new();
        let mut searched_streams: Vec<SearchedLogStream> = Vec::new();

        for (stream_name, stream) in &group.streams {
            // Filter by specific stream names or prefix
            if !input.log_stream_names.is_empty() && !input.log_stream_names.contains(stream_name) {
                continue;
            }
            if let Some(ref prefix) = input.log_stream_name_prefix {
                if !stream_name.starts_with(prefix.as_str()) {
                    continue;
                }
            }

            let mut searched_completely = true;
            for event in &stream.events {
                if event.timestamp < start_time || event.timestamp > end_time {
                    continue;
                }
                // Empty pattern matches all
                if !filter_pattern.is_empty() && !event.message.contains(filter_pattern) {
                    continue;
                }

                events.push(FilteredLogEvent {
                    log_stream_name: Some(stream_name.clone()),
                    timestamp: Some(event.timestamp),
                    message: Some(event.message.clone()),
                    ingestion_time: Some(event.ingestion_time),
                    event_id: Some(uuid::Uuid::new_v4().to_string()),
                });

                if events.len() >= limit {
                    searched_completely = false;
                    break;
                }
            }

            searched_streams.push(SearchedLogStream {
                log_stream_name: Some(stream_name.clone()),
                searched_completely: Some(searched_completely),
            });
        }

        // Sort by timestamp
        events.sort_by_key(|e| e.timestamp);
        events.truncate(limit);

        Ok(FilterLogEventsResponse {
            events,
            searched_log_streams: searched_streams,
            next_token: None,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 1: Retention Policy
    // -----------------------------------------------------------------------

    pub fn handle_put_retention_policy(
        &self,
        input: &PutRetentionPolicyInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;
        group.retention_in_days = Some(input.retention_in_days);
        Ok(serde_json::json!({}))
    }

    pub fn handle_delete_retention_policy(
        &self,
        input: &DeleteRetentionPolicyInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;
        group.retention_in_days = None;
        Ok(serde_json::json!({}))
    }

    // -----------------------------------------------------------------------
    // Phase 1: Tagging (legacy log group API)
    // -----------------------------------------------------------------------

    pub fn handle_tag_log_group(
        &self,
        input: &TagLogGroupInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;
        group.tags.extend(input.tags.clone());
        Ok(serde_json::json!({}))
    }

    pub fn handle_untag_log_group(
        &self,
        input: &UntagLogGroupInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;
        for key in &input.tags {
            group.tags.remove(key);
        }
        Ok(serde_json::json!({}))
    }

    pub fn handle_list_tags_log_group(
        &self,
        input: &ListTagsLogGroupInput,
    ) -> Result<ListTagsLogGroupResponse, LogsError> {
        let group = self.groups.get(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;
        Ok(ListTagsLogGroupResponse {
            tags: group.tags.clone(),
        })
    }

    // -----------------------------------------------------------------------
    // Phase 1: Tagging (ARN-based API)
    // -----------------------------------------------------------------------

    pub fn handle_tag_resource(
        &self,
        input: &TagResourceInput,
    ) -> Result<serde_json::Value, LogsError> {
        let group_name = Self::resolve_log_group_name_from_arn(&input.resource_arn)?;
        let mut group = self.groups.get_mut(&group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified resource does not exist: {}",
                    input.resource_arn
                ),
            )
        })?;
        group.tags.extend(input.tags.clone());
        Ok(serde_json::json!({}))
    }

    pub fn handle_untag_resource(
        &self,
        input: &UntagResourceInput,
    ) -> Result<serde_json::Value, LogsError> {
        let group_name = Self::resolve_log_group_name_from_arn(&input.resource_arn)?;
        let mut group = self.groups.get_mut(&group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified resource does not exist: {}",
                    input.resource_arn
                ),
            )
        })?;
        for key in &input.tag_keys {
            group.tags.remove(key);
        }
        Ok(serde_json::json!({}))
    }

    pub fn handle_list_tags_for_resource(
        &self,
        input: &ListTagsForResourceInput,
    ) -> Result<ListTagsForResourceResponse, LogsError> {
        let group_name = Self::resolve_log_group_name_from_arn(&input.resource_arn)?;
        let group = self.groups.get(&group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified resource does not exist: {}",
                    input.resource_arn
                ),
            )
        })?;
        Ok(ListTagsForResourceResponse {
            tags: group.tags.clone(),
        })
    }

    fn resolve_log_group_name_from_arn(arn: &str) -> Result<String, LogsError> {
        // ARN format: arn:aws:logs:{region}:{account}:log-group:{name}:*
        // or:         arn:aws:logs:{region}:{account}:log-group:{name}
        let parts: Vec<&str> = arn.split(':').collect();
        if parts.len() < 7 || parts[2] != "logs" || parts[5] != "log-group" {
            return Err(LogsError::with_message(
                LogsErrorCode::InvalidParameterException,
                format!("Invalid ARN: {arn}"),
            ));
        }
        let name = parts[6];
        // Remove trailing :* if present
        Ok(name.to_owned())
    }

    // -----------------------------------------------------------------------
    // Phase 1: Resource Policies
    // -----------------------------------------------------------------------

    pub fn handle_put_resource_policy(
        &self,
        input: &PutResourcePolicyInput,
    ) -> Result<PutResourcePolicyResponse, LogsError> {
        let policy_name = input.policy_name.clone().unwrap_or_default();
        let policy_document = input.policy_document.clone().unwrap_or_default();
        let now = Self::now_millis();

        self.resource_policies.insert(
            policy_name.clone(),
            ResourcePolicyRecord {
                name: policy_name.clone(),
                policy_document: policy_document.clone(),
                last_updated_time: now,
            },
        );

        Ok(PutResourcePolicyResponse {
            resource_policy: Some(ResourcePolicy {
                policy_name: Some(policy_name),
                policy_document: Some(policy_document),
                last_updated_time: Some(now),
                ..ResourcePolicy::default()
            }),
            revision_id: None,
        })
    }

    pub fn handle_delete_resource_policy(
        &self,
        input: &DeleteResourcePolicyInput,
    ) -> Result<serde_json::Value, LogsError> {
        let policy_name = input.policy_name.as_deref().unwrap_or("");
        self.resource_policies.remove(policy_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!("The specified resource policy does not exist: {policy_name}"),
            )
        })?;
        Ok(serde_json::json!({}))
    }

    pub fn handle_describe_resource_policies(
        &self,
        _input: &DescribeResourcePoliciesInput,
    ) -> Result<DescribeResourcePoliciesResponse, LogsError> {
        let policies: Vec<ResourcePolicy> = self
            .resource_policies
            .iter()
            .map(|entry| {
                let rp = entry.value();
                ResourcePolicy {
                    policy_name: Some(rp.name.clone()),
                    policy_document: Some(rp.policy_document.clone()),
                    last_updated_time: Some(rp.last_updated_time),
                    ..ResourcePolicy::default()
                }
            })
            .collect();

        Ok(DescribeResourcePoliciesResponse {
            resource_policies: policies,
            next_token: None,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 2: Metric Filters
    // -----------------------------------------------------------------------

    pub fn handle_put_metric_filter(
        &self,
        input: &PutMetricFilterInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;

        let transformations = serde_json::to_value(&input.metric_transformations)
            .unwrap_or_else(|_| serde_json::json!([]));

        group.metric_filters.insert(
            input.filter_name.clone(),
            MetricFilterRecord {
                name: input.filter_name.clone(),
                filter_pattern: input.filter_pattern.clone(),
                log_group_name: input.log_group_name.clone(),
                metric_transformations: transformations,
                creation_time: Self::now_millis(),
            },
        );

        Ok(serde_json::json!({}))
    }

    pub fn handle_delete_metric_filter(
        &self,
        input: &DeleteMetricFilterInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;

        group
            .metric_filters
            .remove(&input.filter_name)
            .ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::ResourceNotFoundException,
                    format!(
                        "The specified metric filter does not exist: {}",
                        input.filter_name
                    ),
                )
            })?;

        Ok(serde_json::json!({}))
    }

    pub fn handle_describe_metric_filters(
        &self,
        input: &DescribeMetricFiltersInput,
    ) -> Result<DescribeMetricFiltersResponse, LogsError> {
        let mut filters: Vec<MetricFilter> = Vec::new();

        if let Some(ref group_name) = input.log_group_name {
            let group = self.groups.get(group_name).ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::ResourceNotFoundException,
                    format!("The specified log group does not exist: {group_name}"),
                )
            })?;

            for mf in group.metric_filters.values() {
                if let Some(ref prefix) = input.filter_name_prefix {
                    if !mf.name.starts_with(prefix.as_str()) {
                        continue;
                    }
                }
                let transformations: Vec<ruststack_logs_model::types::MetricTransformation> =
                    serde_json::from_value(mf.metric_transformations.clone()).unwrap_or_default();
                filters.push(MetricFilter {
                    filter_name: Some(mf.name.clone()),
                    filter_pattern: Some(mf.filter_pattern.clone()),
                    log_group_name: Some(mf.log_group_name.clone()),
                    metric_transformations: transformations,
                    creation_time: Some(mf.creation_time),
                    ..MetricFilter::default()
                });
            }
        }

        Ok(DescribeMetricFiltersResponse {
            metric_filters: filters,
            next_token: None,
        })
    }

    pub fn handle_test_metric_filter(
        &self,
        input: &TestMetricFilterInput,
    ) -> Result<TestMetricFilterResponse, LogsError> {
        // Simple implementation: if pattern is empty, match all; otherwise substring match
        let matches: Vec<MetricFilterMatchRecord> = input
            .log_event_messages
            .iter()
            .enumerate()
            .filter(|(_, msg)| {
                input.filter_pattern.is_empty() || msg.contains(&input.filter_pattern)
            })
            .map(|(i, msg)| MetricFilterMatchRecord {
                event_number: Some(i64::try_from(i + 1).unwrap_or(0)),
                event_message: Some(msg.clone()),
                extracted_values: HashMap::new(),
            })
            .collect();

        Ok(TestMetricFilterResponse { matches })
    }

    // -----------------------------------------------------------------------
    // Phase 2: Subscription Filters
    // -----------------------------------------------------------------------

    pub fn handle_put_subscription_filter(
        &self,
        input: &PutSubscriptionFilterInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;

        group.subscription_filters.insert(
            input.filter_name.clone(),
            SubscriptionFilterRecord {
                name: input.filter_name.clone(),
                filter_pattern: input.filter_pattern.clone(),
                log_group_name: input.log_group_name.clone(),
                destination_arn: input.destination_arn.clone(),
                role_arn: input.role_arn.clone(),
                distribution: input.distribution.as_ref().map(|d| d.as_str().to_owned()),
                creation_time: Self::now_millis(),
            },
        );

        Ok(serde_json::json!({}))
    }

    pub fn handle_delete_subscription_filter(
        &self,
        input: &DeleteSubscriptionFilterInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut group = self.groups.get_mut(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name
                ),
            )
        })?;

        group
            .subscription_filters
            .remove(&input.filter_name)
            .ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::ResourceNotFoundException,
                    format!(
                        "The specified subscription filter does not exist: {}",
                        input.filter_name,
                    ),
                )
            })?;

        Ok(serde_json::json!({}))
    }

    pub fn handle_describe_subscription_filters(
        &self,
        input: &DescribeSubscriptionFiltersInput,
    ) -> Result<DescribeSubscriptionFiltersResponse, LogsError> {
        let group = self.groups.get(&input.log_group_name).ok_or_else(|| {
            LogsError::with_message(
                LogsErrorCode::ResourceNotFoundException,
                format!(
                    "The specified log group does not exist: {}",
                    input.log_group_name,
                ),
            )
        })?;

        let filters: Vec<SubscriptionFilter> = group
            .subscription_filters
            .values()
            .filter(|sf| {
                if let Some(ref prefix) = input.filter_name_prefix {
                    sf.name.starts_with(prefix.as_str())
                } else {
                    true
                }
            })
            .map(|sf| SubscriptionFilter {
                filter_name: Some(sf.name.clone()),
                filter_pattern: Some(sf.filter_pattern.clone()),
                log_group_name: Some(sf.log_group_name.clone()),
                destination_arn: Some(sf.destination_arn.clone()),
                role_arn: sf.role_arn.clone(),
                distribution: sf
                    .distribution
                    .as_ref()
                    .map(|d| ruststack_logs_model::types::Distribution::from(d.as_str())),
                creation_time: Some(sf.creation_time),
                ..SubscriptionFilter::default()
            })
            .collect();

        Ok(DescribeSubscriptionFiltersResponse {
            subscription_filters: filters,
            next_token: None,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 3: Destinations
    // -----------------------------------------------------------------------

    pub fn handle_put_destination(
        &self,
        input: &PutDestinationInput,
    ) -> Result<PutDestinationResponse, LogsError> {
        let arn = self.destination_arn(&input.destination_name);
        let now = Self::now_millis();

        self.destinations.insert(
            input.destination_name.clone(),
            DestinationRecord {
                name: input.destination_name.clone(),
                target_arn: input.target_arn.clone(),
                role_arn: input.role_arn.clone(),
                access_policy: None,
                arn: arn.clone(),
                creation_time: now,
            },
        );

        Ok(PutDestinationResponse {
            destination: Some(Destination {
                destination_name: Some(input.destination_name.clone()),
                target_arn: Some(input.target_arn.clone()),
                role_arn: Some(input.role_arn.clone()),
                arn: Some(arn),
                creation_time: Some(now),
                access_policy: None,
            }),
        })
    }

    pub fn handle_put_destination_policy(
        &self,
        input: &PutDestinationPolicyInput,
    ) -> Result<serde_json::Value, LogsError> {
        let mut dest = self
            .destinations
            .get_mut(&input.destination_name)
            .ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::ResourceNotFoundException,
                    format!(
                        "The specified destination does not exist: {}",
                        input.destination_name,
                    ),
                )
            })?;

        dest.access_policy = Some(input.access_policy.clone());
        Ok(serde_json::json!({}))
    }

    pub fn handle_delete_destination(
        &self,
        input: &DeleteDestinationInput,
    ) -> Result<serde_json::Value, LogsError> {
        self.destinations
            .remove(&input.destination_name)
            .ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::ResourceNotFoundException,
                    format!(
                        "The specified destination does not exist: {}",
                        input.destination_name,
                    ),
                )
            })?;
        Ok(serde_json::json!({}))
    }

    pub fn handle_describe_destinations(
        &self,
        input: &DescribeDestinationsInput,
    ) -> Result<DescribeDestinationsResponse, LogsError> {
        let destinations: Vec<Destination> = self
            .destinations
            .iter()
            .filter(|entry| {
                if let Some(ref prefix) = input.destination_name_prefix {
                    entry.value().name.starts_with(prefix.as_str())
                } else {
                    true
                }
            })
            .map(|entry| {
                let d = entry.value();
                Destination {
                    destination_name: Some(d.name.clone()),
                    target_arn: Some(d.target_arn.clone()),
                    role_arn: Some(d.role_arn.clone()),
                    access_policy: d.access_policy.clone(),
                    arn: Some(d.arn.clone()),
                    creation_time: Some(d.creation_time),
                }
            })
            .collect();

        Ok(DescribeDestinationsResponse {
            destinations,
            next_token: None,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 3: Query operations (stubs)
    // -----------------------------------------------------------------------

    pub fn handle_start_query(
        &self,
        _input: &StartQueryInput,
    ) -> Result<StartQueryResponse, LogsError> {
        let query_id = uuid::Uuid::new_v4().to_string();
        Ok(StartQueryResponse {
            query_id: Some(query_id),
        })
    }

    pub fn handle_stop_query(
        &self,
        _input: &StopQueryInput,
    ) -> Result<StopQueryResponse, LogsError> {
        Ok(StopQueryResponse {
            success: Some(true),
        })
    }

    pub fn handle_get_query_results(
        &self,
        _input: &GetQueryResultsInput,
    ) -> Result<GetQueryResultsResponse, LogsError> {
        Ok(GetQueryResultsResponse {
            results: Vec::new(),
            statistics: Some(QueryStatistics {
                records_matched: Some(0.0),
                records_scanned: Some(0.0),
                bytes_scanned: Some(0.0),
                ..QueryStatistics::default()
            }),
            status: Some(QueryStatus::Complete),
            ..GetQueryResultsResponse::default()
        })
    }

    pub fn handle_describe_queries(
        &self,
        _input: &DescribeQueriesInput,
    ) -> Result<DescribeQueriesResponse, LogsError> {
        Ok(DescribeQueriesResponse {
            queries: Vec::new(),
            next_token: None,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 3: Query Definitions
    // -----------------------------------------------------------------------

    pub fn handle_put_query_definition(
        &self,
        input: &PutQueryDefinitionInput,
    ) -> Result<PutQueryDefinitionResponse, LogsError> {
        let id = input
            .query_definition_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        self.query_definitions.insert(
            id.clone(),
            QueryDefinitionRecord {
                query_definition_id: id.clone(),
                name: input.name.clone(),
                query_string: input.query_string.clone(),
                log_group_names: input.log_group_names.clone(),
                last_modified: Self::now_millis(),
            },
        );

        Ok(PutQueryDefinitionResponse {
            query_definition_id: Some(id),
        })
    }

    pub fn handle_delete_query_definition(
        &self,
        input: &DeleteQueryDefinitionInput,
    ) -> Result<ruststack_logs_model::output::DeleteQueryDefinitionResponse, LogsError> {
        let removed = self
            .query_definitions
            .remove(&input.query_definition_id)
            .is_some();
        Ok(
            ruststack_logs_model::output::DeleteQueryDefinitionResponse {
                success: Some(removed),
            },
        )
    }

    pub fn handle_describe_query_definitions(
        &self,
        input: &DescribeQueryDefinitionsInput,
    ) -> Result<DescribeQueryDefinitionsResponse, LogsError> {
        let defs: Vec<QueryDefinition> = self
            .query_definitions
            .iter()
            .filter(|entry| {
                if let Some(ref prefix) = input.query_definition_name_prefix {
                    entry.value().name.starts_with(prefix.as_str())
                } else {
                    true
                }
            })
            .map(|entry| {
                let qd = entry.value();
                QueryDefinition {
                    query_definition_id: Some(qd.query_definition_id.clone()),
                    name: Some(qd.name.clone()),
                    query_string: Some(qd.query_string.clone()),
                    log_group_names: qd.log_group_names.clone(),
                    last_modified: Some(qd.last_modified),
                    ..QueryDefinition::default()
                }
            })
            .collect();

        Ok(DescribeQueryDefinitionsResponse {
            query_definitions: defs,
            next_token: None,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 3: KMS Key Association
    // -----------------------------------------------------------------------

    pub fn handle_associate_kms_key(
        &self,
        input: &AssociateKmsKeyInput,
    ) -> Result<serde_json::Value, LogsError> {
        if let Some(ref group_name) = input.log_group_name {
            let mut group = self.groups.get_mut(group_name).ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::ResourceNotFoundException,
                    format!("The specified log group does not exist: {group_name}"),
                )
            })?;
            group.kms_key_id = Some(input.kms_key_id.clone());
        }
        Ok(serde_json::json!({}))
    }

    pub fn handle_disassociate_kms_key(
        &self,
        input: &DisassociateKmsKeyInput,
    ) -> Result<serde_json::Value, LogsError> {
        if let Some(ref group_name) = input.log_group_name {
            let mut group = self.groups.get_mut(group_name).ok_or_else(|| {
                LogsError::with_message(
                    LogsErrorCode::ResourceNotFoundException,
                    format!("The specified log group does not exist: {group_name}"),
                )
            })?;
            group.kms_key_id = None;
        }
        Ok(serde_json::json!({}))
    }
}
