//! Auto-generated from AWS CloudWatch Logs Smithy model. DO NOT EDIT.

/// All supported Logs operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogsOperation {
    /// The CreateLogGroup operation.
    CreateLogGroup,
    /// The DeleteLogGroup operation.
    DeleteLogGroup,
    /// The DescribeLogGroups operation.
    DescribeLogGroups,
    /// The CreateLogStream operation.
    CreateLogStream,
    /// The DeleteLogStream operation.
    DeleteLogStream,
    /// The DescribeLogStreams operation.
    DescribeLogStreams,
    /// The PutLogEvents operation.
    PutLogEvents,
    /// The GetLogEvents operation.
    GetLogEvents,
    /// The FilterLogEvents operation.
    FilterLogEvents,
    /// The PutRetentionPolicy operation.
    PutRetentionPolicy,
    /// The DeleteRetentionPolicy operation.
    DeleteRetentionPolicy,
    /// The PutMetricFilter operation.
    PutMetricFilter,
    /// The DeleteMetricFilter operation.
    DeleteMetricFilter,
    /// The DescribeMetricFilters operation.
    DescribeMetricFilters,
    /// The PutSubscriptionFilter operation.
    PutSubscriptionFilter,
    /// The DeleteSubscriptionFilter operation.
    DeleteSubscriptionFilter,
    /// The DescribeSubscriptionFilters operation.
    DescribeSubscriptionFilters,
    /// The PutResourcePolicy operation.
    PutResourcePolicy,
    /// The DeleteResourcePolicy operation.
    DeleteResourcePolicy,
    /// The DescribeResourcePolicies operation.
    DescribeResourcePolicies,
    /// The TagResource operation.
    TagResource,
    /// The UntagResource operation.
    UntagResource,
    /// The ListTagsForResource operation.
    ListTagsForResource,
    /// The TagLogGroup operation.
    TagLogGroup,
    /// The UntagLogGroup operation.
    UntagLogGroup,
    /// The ListTagsLogGroup operation.
    ListTagsLogGroup,
    /// The PutDestination operation.
    PutDestination,
    /// The PutDestinationPolicy operation.
    PutDestinationPolicy,
    /// The DeleteDestination operation.
    DeleteDestination,
    /// The DescribeDestinations operation.
    DescribeDestinations,
    /// The AssociateKmsKey operation.
    AssociateKmsKey,
    /// The DisassociateKmsKey operation.
    DisassociateKmsKey,
    /// The StartQuery operation.
    StartQuery,
    /// The StopQuery operation.
    StopQuery,
    /// The GetQueryResults operation.
    GetQueryResults,
    /// The DescribeQueries operation.
    DescribeQueries,
    /// The PutQueryDefinition operation.
    PutQueryDefinition,
    /// The DeleteQueryDefinition operation.
    DeleteQueryDefinition,
    /// The DescribeQueryDefinitions operation.
    DescribeQueryDefinitions,
    /// The CreateExportTask operation.
    CreateExportTask,
    /// The CancelExportTask operation.
    CancelExportTask,
    /// The DescribeExportTasks operation.
    DescribeExportTasks,
    /// The TestMetricFilter operation.
    TestMetricFilter,
}

