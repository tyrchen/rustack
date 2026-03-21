//! STS request router.
//!
//! STS uses the `awsQuery` protocol where requests are `POST /` with
//! `Content-Type: application/x-www-form-urlencoded`. The operation is
//! specified by the `Action=<OperationName>` form parameter.

use ruststack_sts_model::error::StsError;
use ruststack_sts_model::operations::StsOperation;

/// Resolve an STS operation from parsed form parameters.
///
/// Looks for the `Action` parameter in the form body and maps it to
/// the corresponding [`StsOperation`] variant.
pub fn resolve_operation(params: &[(String, String)]) -> Result<StsOperation, StsError> {
    let action = params
        .iter()
        .find(|(k, _)| k == "Action")
        .map(|(_, v)| v.as_str())
        .ok_or_else(StsError::missing_action)?;

    StsOperation::from_name(action).ok_or_else(|| StsError::unknown_operation(action))
}

#[cfg(test)]
mod tests {
    use ruststack_sts_model::error::StsErrorCode;

    use super::*;

    fn params_with_action(action: &str) -> Vec<(String, String)> {
        vec![("Action".to_owned(), action.to_owned())]
    }

    #[test]
    fn test_should_resolve_get_caller_identity() {
        let params = params_with_action("GetCallerIdentity");
        let op = resolve_operation(&params).unwrap();
        assert_eq!(op, StsOperation::GetCallerIdentity);
    }

    #[test]
    fn test_should_resolve_assume_role() {
        let params = params_with_action("AssumeRole");
        let op = resolve_operation(&params).unwrap();
        assert_eq!(op, StsOperation::AssumeRole);
    }

    #[test]
    fn test_should_resolve_get_session_token() {
        let params = params_with_action("GetSessionToken");
        let op = resolve_operation(&params).unwrap();
        assert_eq!(op, StsOperation::GetSessionToken);
    }

    #[test]
    fn test_should_resolve_get_access_key_info() {
        let params = params_with_action("GetAccessKeyInfo");
        let op = resolve_operation(&params).unwrap();
        assert_eq!(op, StsOperation::GetAccessKeyInfo);
    }

    #[test]
    fn test_should_error_on_missing_action() {
        let params: Vec<(String, String)> = vec![];
        let err = resolve_operation(&params).unwrap_err();
        assert_eq!(err.code, StsErrorCode::MissingAction);
    }

    #[test]
    fn test_should_error_on_unknown_operation() {
        let params = params_with_action("NonExistentOperation");
        let err = resolve_operation(&params).unwrap_err();
        assert_eq!(err.code, StsErrorCode::InvalidAction);
        assert!(err.message.contains("NonExistentOperation"));
    }

    #[test]
    fn test_should_find_action_among_other_params() {
        let params = vec![
            ("Version".to_owned(), "2011-06-15".to_owned()),
            ("Action".to_owned(), "AssumeRole".to_owned()),
            (
                "RoleArn".to_owned(),
                "arn:aws:iam::123456789012:role/MyRole".to_owned(),
            ),
        ];
        let op = resolve_operation(&params).unwrap();
        assert_eq!(op, StsOperation::AssumeRole);
    }
}
