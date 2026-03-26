//! EventBridge operation enum.

use std::fmt;

/// All supported EventBridge operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventsOperation {
    // Phase 0: Event bus management
    /// Create an event bus.
    CreateEventBus,
    /// Delete an event bus.
    DeleteEventBus,
    /// Describe an event bus.
    DescribeEventBus,
    /// List event buses.
    ListEventBuses,

    // Phase 0: Rule management
    /// Create or update a rule.
    PutRule,
    /// Delete a rule.
    DeleteRule,
    /// Describe a rule.
    DescribeRule,
    /// List rules on an event bus.
    ListRules,
    /// Enable a rule.
    EnableRule,
    /// Disable a rule.
    DisableRule,

    // Phase 0: Target management
    /// Add targets to a rule.
    PutTargets,
    /// Remove targets from a rule.
    RemoveTargets,
    /// List targets for a rule.
    ListTargetsByRule,

    // Phase 0: Event operations
    /// Put events onto an event bus.
    PutEvents,
    /// Test an event pattern against an event.
    TestEventPattern,

    // Phase 1: Tags, permissions, reverse lookup
    /// Tag a resource.
    TagResource,
    /// Untag a resource.
    UntagResource,
    /// List tags for a resource.
    ListTagsForResource,
    /// Add a permission to an event bus.
    PutPermission,
    /// Remove a permission from an event bus.
    RemovePermission,
    /// List rule names that target a given ARN.
    ListRuleNamesByTarget,

    // Phase 2: Update and input transform
    /// Update an event bus.
    UpdateEventBus,

    // Phase 3: Archive/Replay stubs
    /// Create an archive.
    CreateArchive,
    /// Delete an archive.
    DeleteArchive,
    /// Describe an archive.
    DescribeArchive,
    /// List archives.
    ListArchives,
    /// Update an archive.
    UpdateArchive,
    /// Start a replay.
    StartReplay,
    /// Cancel a replay.
    CancelReplay,
    /// Describe a replay.
    DescribeReplay,
    /// List replays.
    ListReplays,

    // Phase 3: API Destinations stubs
    /// Create an API destination.
    CreateApiDestination,
    /// Delete an API destination.
    DeleteApiDestination,
    /// Describe an API destination.
    DescribeApiDestination,
    /// List API destinations.
    ListApiDestinations,
    /// Update an API destination.
    UpdateApiDestination,

    // Phase 3: Connections stubs
    /// Create a connection.
    CreateConnection,
    /// Delete a connection.
    DeleteConnection,
    /// Describe a connection.
    DescribeConnection,
    /// List connections.
    ListConnections,
    /// Update a connection.
    UpdateConnection,
    /// Deauthorize a connection.
    DeauthorizeConnection,

    // Phase 3: Endpoints stubs
    /// Create an endpoint.
    CreateEndpoint,
    /// Delete an endpoint.
    DeleteEndpoint,
    /// Describe an endpoint.
    DescribeEndpoint,
    /// List endpoints.
    ListEndpoints,
    /// Update an endpoint.
    UpdateEndpoint,

    // Phase 3: Partner event sources stubs
    /// Activate an event source.
    ActivateEventSource,
    /// Create a partner event source.
    CreatePartnerEventSource,
    /// Deactivate an event source.
    DeactivateEventSource,
    /// Delete a partner event source.
    DeletePartnerEventSource,
    /// Describe an event source.
    DescribeEventSource,
    /// Describe a partner event source.
    DescribePartnerEventSource,
    /// List event sources.
    ListEventSources,
    /// List partner event source accounts.
    ListPartnerEventSourceAccounts,
    /// List partner event sources.
    ListPartnerEventSources,
    /// Put partner events.
    PutPartnerEvents,
}

