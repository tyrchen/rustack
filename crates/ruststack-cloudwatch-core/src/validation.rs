//! Input validation for CloudWatch operations.

use ruststack_cloudwatch_model::error::{CloudWatchError, CloudWatchErrorCode};
use ruststack_cloudwatch_model::types::Dimension;

/// Validate a namespace string.
pub fn validate_namespace(namespace: &str) -> Result<(), CloudWatchError> {
    if namespace.is_empty() || namespace.len() > 255 {
        return Err(CloudWatchError::with_message(
            CloudWatchErrorCode::InvalidParameterValueException,
            "Namespace must be between 1 and 255 characters.",
        ));
    }
    Ok(())
}

/// Validate a metric name.
pub fn validate_metric_name(name: &str) -> Result<(), CloudWatchError> {
    if name.is_empty() || name.len() > 255 {
        return Err(CloudWatchError::with_message(
            CloudWatchErrorCode::InvalidParameterValueException,
            "MetricName must be between 1 and 255 characters.",
        ));
    }
    Ok(())
}

/// Validate an alarm name.
pub fn validate_alarm_name(name: &str) -> Result<(), CloudWatchError> {
    if name.is_empty() || name.len() > 255 {
        return Err(CloudWatchError::with_message(
            CloudWatchErrorCode::InvalidParameterValueException,
            "AlarmName must be between 1 and 255 characters.",
        ));
    }
    Ok(())
}

/// Validate a set of dimensions.
pub fn validate_dimensions(dims: &[Dimension]) -> Result<(), CloudWatchError> {
    if dims.len() > 30 {
        return Err(CloudWatchError::with_message(
            CloudWatchErrorCode::InvalidParameterValueException,
            "Maximum number of dimensions per metric is 30.",
        ));
    }
    for d in dims {
        if d.name.is_empty() || d.name.len() > 255 {
            return Err(CloudWatchError::with_message(
                CloudWatchErrorCode::InvalidParameterValueException,
                "Dimension name must be between 1 and 255 characters.",
            ));
        }
        if d.value.is_empty() || d.value.len() > 1024 {
            return Err(CloudWatchError::with_message(
                CloudWatchErrorCode::InvalidParameterValueException,
                "Dimension value must be between 1 and 1024 characters.",
            ));
        }
    }
    Ok(())
}

/// Validate a dashboard name.
pub fn validate_dashboard_name(name: &str) -> Result<(), CloudWatchError> {
    if name.is_empty() || name.len() > 255 {
        return Err(CloudWatchError::with_message(
            CloudWatchErrorCode::InvalidParameterValueException,
            "DashboardName must be between 1 and 255 characters.",
        ));
    }
    Ok(())
}

/// Validate a dashboard body is valid JSON and within size limits.
pub fn validate_dashboard_body(body: &str) -> Result<(), CloudWatchError> {
    if body.len() > 1_048_576 {
        return Err(CloudWatchError::with_message(
            CloudWatchErrorCode::DashboardInvalidInputError,
            "Dashboard body exceeds maximum size of 1MB.",
        ));
    }
    serde_json::from_str::<serde_json::Value>(body).map_err(|e| {
        CloudWatchError::with_message(
            CloudWatchErrorCode::DashboardInvalidInputError,
            format!("Dashboard body is not valid JSON: {e}"),
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_validate_namespace() {
        assert!(validate_namespace("MyApp").is_ok());
        assert!(validate_namespace("").is_err());
        assert!(validate_namespace(&"x".repeat(256)).is_err());
    }

    #[test]
    fn test_should_validate_metric_name() {
        assert!(validate_metric_name("CPUUtilization").is_ok());
        assert!(validate_metric_name("").is_err());
    }

    #[test]
    fn test_should_validate_dimensions() {
        let dims: Vec<Dimension> = (0..31)
            .map(|i| Dimension {
                name: format!("dim{i}"),
                value: "v".to_owned(),
            })
            .collect();
        assert!(validate_dimensions(&dims).is_err());
    }

    #[test]
    fn test_should_validate_dashboard_body() {
        assert!(validate_dashboard_body(r#"{"widgets":[]}"#).is_ok());
        assert!(validate_dashboard_body("not json").is_err());
    }
}