impl LogsOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateLogGroup => "CreateLogGroup",
            Self::DeleteLogGroup => "DeleteLogGroup",
            Self::DescribeLogGroups => "DescribeLogGroups",
            Self::CreateLogStream => "CreateLogStream",
            Self::DeleteLogStream => "DeleteLogStream",
            Self::DescribeLogStreams => "DescribeLogStreams",
            Self::PutLogEvents => "PutLogEvents",
            Self::GetLogEvents => "GetLogEvents",
            Self::FilterLogEvents => "FilterLogEvents",
            Self::PutRetentionPolicy => "PutRetentionPolicy",
            Self::DeleteRetentionPolicy => "DeleteRetentionPolicy",
            Self::PutMetricFilter => "PutMetricFilter",
            Self::DeleteMetricFilter => "DeleteMetricFilter",
            Self::DescribeMetricFilters => "DescribeMetricFilters",
            Self::PutSubscriptionFilter => "PutSubscriptionFilter",
            Self::DeleteSubscriptionFilter => "DeleteSubscriptionFilter",
            Self::DescribeSubscriptionFilters => "DescribeSubscriptionFilters",
            Self::PutResourcePolicy => "PutResourcePolicy",
            Self::DeleteResourcePolicy => "DeleteResourcePolicy",
            Self::DescribeResourcePolicies => "DescribeResourcePolicies",
            Self::TagResource => "TagResource",
            Self::UntagResource => "UntagResource",
            Self::ListTagsForResource => "ListTagsForResource",
            Self::TagLogGroup => "TagLogGroup",
            Self::UntagLogGroup => "UntagLogGroup",
            Self::ListTagsLogGroup => "ListTagsLogGroup",
            Self::PutDestination => "PutDestination",
            Self::PutDestinationPolicy => "PutDestinationPolicy",
            Self::DeleteDestination => "DeleteDestination",
            Self::DescribeDestinations => "DescribeDestinations",
            Self::AssociateKmsKey => "AssociateKmsKey",
            Self::DisassociateKmsKey => "DisassociateKmsKey",
            Self::StartQuery => "StartQuery",
            Self::StopQuery => "StopQuery",
            Self::GetQueryResults => "GetQueryResults",
            Self::DescribeQueries => "DescribeQueries",
            Self::PutQueryDefinition => "PutQueryDefinition",
            Self::DeleteQueryDefinition => "DeleteQueryDefinition",
            Self::DescribeQueryDefinitions => "DescribeQueryDefinitions",
            Self::CreateExportTask => "CreateExportTask",
            Self::CancelExportTask => "CancelExportTask",
            Self::DescribeExportTasks => "DescribeExportTasks",
            Self::TestMetricFilter => "TestMetricFilter",
        }
    }

    /// Parse an operation name string into an LogsOperation.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateLogGroup" => Some(Self::CreateLogGroup),
            "DeleteLogGroup" => Some(Self::DeleteLogGroup),
            "DescribeLogGroups" => Some(Self::DescribeLogGroups),
            "CreateLogStream" => Some(Self::CreateLogStream),
            "DeleteLogStream" => Some(Self::DeleteLogStream),
            "DescribeLogStreams" => Some(Self::DescribeLogStreams),
            "PutLogEvents" => Some(Self::PutLogEvents),
            "GetLogEvents" => Some(Self::GetLogEvents),
            "FilterLogEvents" => Some(Self::FilterLogEvents),
            "PutRetentionPolicy" => Some(Self::PutRetentionPolicy),
            "DeleteRetentionPolicy" => Some(Self::DeleteRetentionPolicy),
            "PutMetricFilter" => Some(Self::PutMetricFilter),
            "DeleteMetricFilter" => Some(Self::DeleteMetricFilter),
            "DescribeMetricFilters" => Some(Self::DescribeMetricFilters),
            "PutSubscriptionFilter" => Some(Self::PutSubscriptionFilter),
            "DeleteSubscriptionFilter" => Some(Self::DeleteSubscriptionFilter),
            "DescribeSubscriptionFilters" => Some(Self::DescribeSubscriptionFilters),
            "PutResourcePolicy" => Some(Self::PutResourcePolicy),
            "DeleteResourcePolicy" => Some(Self::DeleteResourcePolicy),
            "DescribeResourcePolicies" => Some(Self::DescribeResourcePolicies),
            "TagResource" => Some(Self::TagResource),
            "UntagResource" => Some(Self::UntagResource),
            "ListTagsForResource" => Some(Self::ListTagsForResource),
            "TagLogGroup" => Some(Self::TagLogGroup),
            "UntagLogGroup" => Some(Self::UntagLogGroup),
            "ListTagsLogGroup" => Some(Self::ListTagsLogGroup),
            "PutDestination" => Some(Self::PutDestination),
            "PutDestinationPolicy" => Some(Self::PutDestinationPolicy),
            "DeleteDestination" => Some(Self::DeleteDestination),
            "DescribeDestinations" => Some(Self::DescribeDestinations),
            "AssociateKmsKey" => Some(Self::AssociateKmsKey),
            "DisassociateKmsKey" => Some(Self::DisassociateKmsKey),
            "StartQuery" => Some(Self::StartQuery),
            "StopQuery" => Some(Self::StopQuery),
            "GetQueryResults" => Some(Self::GetQueryResults),
            "DescribeQueries" => Some(Self::DescribeQueries),
            "PutQueryDefinition" => Some(Self::PutQueryDefinition),
            "DeleteQueryDefinition" => Some(Self::DeleteQueryDefinition),
            "DescribeQueryDefinitions" => Some(Self::DescribeQueryDefinitions),
            "CreateExportTask" => Some(Self::CreateExportTask),
            "CancelExportTask" => Some(Self::CancelExportTask),
            "DescribeExportTasks" => Some(Self::DescribeExportTasks),
            "TestMetricFilter" => Some(Self::TestMetricFilter),
            _ => None,
        }
    }
}

impl std::fmt::Display for LogsOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
