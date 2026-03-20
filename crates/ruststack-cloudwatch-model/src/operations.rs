//! Auto-generated from AWS CloudWatch Smithy model. DO NOT EDIT.

/// All supported CloudWatch operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloudWatchOperation {
    /// The PutMetricData operation.
    PutMetricData,
    /// The GetMetricData operation.
    GetMetricData,
    /// The GetMetricStatistics operation.
    GetMetricStatistics,
    /// The ListMetrics operation.
    ListMetrics,
    /// The PutMetricAlarm operation.
    PutMetricAlarm,
    /// The DescribeAlarms operation.
    DescribeAlarms,
    /// The DescribeAlarmsForMetric operation.
    DescribeAlarmsForMetric,
    /// The DeleteAlarms operation.
    DeleteAlarms,
    /// The SetAlarmState operation.
    SetAlarmState,
    /// The EnableAlarmActions operation.
    EnableAlarmActions,
    /// The DisableAlarmActions operation.
    DisableAlarmActions,
    /// The DescribeAlarmHistory operation.
    DescribeAlarmHistory,
    /// The TagResource operation.
    TagResource,
    /// The UntagResource operation.
    UntagResource,
    /// The ListTagsForResource operation.
    ListTagsForResource,
    /// The PutCompositeAlarm operation.
    PutCompositeAlarm,
    /// The PutDashboard operation.
    PutDashboard,
    /// The GetDashboard operation.
    GetDashboard,
    /// The DeleteDashboards operation.
    DeleteDashboards,
    /// The ListDashboards operation.
    ListDashboards,
    /// The PutInsightRule operation.
    PutInsightRule,
    /// The DeleteInsightRules operation.
    DeleteInsightRules,
    /// The DescribeInsightRules operation.
    DescribeInsightRules,
    /// The PutAnomalyDetector operation.
    PutAnomalyDetector,
    /// The DescribeAnomalyDetectors operation.
    DescribeAnomalyDetectors,
    /// The DeleteAnomalyDetector operation.
    DeleteAnomalyDetector,
    /// The PutManagedInsightRules operation.
    PutManagedInsightRules,
    /// The PutMetricStream operation.
    PutMetricStream,
    /// The DeleteMetricStream operation.
    DeleteMetricStream,
    /// The ListMetricStreams operation.
    ListMetricStreams,
    /// The GetMetricStream operation.
    GetMetricStream,
}

impl CloudWatchOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PutMetricData => "PutMetricData",
            Self::GetMetricData => "GetMetricData",
            Self::GetMetricStatistics => "GetMetricStatistics",
            Self::ListMetrics => "ListMetrics",
            Self::PutMetricAlarm => "PutMetricAlarm",
            Self::DescribeAlarms => "DescribeAlarms",
            Self::DescribeAlarmsForMetric => "DescribeAlarmsForMetric",
            Self::DeleteAlarms => "DeleteAlarms",
            Self::SetAlarmState => "SetAlarmState",
            Self::EnableAlarmActions => "EnableAlarmActions",
            Self::DisableAlarmActions => "DisableAlarmActions",
            Self::DescribeAlarmHistory => "DescribeAlarmHistory",
            Self::TagResource => "TagResource",
            Self::UntagResource => "UntagResource",
            Self::ListTagsForResource => "ListTagsForResource",
            Self::PutCompositeAlarm => "PutCompositeAlarm",
            Self::PutDashboard => "PutDashboard",
            Self::GetDashboard => "GetDashboard",
            Self::DeleteDashboards => "DeleteDashboards",
            Self::ListDashboards => "ListDashboards",
            Self::PutInsightRule => "PutInsightRule",
            Self::DeleteInsightRules => "DeleteInsightRules",
            Self::DescribeInsightRules => "DescribeInsightRules",
            Self::PutAnomalyDetector => "PutAnomalyDetector",
            Self::DescribeAnomalyDetectors => "DescribeAnomalyDetectors",
            Self::DeleteAnomalyDetector => "DeleteAnomalyDetector",
            Self::PutManagedInsightRules => "PutManagedInsightRules",
            Self::PutMetricStream => "PutMetricStream",
            Self::DeleteMetricStream => "DeleteMetricStream",
            Self::ListMetricStreams => "ListMetricStreams",
            Self::GetMetricStream => "GetMetricStream",
        }
    }

    /// Parse an operation name string into an CloudWatchOperation.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "PutMetricData" => Some(Self::PutMetricData),
            "GetMetricData" => Some(Self::GetMetricData),
            "GetMetricStatistics" => Some(Self::GetMetricStatistics),
            "ListMetrics" => Some(Self::ListMetrics),
            "PutMetricAlarm" => Some(Self::PutMetricAlarm),
            "DescribeAlarms" => Some(Self::DescribeAlarms),
            "DescribeAlarmsForMetric" => Some(Self::DescribeAlarmsForMetric),
            "DeleteAlarms" => Some(Self::DeleteAlarms),
            "SetAlarmState" => Some(Self::SetAlarmState),
            "EnableAlarmActions" => Some(Self::EnableAlarmActions),
            "DisableAlarmActions" => Some(Self::DisableAlarmActions),
            "DescribeAlarmHistory" => Some(Self::DescribeAlarmHistory),
            "TagResource" => Some(Self::TagResource),
            "UntagResource" => Some(Self::UntagResource),
            "ListTagsForResource" => Some(Self::ListTagsForResource),
            "PutCompositeAlarm" => Some(Self::PutCompositeAlarm),
            "PutDashboard" => Some(Self::PutDashboard),
            "GetDashboard" => Some(Self::GetDashboard),
            "DeleteDashboards" => Some(Self::DeleteDashboards),
            "ListDashboards" => Some(Self::ListDashboards),
            "PutInsightRule" => Some(Self::PutInsightRule),
            "DeleteInsightRules" => Some(Self::DeleteInsightRules),
            "DescribeInsightRules" => Some(Self::DescribeInsightRules),
            "PutAnomalyDetector" => Some(Self::PutAnomalyDetector),
            "DescribeAnomalyDetectors" => Some(Self::DescribeAnomalyDetectors),
            "DeleteAnomalyDetector" => Some(Self::DeleteAnomalyDetector),
            "PutManagedInsightRules" => Some(Self::PutManagedInsightRules),
            "PutMetricStream" => Some(Self::PutMetricStream),
            "DeleteMetricStream" => Some(Self::DeleteMetricStream),
            "ListMetricStreams" => Some(Self::ListMetricStreams),
            "GetMetricStream" => Some(Self::GetMetricStream),
            _ => None,
        }
    }
}

impl std::fmt::Display for CloudWatchOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
