//! EventBridge provider implementing Phase 0, Phase 1, and Phase 2 operations.
//!
//! The provider uses `DashMap` for concurrent access to event buses, keeping
//! the design simple without an actor model. Pattern matching is synchronous;
//! only delivery to targets is asynchronous.

use std::{collections::HashMap, sync::Arc};

use dashmap::{DashMap, mapref::entry::Entry};
use ruststack_events_model::{
    error::EventsError,
    input::{
        CreateEventBusInput, DeleteEventBusInput, DeleteRuleInput, DescribeEventBusInput,
        DescribeRuleInput, DisableRuleInput, EnableRuleInput, GenericInput, ListEventBusesInput,
        ListRuleNamesByTargetInput, ListRulesInput, ListTagsForResourceInput,
        ListTargetsByRuleInput, PutEventsInput, PutPermissionInput, PutRuleInput, PutTargetsInput,
        RemovePermissionInput, RemoveTargetsInput, TagResourceInput, TestEventPatternInput,
        UntagResourceInput, UpdateEventBusInput,
    },
    output::{
        CreateEventBusOutput, DeleteEventBusOutput, DeleteRuleOutput, DescribeEventBusOutput,
        DescribeRuleOutput, DisableRuleOutput, EnableRuleOutput, GenericOutput,
        ListEventBusesOutput, ListRuleNamesByTargetOutput, ListRulesOutput,
        ListTagsForResourceOutput, ListTargetsByRuleOutput, PutEventsOutput, PutPermissionOutput,
        PutRuleOutput, PutTargetsOutput, RemovePermissionOutput, RemoveTargetsOutput,
        TagResourceOutput, TestEventPatternOutput, UntagResourceOutput, UpdateEventBusOutput,
    },
    types::{EventBus, InputTransformer, PutEventsResultEntry, Rule, Tag, Target},
};

use crate::{config::EventsConfig, delivery::TargetDelivery, pattern::EventPattern};

/// Maximum number of entries per `PutEvents` call.
const MAX_PUT_EVENTS_ENTRIES: usize = 10;

/// Maximum number of targets per rule.
const MAX_TARGETS_PER_RULE: usize = 5;

/// Default page size for list operations.
const DEFAULT_PAGE_SIZE: usize = 100;

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

struct EventBusState {
    name: String,
    arn: String,
    description: Option<String>,
    policy: Option<String>,
    tags: HashMap<String, String>,
    rules: HashMap<String, RuleState>,
}

struct RuleState {
    name: String,
    arn: String,
    description: Option<String>,
    event_pattern: Option<EventPattern>,
    event_pattern_json: Option<String>,
    schedule_expression: Option<String>,
    state: String,
    role_arn: Option<String>,
    managed_by: Option<String>,
    event_bus_name: String,
    tags: HashMap<String, String>,
    targets: HashMap<String, TargetState>,
    created_at: String,
}

#[derive(Clone)]
struct TargetState {
    id: String,
    arn: String,
    role_arn: Option<String>,
    input_path: Option<String>,
    input: Option<String>,
    input_transformer: Option<InputTransformerState>,
}

#[derive(Clone)]
struct InputTransformerState {
    input_paths_map: HashMap<String, String>,
    input_template: String,
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// The EventBridge provider. Manages event buses, rules, targets, and event
/// routing using `DashMap` for concurrent access.
pub struct RustStackEvents {
    config: EventsConfig,
    buses: DashMap<String, EventBusState>,
    delivery: Arc<dyn TargetDelivery>,
    /// Phase 3: Archive metadata storage (key = archive name).
    archives: DashMap<String, serde_json::Value>,
    /// Phase 3: Connection metadata storage (key = connection name).
    connections: DashMap<String, serde_json::Value>,
    /// Phase 3: API Destination metadata storage (key = destination name).
    api_destinations: DashMap<String, serde_json::Value>,
    /// Phase 3: Endpoint metadata storage (key = endpoint name).
    endpoints: DashMap<String, serde_json::Value>,
    /// Phase 3: Replay metadata storage (key = replay name).
    replays: DashMap<String, serde_json::Value>,
}

impl std::fmt::Debug for RustStackEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RustStackEvents")
            .field("config", &self.config)
            .field("bus_count", &self.buses.len())
            .field("archive_count", &self.archives.len())
            .field("connection_count", &self.connections.len())
            .field("api_destination_count", &self.api_destinations.len())
            .field("endpoint_count", &self.endpoints.len())
            .field("replay_count", &self.replays.len())
            .finish_non_exhaustive()
    }
}

impl RustStackEvents {
    /// Create a new EventBridge provider. The default event bus is created
    /// automatically.
    #[must_use]
    pub fn new(config: EventsConfig, delivery: Arc<dyn TargetDelivery>) -> Self {
        let provider = Self {
            config,
            buses: DashMap::new(),
            delivery,
            archives: DashMap::new(),
            connections: DashMap::new(),
            api_destinations: DashMap::new(),
            endpoints: DashMap::new(),
            replays: DashMap::new(),
        };
        provider.create_default_bus();
        provider
    }