impl EventsOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateEventBus => "CreateEventBus",
            Self::DeleteEventBus => "DeleteEventBus",
            Self::DescribeEventBus => "DescribeEventBus",
            Self::ListEventBuses => "ListEventBuses",
            Self::PutRule => "PutRule",
            Self::DeleteRule => "DeleteRule",
            Self::DescribeRule => "DescribeRule",
            Self::ListRules => "ListRules",
            Self::EnableRule => "EnableRule",
            Self::DisableRule => "DisableRule",
            Self::PutTargets => "PutTargets",
            Self::RemoveTargets => "RemoveTargets",
            Self::ListTargetsByRule => "ListTargetsByRule",
            Self::PutEvents => "PutEvents",
            Self::TestEventPattern => "TestEventPattern",
            Self::TagResource => "TagResource",
            Self::UntagResource => "UntagResource",
            Self::ListTagsForResource => "ListTagsForResource",
            Self::PutPermission => "PutPermission",
            Self::RemovePermission => "RemovePermission",
            Self::ListRuleNamesByTarget => "ListRuleNamesByTarget",
            Self::UpdateEventBus => "UpdateEventBus",
            Self::CreateArchive => "CreateArchive",
            Self::DeleteArchive => "DeleteArchive",
            Self::DescribeArchive => "DescribeArchive",
            Self::ListArchives => "ListArchives",
            Self::UpdateArchive => "UpdateArchive",
            Self::StartReplay => "StartReplay",
            Self::CancelReplay => "CancelReplay",
            Self::DescribeReplay => "DescribeReplay",
            Self::ListReplays => "ListReplays",
            Self::CreateApiDestination => "CreateApiDestination",
            Self::DeleteApiDestination => "DeleteApiDestination",
            Self::DescribeApiDestination => "DescribeApiDestination",
            Self::ListApiDestinations => "ListApiDestinations",
            Self::UpdateApiDestination => "UpdateApiDestination",
            Self::CreateConnection => "CreateConnection",
            Self::DeleteConnection => "DeleteConnection",
            Self::DescribeConnection => "DescribeConnection",
            Self::ListConnections => "ListConnections",
            Self::UpdateConnection => "UpdateConnection",
            Self::DeauthorizeConnection => "DeauthorizeConnection",
            Self::CreateEndpoint => "CreateEndpoint",
            Self::DeleteEndpoint => "DeleteEndpoint",
            Self::DescribeEndpoint => "DescribeEndpoint",
            Self::ListEndpoints => "ListEndpoints",
            Self::UpdateEndpoint => "UpdateEndpoint",
            Self::ActivateEventSource => "ActivateEventSource",
            Self::CreatePartnerEventSource => "CreatePartnerEventSource",
            Self::DeactivateEventSource => "DeactivateEventSource",
            Self::DeletePartnerEventSource => "DeletePartnerEventSource",
            Self::DescribeEventSource => "DescribeEventSource",
            Self::DescribePartnerEventSource => "DescribePartnerEventSource",
            Self::ListEventSources => "ListEventSources",
            Self::ListPartnerEventSourceAccounts => "ListPartnerEventSourceAccounts",
            Self::ListPartnerEventSources => "ListPartnerEventSources",
            Self::PutPartnerEvents => "PutPartnerEvents",
        }
    }

    /// Parse an operation name string into an `EventsOperation`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateEventBus" => Some(Self::CreateEventBus),
            "DeleteEventBus" => Some(Self::DeleteEventBus),
            "DescribeEventBus" => Some(Self::DescribeEventBus),
            "ListEventBuses" => Some(Self::ListEventBuses),
            "PutRule" => Some(Self::PutRule),
            "DeleteRule" => Some(Self::DeleteRule),
            "DescribeRule" => Some(Self::DescribeRule),
            "ListRules" => Some(Self::ListRules),
            "EnableRule" => Some(Self::EnableRule),
            "DisableRule" => Some(Self::DisableRule),
            "PutTargets" => Some(Self::PutTargets),
            "RemoveTargets" => Some(Self::RemoveTargets),
            "ListTargetsByRule" => Some(Self::ListTargetsByRule),
            "PutEvents" => Some(Self::PutEvents),
            "TestEventPattern" => Some(Self::TestEventPattern),
            "TagResource" => Some(Self::TagResource),
            "UntagResource" => Some(Self::UntagResource),
            "ListTagsForResource" => Some(Self::ListTagsForResource),
            "PutPermission" => Some(Self::PutPermission),
            "RemovePermission" => Some(Self::RemovePermission),
            "ListRuleNamesByTarget" => Some(Self::ListRuleNamesByTarget),
            "UpdateEventBus" => Some(Self::UpdateEventBus),
            "CreateArchive" => Some(Self::CreateArchive),
            "DeleteArchive" => Some(Self::DeleteArchive),
            "DescribeArchive" => Some(Self::DescribeArchive),
            "ListArchives" => Some(Self::ListArchives),
            "UpdateArchive" => Some(Self::UpdateArchive),
            "StartReplay" => Some(Self::StartReplay),
            "CancelReplay" => Some(Self::CancelReplay),
            "DescribeReplay" => Some(Self::DescribeReplay),
            "ListReplays" => Some(Self::ListReplays),
            "CreateApiDestination" => Some(Self::CreateApiDestination),
            "DeleteApiDestination" => Some(Self::DeleteApiDestination),
            "DescribeApiDestination" => Some(Self::DescribeApiDestination),
            "ListApiDestinations" => Some(Self::ListApiDestinations),
            "UpdateApiDestination" => Some(Self::UpdateApiDestination),
            "CreateConnection" => Some(Self::CreateConnection),
            "DeleteConnection" => Some(Self::DeleteConnection),
            "DescribeConnection" => Some(Self::DescribeConnection),
            "ListConnections" => Some(Self::ListConnections),
            "UpdateConnection" => Some(Self::UpdateConnection),
            "DeauthorizeConnection" => Some(Self::DeauthorizeConnection),
            "CreateEndpoint" => Some(Self::CreateEndpoint),
            "DeleteEndpoint" => Some(Self::DeleteEndpoint),
            "DescribeEndpoint" => Some(Self::DescribeEndpoint),
            "ListEndpoints" => Some(Self::ListEndpoints),
            "UpdateEndpoint" => Some(Self::UpdateEndpoint),
            "ActivateEventSource" => Some(Self::ActivateEventSource),
            "CreatePartnerEventSource" => Some(Self::CreatePartnerEventSource),
            "DeactivateEventSource" => Some(Self::DeactivateEventSource),
            "DeletePartnerEventSource" => Some(Self::DeletePartnerEventSource),
            "DescribeEventSource" => Some(Self::DescribeEventSource),
            "DescribePartnerEventSource" => Some(Self::DescribePartnerEventSource),
            "ListEventSources" => Some(Self::ListEventSources),
            "ListPartnerEventSourceAccounts" => Some(Self::ListPartnerEventSourceAccounts),
            "ListPartnerEventSources" => Some(Self::ListPartnerEventSources),
            "PutPartnerEvents" => Some(Self::PutPartnerEvents),
            _ => None,
        }
    }

    /// Returns `true` if this operation is implemented.
    #[must_use]
    pub fn is_implemented(&self) -> bool {
        matches!(
            self,
            // Phase 0
            Self::CreateEventBus
                | Self::DeleteEventBus
                | Self::DescribeEventBus
                | Self::ListEventBuses
                | Self::PutRule
                | Self::DeleteRule
                | Self::DescribeRule
                | Self::ListRules
                | Self::EnableRule
                | Self::DisableRule
                | Self::PutTargets
                | Self::RemoveTargets
                | Self::ListTargetsByRule
                | Self::PutEvents
                | Self::TestEventPattern
                // Phase 1
                | Self::TagResource
                | Self::UntagResource
                | Self::ListTagsForResource
                | Self::PutPermission
                | Self::RemovePermission
                | Self::ListRuleNamesByTarget
                // Phase 2
                | Self::UpdateEventBus
                // Phase 3: Archives
                | Self::CreateArchive
                | Self::DeleteArchive
                | Self::DescribeArchive
                | Self::ListArchives
                | Self::UpdateArchive
                // Phase 3: Replays
                | Self::StartReplay
                | Self::CancelReplay
                | Self::DescribeReplay
                | Self::ListReplays
                // Phase 3: API Destinations
                | Self::CreateApiDestination
                | Self::DeleteApiDestination
                | Self::DescribeApiDestination
                | Self::ListApiDestinations
                | Self::UpdateApiDestination
                // Phase 3: Connections
                | Self::CreateConnection
                | Self::DeleteConnection
                | Self::DescribeConnection
                | Self::ListConnections
                | Self::UpdateConnection
                | Self::DeauthorizeConnection
                // Phase 3: Endpoints
                | Self::CreateEndpoint
                | Self::DeleteEndpoint
                | Self::DescribeEndpoint
                | Self::ListEndpoints
                | Self::UpdateEndpoint
        )
    }
}

impl fmt::Display for EventsOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