    fn create_default_bus(&self) {
        let arn = format!(
            "arn:aws:events:{}:{}:event-bus/default",
            self.config.default_region, self.config.account_id,
        );
        self.buses.insert(
            "default".to_owned(),
            EventBusState {
                name: "default".to_owned(),
                arn,
                description: None,
                policy: None,
                tags: HashMap::new(),
                rules: HashMap::new(),
            },
        );
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn resolve_bus_name(name: Option<&str>) -> &str {
        name.unwrap_or("default")
    }

    fn build_bus_arn(&self, bus_name: &str) -> String {
        format!(
            "arn:aws:events:{}:{}:event-bus/{}",
            self.config.default_region, self.config.account_id, bus_name,
        )
    }

    fn build_rule_arn(&self, bus_name: &str, rule_name: &str) -> String {
        format!(
            "arn:aws:events:{}:{}:rule/{}/{}",
            self.config.default_region, self.config.account_id, bus_name, rule_name,
        )
    }

    fn build_event_envelope(
        &self,
        source: &str,
        detail_type: &str,
        detail: &str,
        resources: &[String],
        time: Option<&str>,
        bus_name: &str,
    ) -> Result<serde_json::Value, EventsError> {
        let event_id = uuid::Uuid::new_v4().to_string();
        let time_str = time.map_or_else(|| chrono::Utc::now().to_rfc3339(), ToOwned::to_owned);

        let detail_value: serde_json::Value = serde_json::from_str(detail)
            .map_err(|e| EventsError::validation(format!("Detail is not valid JSON: {e}")))?;

        let bus_arn = self.build_bus_arn(bus_name);

        Ok(serde_json::json!({
            "version": "0",
            "id": event_id,
            "source": source,
            "account": self.config.account_id,
            "time": time_str,
            "region": self.config.default_region,
            "resources": resources,
            "detail-type": detail_type,
            "detail": detail_value,
            "event-bus-name": bus_arn,
        }))
    }

    fn apply_input_transform(target: &TargetState, event: &serde_json::Value) -> String {
        // If target has explicit Input, use it directly.
        if let Some(ref input) = target.input {
            return input.clone();
        }

        // If target has InputPath, extract from event.
        if let Some(ref input_path) = target.input_path {
            let extracted = apply_json_path(event, input_path);
            return extracted.to_string();
        }

        // If target has InputTransformer, apply it.
        if let Some(ref transformer) = target.input_transformer {
            let mut result = transformer.input_template.clone();
            for (key, path) in &transformer.input_paths_map {
                let value = apply_json_path(event, path);
                let value_str = match &value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let placeholder = format!("<{key}>");
                result = result.replace(&placeholder, &value_str);
            }
            return result;
        }

        // Default: pass the full event.
        event.to_string()
    }

    // -----------------------------------------------------------------------
    // Phase 0: Event Bus Management
    // -----------------------------------------------------------------------

    /// Handle `CreateEventBus`.
    pub fn handle_create_event_bus(
        &self,
        input: &CreateEventBusInput,
    ) -> Result<CreateEventBusOutput, EventsError> {
        let bus_name = &input.name;

        validate_event_bus_name(bus_name)?;

        if bus_name == "default" {
            return Err(EventsError::resource_already_exists(
                "Event bus default already exists.",
            ));
        }

        let arn = self.build_bus_arn(bus_name);

        let tags: HashMap<String, String> = input
            .tags
            .iter()
            .map(|t| (t.key.clone(), t.value.clone()))
            .collect();

        let description = input.description.clone();

        // Atomic check-and-insert to avoid TOCTOU race.
        match self.buses.entry(bus_name.to_owned()) {
            Entry::Occupied(_) => {
                return Err(EventsError::resource_already_exists(format!(
                    "Event bus {bus_name} already exists."
                )));
            }
            Entry::Vacant(v) => {
                v.insert(EventBusState {
                    name: bus_name.to_owned(),
                    arn: arn.clone(),
                    description: description.clone(),
                    policy: None,
                    tags,
                    rules: HashMap::new(),
                });
            }
        }

        Ok(CreateEventBusOutput {
            event_bus_arn: Some(arn),
            description,
            ..Default::default()
        })
    }

    /// Handle `DeleteEventBus`.
    pub fn handle_delete_event_bus(
        &self,
        input: &DeleteEventBusInput,
    ) -> Result<DeleteEventBusOutput, EventsError> {
        if input.name == "default" {
            return Err(EventsError::validation(
                "Cannot delete the default event bus.",
            ));
        }

        self.buses.remove(&input.name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {} does not exist.", input.name))
        })?;

        Ok(DeleteEventBusOutput {})
    }

    /// Handle `DescribeEventBus`.
    pub fn handle_describe_event_bus(
        &self,
        input: &DescribeEventBusInput,
    ) -> Result<DescribeEventBusOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.name.as_deref());

        let bus = self.buses.get(bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        Ok(DescribeEventBusOutput {
            name: Some(bus.name.clone()),
            arn: Some(bus.arn.clone()),
            description: bus.description.clone(),
            policy: bus.policy.clone(),
            ..Default::default()
        })
    }

    /// Handle `ListEventBuses`.
    pub fn handle_list_event_buses(
        &self,
        input: &ListEventBusesInput,
    ) -> Result<ListEventBusesOutput, EventsError> {
        let page_size = resolve_page_size(input.limit);

        let mut all_buses: Vec<EventBus> = self
            .buses
            .iter()
            .filter(|entry| {
                if let Some(ref prefix) = input.name_prefix {
                    entry.value().name.starts_with(prefix.as_str())
                } else {
                    true
                }
            })
            .map(|entry| {
                let bus = entry.value();
                EventBus {
                    name: Some(bus.name.clone()),
                    arn: Some(bus.arn.clone()),
                    description: bus.description.clone(),
                    policy: bus.policy.clone(),
                }
            })
            .collect();

        all_buses.sort_by(|a, b| a.name.cmp(&b.name));

        let start = if let Some(ref token) = input.next_token {
            token.parse::<usize>().unwrap_or(0)
        } else {
            0
        };

        let page: Vec<EventBus> = all_buses.into_iter().skip(start).take(page_size).collect();
        let next_token = if page.len() == page_size {
            Some((start + page_size).to_string())
        } else {
            None
        };

        Ok(ListEventBusesOutput {
            event_buses: page,
            next_token,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 0: Rule Management
    // -----------------------------------------------------------------------

    /// Handle `PutRule`.
    pub fn handle_put_rule(&self, input: PutRuleInput) -> Result<PutRuleOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        validate_rule_name(&input.name)?;

        if input.event_pattern.is_none() && input.schedule_expression.is_none() {
            return Err(EventsError::validation(
                "Either EventPattern or ScheduleExpression must be provided.",
            ));
        }

        let parsed_pattern = if let Some(ref pattern_json) = input.event_pattern {
            Some(
                crate::pattern::parse_event_pattern(pattern_json)
                    .map_err(|e| EventsError::invalid_event_pattern(e.to_string()))?,
            )
        } else {
            None
        };

        let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        let arn = self.build_rule_arn(&bus_name, &input.name);
        let now = chrono::Utc::now().to_rfc3339();

        let existing_targets = bus
            .rules
            .get(&input.name)
            .map(|r| r.targets.clone())
            .unwrap_or_default();
        let existing_created_at = bus.rules.get(&input.name).map(|r| r.created_at.clone());

        let state_str = input.state.as_deref().map_or_else(
            || "ENABLED".to_owned(),
            |s| {
                if s == "DISABLED" {
                    "DISABLED".to_owned()
                } else {
                    "ENABLED".to_owned()
                }
            },
        );

        // Preserve existing tags on update; only set tags on initial creation.
        let is_update = bus.rules.contains_key(&input.name);
        let tags: HashMap<String, String> = if is_update {
            bus.rules
                .get(&input.name)
                .map(|r| r.tags.clone())
                .unwrap_or_default()
        } else {
            input
                .tags
                .iter()
                .map(|t| (t.key.clone(), t.value.clone()))
                .collect()
        };

        let rule = RuleState {
            name: input.name.clone(),
            arn: arn.clone(),
            description: input.description,
            event_pattern: parsed_pattern,
            event_pattern_json: input.event_pattern,
            schedule_expression: input.schedule_expression,
            state: state_str,
            role_arn: input.role_arn,
            managed_by: None,
            event_bus_name: bus_name.clone(),
            tags,
            targets: existing_targets,
            created_at: existing_created_at.unwrap_or(now),
        };

        bus.rules.insert(input.name, rule);

        Ok(PutRuleOutput {
            rule_arn: Some(arn),
        })
    }

    /// Handle `DeleteRule`.
    pub fn handle_delete_rule(
        &self,
        input: &DeleteRuleInput,
    ) -> Result<DeleteRuleOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();
        let force = input.force.unwrap_or(false);

        let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        if let Some(rule) = bus.rules.get(&input.name) {
            if !rule.targets.is_empty() && !force {
                return Err(EventsError::validation(format!(
                    "Rule {} has targets. Remove targets before deleting the rule, or use Force.",
                    input.name,
                )));
            }
        } else {
            return Err(EventsError::resource_not_found(format!(
                "Rule {} does not exist on event bus {bus_name}.",
                input.name,
            )));
        }

        bus.rules.remove(&input.name);

        Ok(DeleteRuleOutput {})
    }

    /// Handle `DescribeRule`.
    pub fn handle_describe_rule(
        &self,
        input: &DescribeRuleInput,
    ) -> Result<DescribeRuleOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let bus = self.buses.get(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        let rule = bus.rules.get(&input.name).ok_or_else(|| {
            EventsError::resource_not_found(format!(
                "Rule {} does not exist on event bus {bus_name}.",
                input.name,
            ))
        })?;

        Ok(DescribeRuleOutput {
            name: Some(rule.name.clone()),
            arn: Some(rule.arn.clone()),
            event_pattern: rule.event_pattern_json.clone(),
            schedule_expression: rule.schedule_expression.clone(),
            state: Some(rule.state.clone()),
            description: rule.description.clone(),
            role_arn: rule.role_arn.clone(),
            managed_by: rule.managed_by.clone(),
            event_bus_name: Some(rule.event_bus_name.clone()),
            created_by: Some(self.config.account_id.clone()),
        })
    }

    /// Handle `ListRules`.
    pub fn handle_list_rules(
        &self,
        input: &ListRulesInput,
    ) -> Result<ListRulesOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let bus = self.buses.get(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        let page_size = resolve_page_size(input.limit);

        let mut rules: Vec<Rule> = bus
            .rules
            .values()
            .filter(|r| {
                if let Some(ref prefix) = input.name_prefix {
                    r.name.starts_with(prefix.as_str())
                } else {
                    true
                }
            })
            .map(|r| Rule {
                name: Some(r.name.clone()),
                arn: Some(r.arn.clone()),
                event_pattern: r.event_pattern_json.clone(),
                schedule_expression: r.schedule_expression.clone(),
                state: Some(r.state.clone()),
                description: r.description.clone(),
                role_arn: r.role_arn.clone(),
                managed_by: r.managed_by.clone(),
                event_bus_name: Some(r.event_bus_name.clone()),
            })
            .collect();

        rules.sort_by(|a, b| a.name.cmp(&b.name));

        let start = if let Some(ref token) = input.next_token {
            token.parse::<usize>().unwrap_or(0)
        } else {
            0
        };

        let page: Vec<Rule> = rules.into_iter().skip(start).take(page_size).collect();
        let next_token = if page.len() == page_size {
            Some((start + page_size).to_string())
        } else {
            None
        };

        Ok(ListRulesOutput {
            rules: page,
            next_token,
        })
    }

    /// Handle `EnableRule`.
    pub fn handle_enable_rule(
        &self,
        input: &EnableRuleInput,
    ) -> Result<EnableRuleOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        let rule = bus.rules.get_mut(&input.name).ok_or_else(|| {
            EventsError::resource_not_found(format!(
                "Rule {} does not exist on event bus {bus_name}.",
                input.name,
            ))
        })?;

        "ENABLED".clone_into(&mut rule.state);

        Ok(EnableRuleOutput {})
    }

    /// Handle `DisableRule`.
    pub fn handle_disable_rule(
        &self,
        input: &DisableRuleInput,
    ) -> Result<DisableRuleOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        let rule = bus.rules.get_mut(&input.name).ok_or_else(|| {
            EventsError::resource_not_found(format!(
                "Rule {} does not exist on event bus {bus_name}.",
                input.name,
            ))
        })?;

        "DISABLED".clone_into(&mut rule.state);

        Ok(DisableRuleOutput {})
    }

    // -----------------------------------------------------------------------
    // Phase 0: Target Management
    // -----------------------------------------------------------------------

    /// Handle `PutTargets`.
    pub fn handle_put_targets(
        &self,
        input: PutTargetsInput,
    ) -> Result<PutTargetsOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        let rule = bus.rules.get_mut(&input.rule).ok_or_else(|| {
            EventsError::resource_not_found(format!(
                "Rule {} does not exist on event bus {bus_name}.",
                input.rule,
            ))
        })?;

        let mut failed_entries = Vec::new();

        for target in input.targets {
            // Check target limit (count existing + new that are not updates).
            if !rule.targets.contains_key(&target.id) && rule.targets.len() >= MAX_TARGETS_PER_RULE
            {
                failed_entries.push(ruststack_events_model::types::PutTargetsResultEntry {
                    target_id: Some(target.id.clone()),
                    error_code: Some("LimitExceededException".to_owned()),
                    error_message: Some(format!(
                        "Maximum number of targets ({MAX_TARGETS_PER_RULE}) per rule exceeded.",
                    )),
                });
                continue;
            }

            let transformer = target.input_transformer.map(|it| InputTransformerState {
                input_paths_map: it.input_paths_map,
                input_template: it.input_template,
            });

            rule.targets.insert(
                target.id.clone(),
                TargetState {
                    id: target.id,
                    arn: target.arn,
                    role_arn: target.role_arn,
                    input_path: target.input_path,
                    input: target.input,
                    input_transformer: transformer,
                },
            );
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let failed_entry_count = failed_entries.len() as i32;

        Ok(PutTargetsOutput {
            failed_entry_count,
            failed_entries,
        })
    }

    /// Handle `RemoveTargets`.
    pub fn handle_remove_targets(
        &self,
        input: &RemoveTargetsInput,
    ) -> Result<RemoveTargetsOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        let rule = bus.rules.get_mut(&input.rule).ok_or_else(|| {
            EventsError::resource_not_found(format!(
                "Rule {} does not exist on event bus {bus_name}.",
                input.rule,
            ))
        })?;

        // AWS silently ignores removal of non-existent targets.
        for id in &input.ids {
            rule.targets.remove(id);
        }

        Ok(RemoveTargetsOutput {
            failed_entry_count: 0,
            failed_entries: Vec::new(),
        })
    }

    /// Handle `ListTargetsByRule`.
    pub fn handle_list_targets_by_rule(
        &self,
        input: &ListTargetsByRuleInput,
    ) -> Result<ListTargetsByRuleOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let bus = self.buses.get(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        let rule = bus.rules.get(&input.rule).ok_or_else(|| {
            EventsError::resource_not_found(format!(
                "Rule {} does not exist on event bus {bus_name}.",
                input.rule,
            ))
        })?;

        let page_size = resolve_page_size(input.limit);

        let mut targets: Vec<Target> = rule.targets.values().map(target_state_to_model).collect();

        targets.sort_by(|a, b| a.id.cmp(&b.id));

        let start = if let Some(ref token) = input.next_token {
            token.parse::<usize>().unwrap_or(0)
        } else {
            0
        };

        let page: Vec<Target> = targets.into_iter().skip(start).take(page_size).collect();
        let next_token = if page.len() == page_size {
            Some((start + page_size).to_string())
        } else {
            None
        };

        Ok(ListTargetsByRuleOutput {
            targets: page,
            next_token,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 0: Event Operations
    // -----------------------------------------------------------------------

    /// Handle `PutEvents`. Routes events through pattern matching to targets
    /// and delivers them asynchronously via spawned tasks.
    pub fn handle_put_events(
        &self,
        input: &PutEventsInput,
    ) -> Result<PutEventsOutput, EventsError> {
        if input.entries.len() > MAX_PUT_EVENTS_ENTRIES {
            return Err(EventsError::validation(format!(
                "PutEvents supports a maximum of {MAX_PUT_EVENTS_ENTRIES} entries per request.",
            )));
        }

        let mut result_entries = Vec::with_capacity(input.entries.len());
        let mut failed_count = 0i32;

        for entry in &input.entries {
            let source = entry.source.as_deref().unwrap_or("");
            let detail_type = entry.detail_type.as_deref().unwrap_or("");
            let detail = entry.detail.as_deref().unwrap_or("{}");
            let bus_name = Self::resolve_bus_name(entry.event_bus_name.as_deref()).to_owned();

            let Ok(envelope) = self.build_event_envelope(
                source,
                detail_type,
                detail,
                &entry.resources,
                entry.time.as_deref(),
                &bus_name,
            ) else {
                failed_count += 1;
                result_entries.push(PutEventsResultEntry {
                    event_id: None,
                    error_code: Some("MalformedDetail".to_owned()),
                    error_message: Some("Detail is not valid JSON.".to_owned()),
                });
                continue;
            };

            let event_id = envelope["id"].as_str().unwrap_or_default().to_owned();

            // Route through matching rules in the bus.
            if let Some(bus) = self.buses.get(&bus_name) {
                for rule in bus.rules.values() {
                    if rule.state != "ENABLED" {
                        continue;
                    }

                    let matched = if let Some(ref pattern) = rule.event_pattern {
                        crate::pattern::matches(pattern, &envelope)
                    } else {
                        // Rules with only schedule_expression do not match events.
                        false
                    };

                    if matched {
                        for target in rule.targets.values() {
                            let event_json = Self::apply_input_transform(target, &envelope);
                            let delivery = Arc::clone(&self.delivery);
                            let target_arn = target.arn.clone();
                            tokio::spawn(async move {
                                if let Err(e) = delivery.deliver(&target_arn, &event_json).await {
                                    tracing::warn!(
                                        target_arn = %target_arn,
                                        error = %e,
                                        "Failed to deliver event to target",
                                    );
                                }
                            });
                        }
                    }
                }
            }

            result_entries.push(PutEventsResultEntry {
                event_id: Some(event_id),
                error_code: None,
                error_message: None,
            });
        }

        Ok(PutEventsOutput {
            failed_entry_count: failed_count,
            entries: result_entries,
        })
    }

    /// Handle `TestEventPattern`.
    pub fn handle_test_event_pattern(
        &self,
        input: &TestEventPatternInput,
    ) -> Result<TestEventPatternOutput, EventsError> {
        let pattern = crate::pattern::parse_event_pattern(&input.event_pattern)
            .map_err(|e| EventsError::invalid_event_pattern(e.to_string()))?;

        let event: serde_json::Value = serde_json::from_str(&input.event).map_err(|e| {
            EventsError::invalid_event_pattern(format!("Event is not valid JSON: {e}"))
        })?;

        if !event.is_object() {
            return Err(EventsError::invalid_event_pattern(
                "Event must be a JSON object.",
            ));
        }

        let result = crate::pattern::matches(&pattern, &event);

        Ok(TestEventPatternOutput { result })
    }

    // -----------------------------------------------------------------------
    // Phase 1: Tagging
    // -----------------------------------------------------------------------

    /// Handle `TagResource`.
    pub fn handle_tag_resource(
        &self,
        input: &TagResourceInput,
    ) -> Result<TagResourceOutput, EventsError> {
        let arn = &input.resource_arn;

        // Try to find and tag a bus.
        if let Some(bus_name) = extract_bus_name_from_arn(arn) {
            let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
                EventsError::resource_not_found(format!("Resource {arn} does not exist."))
            })?;
            for tag in &input.tags {
                bus.tags.insert(tag.key.clone(), tag.value.clone());
            }
            return Ok(TagResourceOutput {});
        }

        // Try to find and tag a rule.
        if let Some((bus_name, rule_name)) = extract_rule_names_from_arn(arn) {
            let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
                EventsError::resource_not_found(format!("Resource {arn} does not exist."))
            })?;
            let rule = bus.rules.get_mut(&rule_name).ok_or_else(|| {
                EventsError::resource_not_found(format!("Resource {arn} does not exist."))
            })?;
            for tag in &input.tags {
                rule.tags.insert(tag.key.clone(), tag.value.clone());
            }
            return Ok(TagResourceOutput {});
        }

        Err(EventsError::resource_not_found(format!(
            "Resource {arn} does not exist.",
        )))
    }

    /// Handle `UntagResource`.
    pub fn handle_untag_resource(
        &self,
        input: &UntagResourceInput,
    ) -> Result<UntagResourceOutput, EventsError> {
        let arn = &input.resource_arn;

        if let Some(bus_name) = extract_bus_name_from_arn(arn) {
            let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
                EventsError::resource_not_found(format!("Resource {arn} does not exist."))
            })?;
            for key in &input.tag_keys {
                bus.tags.remove(key);
            }
            return Ok(UntagResourceOutput {});
        }

        if let Some((bus_name, rule_name)) = extract_rule_names_from_arn(arn) {
            let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
                EventsError::resource_not_found(format!("Resource {arn} does not exist."))
            })?;
            let rule = bus.rules.get_mut(&rule_name).ok_or_else(|| {
                EventsError::resource_not_found(format!("Resource {arn} does not exist."))
            })?;
            for key in &input.tag_keys {
                rule.tags.remove(key);
            }
            return Ok(UntagResourceOutput {});
        }

        Err(EventsError::resource_not_found(format!(
            "Resource {arn} does not exist.",
        )))
    }

    /// Handle `ListTagsForResource`.
    pub fn handle_list_tags_for_resource(
        &self,
        input: &ListTagsForResourceInput,
    ) -> Result<ListTagsForResourceOutput, EventsError> {
        let arn = &input.resource_arn;

        if let Some(bus_name) = extract_bus_name_from_arn(arn) {
            let bus = self.buses.get(&bus_name).ok_or_else(|| {
                EventsError::resource_not_found(format!("Resource {arn} does not exist."))
            })?;
            let tags: Vec<Tag> = bus
                .tags
                .iter()
                .map(|(k, v)| Tag {
                    key: k.clone(),
                    value: v.clone(),
                })
                .collect();
            return Ok(ListTagsForResourceOutput { tags });
        }

        if let Some((bus_name, rule_name)) = extract_rule_names_from_arn(arn) {
            let bus = self.buses.get(&bus_name).ok_or_else(|| {
                EventsError::resource_not_found(format!("Resource {arn} does not exist."))
            })?;
            let rule = bus.rules.get(&rule_name).ok_or_else(|| {
                EventsError::resource_not_found(format!("Resource {arn} does not exist."))
            })?;
            let tags: Vec<Tag> = rule
                .tags
                .iter()
                .map(|(k, v)| Tag {
                    key: k.clone(),
                    value: v.clone(),
                })
                .collect();
            return Ok(ListTagsForResourceOutput { tags });
        }

        Err(EventsError::resource_not_found(format!(
            "Resource {arn} does not exist.",
        )))
    }

    // -----------------------------------------------------------------------
    // Phase 1: Permissions
    // -----------------------------------------------------------------------

    /// Handle `PutPermission`. Stores permission policy on the bus without
    /// enforcement.
    pub fn handle_put_permission(
        &self,
        input: &PutPermissionInput,
    ) -> Result<PutPermissionOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        // If a full policy is provided, use it directly.
        if let Some(ref policy) = input.policy {
            bus.policy = Some(policy.clone());
            return Ok(PutPermissionOutput {});
        }

        // Otherwise, build a policy statement from the individual fields.
        let statement_id = input.statement_id.as_deref().unwrap_or("default");
        let principal = input.principal.as_deref().unwrap_or("*");
        let action = input.action.as_deref().unwrap_or("events:PutEvents");

        let statement = serde_json::json!({
            "Sid": statement_id,
            "Effect": "Allow",
            "Principal": {
                "AWS": principal,
            },
            "Action": action,
            "Resource": bus.arn,
        });

        // Merge into existing policy or create new one.
        let mut policy: serde_json::Value = if let Some(ref existing) = bus.policy {
            serde_json::from_str(existing).unwrap_or_else(|_| {
                serde_json::json!({
                    "Version": "2012-10-17",
                    "Statement": [],
                })
            })
        } else {
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": [],
            })
        };

        if let Some(statements) = policy["Statement"].as_array_mut() {
            // Replace existing statement with same Sid, or append.
            let pos = statements.iter().position(|s| s["Sid"] == statement_id);
            if let Some(idx) = pos {
                statements[idx] = statement;
            } else {
                statements.push(statement);
            }
        }

        bus.policy = Some(policy.to_string());

        Ok(PutPermissionOutput {})
    }

    /// Handle `RemovePermission`.
    pub fn handle_remove_permission(
        &self,
        input: &RemovePermissionInput,
    ) -> Result<RemovePermissionOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        if input.remove_all_permissions.unwrap_or(false) {
            bus.policy = None;
            return Ok(RemovePermissionOutput {});
        }

        if let Some(ref statement_id) = input.statement_id {
            if let Some(ref policy_str) = bus.policy {
                if let Ok(mut policy) = serde_json::from_str::<serde_json::Value>(policy_str) {
                    if let Some(statements) = policy["Statement"].as_array_mut() {
                        statements.retain(|s| s["Sid"] != *statement_id);
                        if statements.is_empty() {
                            bus.policy = None;
                        } else {
                            bus.policy = Some(policy.to_string());
                        }
                    }
                }
            }
        }

        Ok(RemovePermissionOutput {})
    }

    // -----------------------------------------------------------------------
    // Phase 1: Reverse Lookup
    // -----------------------------------------------------------------------

    /// Handle `ListRuleNamesByTarget`. Scans all rules in a bus to find those
    /// with targets matching the given ARN.
    pub fn handle_list_rule_names_by_target(
        &self,
        input: &ListRuleNamesByTargetInput,
    ) -> Result<ListRuleNamesByTargetOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.event_bus_name.as_deref()).to_owned();

        let bus = self.buses.get(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        let page_size = resolve_page_size(input.limit);

        let mut rule_names: Vec<String> = bus
            .rules
            .values()
            .filter(|rule| rule.targets.values().any(|t| t.arn == input.target_arn))
            .map(|rule| rule.name.clone())
            .collect();

        rule_names.sort();

        let start = if let Some(ref token) = input.next_token {
            token.parse::<usize>().unwrap_or(0)
        } else {
            0
        };

        let page: Vec<String> = rule_names.into_iter().skip(start).take(page_size).collect();
        let next_token = if page.len() == page_size {
            Some((start + page_size).to_string())
        } else {
            None
        };

        Ok(ListRuleNamesByTargetOutput {
            rule_names: page,
            next_token,
        })
    }

    // -----------------------------------------------------------------------
    // Phase 2: Update
    // -----------------------------------------------------------------------

    /// Handle `UpdateEventBus`.
    pub fn handle_update_event_bus(
        &self,
        input: &UpdateEventBusInput,
    ) -> Result<UpdateEventBusOutput, EventsError> {
        let bus_name = Self::resolve_bus_name(input.name.as_deref()).to_owned();

        let mut bus = self.buses.get_mut(&bus_name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Event bus {bus_name} does not exist."))
        })?;

        if let Some(ref desc) = input.description {
            bus.description = Some(desc.clone());
        }

        Ok(UpdateEventBusOutput {
            arn: Some(bus.arn.clone()),
            name: Some(bus.name.clone()),
            description: bus.description.clone(),
            ..Default::default()
        })
    }

    // -----------------------------------------------------------------------
    // Phase 3: Generic resource helpers
    // -----------------------------------------------------------------------

    fn build_resource_arn(&self, resource_type: &str, name: &str) -> String {
        format!(
            "arn:aws:events:{}:{}:{}/{}",
            self.config.default_region, self.config.account_id, resource_type, name,
        )
    }

    fn now_timestamp() -> serde_json::Value {
        // EventBridge returns timestamps as epoch seconds in JSON.
        serde_json::Value::Number(serde_json::Number::from(chrono::Utc::now().timestamp()))
    }

    /// Extract a required string field from JSON input, returning a validation error if missing.
    fn require_name(input: &serde_json::Value, field: &str) -> Result<String, EventsError> {
        input
            .get(field)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                EventsError::validation(format!(
                    "1 validation error detected: Value at '{field}' failed to satisfy \
                     constraint: Member must not be null"
                ))
            })
    }

    /// Generic create for a `DashMap<String, Value>` resource collection.
    fn generic_create(
        store: &DashMap<String, serde_json::Value>,
        name: &str,
        mut record: serde_json::Value,
        arn: &str,
        initial_state: &str,
        resource_label: &str,
    ) -> Result<serde_json::Value, EventsError> {
        let now = Self::now_timestamp();
        if let Some(obj) = record.as_object_mut() {
            obj.insert("Arn".to_owned(), serde_json::Value::String(arn.to_owned()));
            obj.insert(
                "State".to_owned(),
                serde_json::Value::String(initial_state.to_owned()),
            );
            obj.insert("CreationTime".to_owned(), now.clone());
            obj.insert("LastModifiedTime".to_owned(), now);
        }

        match store.entry(name.to_owned()) {
            Entry::Occupied(_) => Err(EventsError::resource_already_exists(format!(
                "{resource_label} {name} already exists."
            ))),
            Entry::Vacant(v) => {
                let response = record.clone();
                v.insert(record);
                Ok(response)
            }
        }
    }

    /// Generic describe: look up a resource by name.
    fn generic_describe(
        store: &DashMap<String, serde_json::Value>,
        name: &str,
        resource_label: &str,
    ) -> Result<serde_json::Value, EventsError> {
        store.get(name).map(|r| r.value().clone()).ok_or_else(|| {
            EventsError::resource_not_found(format!("{resource_label} {name} does not exist."))
        })
    }

    /// Generic delete: remove a resource by name.
    fn generic_delete(
        store: &DashMap<String, serde_json::Value>,
        name: &str,
        resource_label: &str,
    ) -> Result<serde_json::Value, EventsError> {
        store
            .remove(name)
            .map(|_| serde_json::json!({}))
            .ok_or_else(|| {
                EventsError::resource_not_found(format!("{resource_label} {name} does not exist."))
            })
    }

    /// Generic update: merge fields from input JSON into stored record.
    fn generic_update(
        store: &DashMap<String, serde_json::Value>,
        name: &str,
        updates: &serde_json::Value,
        resource_label: &str,
    ) -> Result<serde_json::Value, EventsError> {
        let mut entry = store.get_mut(name).ok_or_else(|| {
            EventsError::resource_not_found(format!("{resource_label} {name} does not exist."))
        })?;

        let now = Self::now_timestamp();
        if let (Some(stored), Some(input_obj)) = (entry.as_object_mut(), updates.as_object()) {
            for (k, v) in input_obj {
                // Skip the name/key field itself, don't overwrite ARN or creation time.
                if k == "Name"
                    || k == "ReplayName"
                    || k == "ConnectionName"
                    || k == "ArchiveName"
                    || k == "ApiDestinationName"
                    || k == "EndpointName"
                    || k == "Arn"
                    || k == "CreationTime"
                {
                    continue;
                }
                stored.insert(k.clone(), v.clone());
            }
            stored.insert("LastModifiedTime".to_owned(), now);
        }

        Ok(entry.clone())
    }

    /// Generic list: iterate all items in a collection, optionally filter by name prefix.
    fn generic_list(
        store: &DashMap<String, serde_json::Value>,
        name_prefix: Option<&str>,
        list_key: &str,
    ) -> serde_json::Value {
        let items: Vec<serde_json::Value> = store
            .iter()
            .filter(|entry| name_prefix.is_none_or(|prefix| entry.key().starts_with(prefix)))
            .map(|entry| entry.value().clone())
            .collect();

        serde_json::json!({ list_key: items })
    }

    // -----------------------------------------------------------------------
    // Phase 3: Archives
    // -----------------------------------------------------------------------

    /// Handle `CreateArchive`.
    pub fn handle_create_archive(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "ArchiveName")?;
        let arn = self.build_resource_arn("archive", &name);
        let result = Self::generic_create(
            &self.archives,
            &name,
            input.value.clone(),
            &arn,
            "ENABLED",
            "Archive",
        )?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ArchiveArn": result.get("Arn"),
                "ArchiveName": name,
                "State": "ENABLED",
                "CreationTime": result.get("CreationTime"),
            }),
        })
    }

    /// Handle `DeleteArchive`.
    pub fn handle_delete_archive(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "ArchiveName")?;
        let result = Self::generic_delete(&self.archives, &name, "Archive")?;
        Ok(GenericOutput { value: result })
    }

    /// Handle `DescribeArchive`.
    pub fn handle_describe_archive(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "ArchiveName")?;
        let stored = Self::generic_describe(&self.archives, &name, "Archive")?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ArchiveArn": stored.get("Arn"),
                "ArchiveName": name,
                "Description": stored.get("Description"),
                "EventSourceArn": stored.get("EventSourceArn"),
                "EventPattern": stored.get("EventPattern"),
                "State": stored.get("State"),
                "RetentionDays": stored.get("RetentionDays"),
                "SizeBytes": 0,
                "EventCount": 0,
                "CreationTime": stored.get("CreationTime"),
            }),
        })
    }

    /// Handle `ListArchives`.
    pub fn handle_list_archives(&self, input: &GenericInput) -> Result<GenericOutput, EventsError> {
        let prefix = input
            .value
            .get("NamePrefix")
            .and_then(serde_json::Value::as_str);
        let result = Self::generic_list(&self.archives, prefix, "Archives");
        Ok(GenericOutput { value: result })
    }

    /// Handle `UpdateArchive`.
    pub fn handle_update_archive(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "ArchiveName")?;
        let updated = Self::generic_update(&self.archives, &name, &input.value, "Archive")?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ArchiveArn": updated.get("Arn"),
                "ArchiveName": name,
                "State": updated.get("State"),
                "CreationTime": updated.get("CreationTime"),
            }),
        })
    }

    // -----------------------------------------------------------------------
    // Phase 3: Replays
    // -----------------------------------------------------------------------

    /// Handle `StartReplay`.
    pub fn handle_start_replay(&self, input: &GenericInput) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "ReplayName")?;
        let arn = self.build_resource_arn("replay", &name);
        let result = Self::generic_create(
            &self.replays,
            &name,
            input.value.clone(),
            &arn,
            "STARTING",
            "Replay",
        )?;

        // Transition immediately to RUNNING for local dev simulation.
        if let Some(mut entry) = self.replays.get_mut(&name) {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(
                    "State".to_owned(),
                    serde_json::Value::String("RUNNING".to_owned()),
                );
            }
        }

        Ok(GenericOutput {
            value: serde_json::json!({
                "ReplayArn": result.get("Arn"),
                "ReplayName": name,
                "State": "STARTING",
                "StateReason": "Replay starting",
                "ReplayStartTime": result.get("CreationTime"),
            }),
        })
    }

    /// Handle `CancelReplay`.
    pub fn handle_cancel_replay(&self, input: &GenericInput) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "ReplayName")?;

        let mut entry = self.replays.get_mut(&name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Replay {name} does not exist."))
        })?;

        if let Some(obj) = entry.as_object_mut() {
            obj.insert(
                "State".to_owned(),
                serde_json::Value::String("CANCELLED".to_owned()),
            );
            obj.insert("LastModifiedTime".to_owned(), Self::now_timestamp());
        }

        Ok(GenericOutput {
            value: serde_json::json!({
                "ReplayArn": entry.get("Arn"),
                "ReplayName": name,
                "State": "CANCELLING",
                "StateReason": "Replay is being cancelled",
            }),
        })
    }

    /// Handle `DescribeReplay`.
    pub fn handle_describe_replay(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "ReplayName")?;
        let stored = Self::generic_describe(&self.replays, &name, "Replay")?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ReplayArn": stored.get("Arn"),
                "ReplayName": name,
                "Description": stored.get("Description"),
                "State": stored.get("State"),
                "StateReason": stored.get("StateReason"),
                "EventSourceArn": stored.get("EventSourceArn"),
                "Destination": stored.get("Destination"),
                "EventStartTime": stored.get("EventStartTime"),
                "EventEndTime": stored.get("EventEndTime"),
                "EventLastReplayedTime": stored.get("EventLastReplayedTime"),
                "ReplayStartTime": stored.get("CreationTime"),
                "ReplayEndTime": stored.get("ReplayEndTime"),
            }),
        })
    }

    /// Handle `ListReplays`.
    pub fn handle_list_replays(&self, input: &GenericInput) -> Result<GenericOutput, EventsError> {
        let prefix = input
            .value
            .get("NamePrefix")
            .and_then(serde_json::Value::as_str);
        let result = Self::generic_list(&self.replays, prefix, "Replays");
        Ok(GenericOutput { value: result })
    }

    // -----------------------------------------------------------------------
    // Phase 3: API Destinations
    // -----------------------------------------------------------------------

    /// Handle `CreateApiDestination`.
    pub fn handle_create_api_destination(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let arn = self.build_resource_arn("api-destination", &name);
        let result = Self::generic_create(
            &self.api_destinations,
            &name,
            input.value.clone(),
            &arn,
            "ACTIVE",
            "ApiDestination",
        )?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ApiDestinationArn": result.get("Arn"),
                "ApiDestinationState": "ACTIVE",
                "CreationTime": result.get("CreationTime"),
                "LastModifiedTime": result.get("LastModifiedTime"),
            }),
        })
    }

    /// Handle `DeleteApiDestination`.
    pub fn handle_delete_api_destination(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let result = Self::generic_delete(&self.api_destinations, &name, "ApiDestination")?;
        Ok(GenericOutput { value: result })
    }

    /// Handle `DescribeApiDestination`.
    pub fn handle_describe_api_destination(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let stored = Self::generic_describe(&self.api_destinations, &name, "ApiDestination")?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ApiDestinationArn": stored.get("Arn"),
                "Name": name,
                "Description": stored.get("Description"),
                "ApiDestinationState": stored.get("State"),
                "ConnectionArn": stored.get("ConnectionArn"),
                "InvocationEndpoint": stored.get("InvocationEndpoint"),
                "HttpMethod": stored.get("HttpMethod"),
                "InvocationRateLimitPerSecond": stored.get("InvocationRateLimitPerSecond"),
                "CreationTime": stored.get("CreationTime"),
                "LastModifiedTime": stored.get("LastModifiedTime"),
            }),
        })
    }

    /// Handle `ListApiDestinations`.
    pub fn handle_list_api_destinations(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let prefix = input
            .value
            .get("NamePrefix")
            .and_then(serde_json::Value::as_str);
        let result = Self::generic_list(&self.api_destinations, prefix, "ApiDestinations");
        Ok(GenericOutput { value: result })
    }

    /// Handle `UpdateApiDestination`.
    pub fn handle_update_api_destination(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let updated = Self::generic_update(
            &self.api_destinations,
            &name,
            &input.value,
            "ApiDestination",
        )?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ApiDestinationArn": updated.get("Arn"),
                "ApiDestinationState": updated.get("State"),
                "CreationTime": updated.get("CreationTime"),
                "LastModifiedTime": updated.get("LastModifiedTime"),
            }),
        })
    }

    // -----------------------------------------------------------------------
    // Phase 3: Connections
    // -----------------------------------------------------------------------

    /// Handle `CreateConnection`.
    pub fn handle_create_connection(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let arn = self.build_resource_arn("connection", &name);
        let result = Self::generic_create(
            &self.connections,
            &name,
            input.value.clone(),
            &arn,
            "AUTHORIZED",
            "Connection",
        )?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ConnectionArn": result.get("Arn"),
                "ConnectionState": "AUTHORIZED",
                "CreationTime": result.get("CreationTime"),
                "LastModifiedTime": result.get("LastModifiedTime"),
            }),
        })
    }

    /// Handle `DeleteConnection`.
    pub fn handle_delete_connection(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let stored = self.connections.remove(&name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Connection {name} does not exist."))
        })?;
        let (_, val) = stored;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ConnectionArn": val.get("Arn"),
                "ConnectionState": "DELETING",
                "CreationTime": val.get("CreationTime"),
                "LastModifiedTime": val.get("LastModifiedTime"),
                "LastAuthorizedTime": val.get("LastAuthorizedTime"),
            }),
        })
    }

    /// Handle `DescribeConnection`.
    pub fn handle_describe_connection(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let stored = Self::generic_describe(&self.connections, &name, "Connection")?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ConnectionArn": stored.get("Arn"),
                "Name": name,
                "Description": stored.get("Description"),
                "ConnectionState": stored.get("State"),
                "AuthorizationType": stored.get("AuthorizationType"),
                "AuthParameters": stored.get("AuthParameters"),
                "SecretArn": stored.get("SecretArn"),
                "CreationTime": stored.get("CreationTime"),
                "LastModifiedTime": stored.get("LastModifiedTime"),
                "LastAuthorizedTime": stored.get("LastAuthorizedTime"),
            }),
        })
    }

    /// Handle `ListConnections`.
    pub fn handle_list_connections(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let prefix = input
            .value
            .get("NamePrefix")
            .and_then(serde_json::Value::as_str);
        let result = Self::generic_list(&self.connections, prefix, "Connections");
        Ok(GenericOutput { value: result })
    }

    /// Handle `UpdateConnection`.
    pub fn handle_update_connection(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let updated = Self::generic_update(&self.connections, &name, &input.value, "Connection")?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "ConnectionArn": updated.get("Arn"),
                "ConnectionState": updated.get("State"),
                "CreationTime": updated.get("CreationTime"),
                "LastModifiedTime": updated.get("LastModifiedTime"),
                "LastAuthorizedTime": updated.get("LastAuthorizedTime"),
            }),
        })
    }

    /// Handle `DeauthorizeConnection`.
    pub fn handle_deauthorize_connection(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;

        let mut entry = self.connections.get_mut(&name).ok_or_else(|| {
            EventsError::resource_not_found(format!("Connection {name} does not exist."))
        })?;

        let now = Self::now_timestamp();
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(
                "State".to_owned(),
                serde_json::Value::String("DEAUTHORIZING".to_owned()),
            );
            obj.insert("LastModifiedTime".to_owned(), now);
        }

        Ok(GenericOutput {
            value: serde_json::json!({
                "ConnectionArn": entry.get("Arn"),
                "ConnectionState": "DEAUTHORIZING",
                "CreationTime": entry.get("CreationTime"),
                "LastModifiedTime": entry.get("LastModifiedTime"),
                "LastAuthorizedTime": entry.get("LastAuthorizedTime"),
            }),
        })
    }

    // -----------------------------------------------------------------------
    // Phase 3: Endpoints
    // -----------------------------------------------------------------------

    /// Handle `CreateEndpoint`.
    pub fn handle_create_endpoint(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let arn = self.build_resource_arn("endpoint", &name);
        let result = Self::generic_create(
            &self.endpoints,
            &name,
            input.value.clone(),
            &arn,
            "ACTIVE",
            "Endpoint",
        )?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "Arn": result.get("Arn"),
                "Name": name,
                "State": "CREATING",
                "EventBuses": result.get("EventBuses"),
                "RoutingConfig": result.get("RoutingConfig"),
                "ReplicationConfig": result.get("ReplicationConfig"),
                "RoleArn": result.get("RoleArn"),
            }),
        })
    }

    /// Handle `DeleteEndpoint`.
    pub fn handle_delete_endpoint(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let result = Self::generic_delete(&self.endpoints, &name, "Endpoint")?;
        Ok(GenericOutput { value: result })
    }

    /// Handle `DescribeEndpoint`.
    pub fn handle_describe_endpoint(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let stored = Self::generic_describe(&self.endpoints, &name, "Endpoint")?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "Arn": stored.get("Arn"),
                "Name": name,
                "Description": stored.get("Description"),
                "State": stored.get("State"),
                "StateReason": stored.get("StateReason"),
                "EventBuses": stored.get("EventBuses"),
                "RoutingConfig": stored.get("RoutingConfig"),
                "ReplicationConfig": stored.get("ReplicationConfig"),
                "RoleArn": stored.get("RoleArn"),
                "EndpointId": stored.get("EndpointId"),
                "EndpointUrl": stored.get("EndpointUrl"),
                "CreationTime": stored.get("CreationTime"),
                "LastModifiedTime": stored.get("LastModifiedTime"),
            }),
        })
    }

    /// Handle `ListEndpoints`.
    pub fn handle_list_endpoints(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let prefix = input
            .value
            .get("NamePrefix")
            .and_then(serde_json::Value::as_str);
        let result = Self::generic_list(&self.endpoints, prefix, "Endpoints");
        Ok(GenericOutput { value: result })
    }

    /// Handle `UpdateEndpoint`.
    pub fn handle_update_endpoint(
        &self,
        input: &GenericInput,
    ) -> Result<GenericOutput, EventsError> {
        let name = Self::require_name(&input.value, "Name")?;
        let updated = Self::generic_update(&self.endpoints, &name, &input.value, "Endpoint")?;

        Ok(GenericOutput {
            value: serde_json::json!({
                "Arn": updated.get("Arn"),
                "Name": name,
                "State": "UPDATING",
                "EventBuses": updated.get("EventBuses"),
                "RoutingConfig": updated.get("RoutingConfig"),
                "ReplicationConfig": updated.get("ReplicationConfig"),
                "RoleArn": updated.get("RoleArn"),
                "EndpointId": updated.get("EndpointId"),
                "EndpointUrl": updated.get("EndpointUrl"),
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Validate an event bus name: 1-256 chars, `[/.\-_A-Za-z0-9]`.
fn validate_event_bus_name(name: &str) -> Result<(), EventsError> {
    if name.is_empty() || name.len() > 256 {
        return Err(EventsError::validation(
            "Event bus name must be between 1 and 256 characters.",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_')
    {
        return Err(EventsError::validation(
            "Event bus name can only contain alphanumeric, '.', '-', '_', and '/' characters.",
        ));
    }
    Ok(())
}

/// Validate a rule name: 1-64 chars, `[.\-_A-Za-z0-9]`.
fn validate_rule_name(name: &str) -> Result<(), EventsError> {
    if name.is_empty() || name.len() > 64 {
        return Err(EventsError::validation(
            "Rule name must be between 1 and 64 characters.",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err(EventsError::validation(
            "Rule name can only contain alphanumeric, '.', '-', and '_' characters.",
        ));
    }
    Ok(())
}

/// Extract bus name from an ARN like `arn:aws:events:REGION:ACCOUNT:event-bus/NAME`.
fn extract_bus_name_from_arn(arn: &str) -> Option<String> {
    let parts: Vec<&str> = arn.splitn(6, ':').collect();
    if parts.len() == 6 {
        let resource = parts[5];
        if let Some(name) = resource.strip_prefix("event-bus/") {
            return Some(name.to_owned());
        }
    }
    None
}

/// Extract bus name and rule name from an ARN like
/// `arn:aws:events:REGION:ACCOUNT:rule/BUS_NAME/RULE_NAME`.
fn extract_rule_names_from_arn(arn: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = arn.splitn(6, ':').collect();
    if parts.len() == 6 {
        let resource = parts[5];
        if let Some(rest) = resource.strip_prefix("rule/") {
            let segments: Vec<&str> = rest.splitn(2, '/').collect();
            if segments.len() == 2 {
                return Some((segments[0].to_owned(), segments[1].to_owned()));
            }
        }
    }
    None
}

/// Simple JSON path navigation supporting `$.field.subfield` style paths.
fn apply_json_path(value: &serde_json::Value, path: &str) -> serde_json::Value {
    let path = path
        .strip_prefix("$.")
        .unwrap_or(path.strip_prefix('$').unwrap_or(path));

    if path.is_empty() {
        return value.clone();
    }

    let mut current = value;
    for segment in path.split('.') {
        match current.get(segment) {
            Some(v) => current = v,
            None => return serde_json::Value::Null,
        }
    }
    current.clone()
}

/// Convert internal `TargetState` to the model `Target` type.
fn target_state_to_model(t: &TargetState) -> Target {
    Target {
        id: t.id.clone(),
        arn: t.arn.clone(),
        role_arn: t.role_arn.clone(),
        input: t.input.clone(),
        input_path: t.input_path.clone(),
        input_transformer: t.input_transformer.as_ref().map(|it| InputTransformer {
            input_paths_map: it.input_paths_map.clone(),
            input_template: it.input_template.clone(),
        }),
        run_command_parameters: None,
        ecs_parameters: None,
        batch_parameters: None,
        sqs_parameters: None,
        http_parameters: None,
        redshift_data_parameters: None,
        sage_maker_pipeline_parameters: None,
        dead_letter_config: None,
        retry_policy: None,
        app_sync_parameters: None,
    }
}
