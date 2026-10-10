//! IAM `awsQuery` request parameter parsing utilities.
//!
//! IAM uses `application/x-www-form-urlencoded` request bodies with
//! dot-notation for nested parameters (e.g., `Tags.member.1.Key`).

use rustack_iam_model::error::IamError;

/// Parse a URL-encoded body into a list of key-value pairs.
#[must_use]
pub fn parse_form_params(body: &[u8]) -> Vec<(String, String)> {
    form_urlencoded::parse(body)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// Get a required parameter value.
///
/// Returns an error if the parameter is not present.
pub fn get_required_param<'a>(
    params: &'a [(String, String)],
    key: &str,
) -> Result<&'a str, IamError> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| IamError::invalid_input(format!("Missing required parameter: {key}")))
}

/// Get an optional parameter value.
#[must_use]
pub fn get_optional_param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Get an optional i32 parameter.
///
/// Parses the parameter value as an `i32`. Returns `None` if the parameter
/// is absent or cannot be parsed.
#[must_use]
pub fn get_optional_i32(params: &[(String, String)], key: &str) -> Option<i32> {
    get_optional_param(params, key).and_then(|v| v.parse::<i32>().ok())
}

/// Get an optional boolean parameter.
///
/// Parses `"true"` / `"false"` (case-insensitive). Returns `None` if
/// the parameter is absent.
#[must_use]
pub fn get_optional_bool(params: &[(String, String)], key: &str) -> Option<bool> {
    get_optional_param(params, key).map(|v| v.eq_ignore_ascii_case("true"))
}

/// Collect unique numeric indices from params matching a prefix pattern.
///
/// Given parameters like `prefix.1.suffix`, `prefix.2.suffix`, extracts
/// the unique numeric indices (1, 2, ...).
fn collect_indices(params: &[(String, String)], prefix: &str) -> Vec<u32> {
    let mut indices: Vec<u32> = Vec::new();
    for (k, _) in params {
        if let Some(rest) = k.strip_prefix(prefix) {
            if let Some(idx_str) = rest.split('.').next() {
                if let Ok(idx) = idx_str.parse::<u32>() {
                    if !indices.contains(&idx) {
                        indices.push(idx);
                    }
                }
            }
        }
    }
    indices
}

/// Parse `Tags.member.N.Key` / `Tags.member.N.Value` into a list of
/// key-value pairs.
///
/// AWS `awsQuery` tag list format:
///
/// ```text
/// Tags.member.1.Key=Environment
/// Tags.member.1.Value=Production
/// Tags.member.2.Key=Project
/// Tags.member.2.Value=MyApp
/// ```
#[must_use]
pub fn parse_tag_list(params: &[(String, String)]) -> Vec<(String, String)> {
    let member_prefix = "Tags.member.";
    let indices = collect_indices(params, member_prefix);

    let mut tags = Vec::with_capacity(indices.len());
    for idx in indices {
        let key_param = format!("{member_prefix}{idx}.Key");
        let value_param = format!("{member_prefix}{idx}.Value");

        if let Some(key) = get_optional_param(params, &key_param) {
            let value = get_optional_param(params, &value_param).unwrap_or("");
            tags.push((key.to_owned(), value.to_owned()));
        }
    }

    tags
}

/// Parse a list of strings: `Prefix.member.N`.
///
/// ```text
/// ActionName.member.1=Publish
/// ActionName.member.2=Subscribe
/// ```
#[must_use]
pub fn parse_string_list(params: &[(String, String)], prefix: &str) -> Vec<String> {
    let member_prefix = format!("{prefix}.member.");

    let mut entries: Vec<(u32, String)> = Vec::new();
    for (k, v) in params {
        if let Some(rest) = k.strip_prefix(&member_prefix) {
            if let Ok(idx) = rest.parse::<u32>() {
                entries.push((idx, v.clone()));
            }
        }
    }

    entries.sort_by_key(|(idx, _)| *idx);
    entries.into_iter().map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_form_params() {
        let body = b"Action=CreateUser&UserName=testuser&Version=2010-05-08";
        let params = parse_form_params(body);
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], ("Action".to_owned(), "CreateUser".to_owned()));
        assert_eq!(params[1], ("UserName".to_owned(), "testuser".to_owned()));
    }

    #[test]
    fn test_should_get_required_param() {
        let params = vec![("UserName".to_owned(), "testuser".to_owned())];
        assert_eq!(get_required_param(&params, "UserName").unwrap(), "testuser");
    }

    #[test]
    fn test_should_error_on_missing_required_param() {
        let params: Vec<(String, String)> = vec![];
        let err = get_required_param(&params, "UserName").unwrap_err();
        assert!(err.message.contains("UserName"));
    }

    #[test]
    fn test_should_get_optional_param() {
        let params = vec![("UserName".to_owned(), "testuser".to_owned())];
        assert_eq!(get_optional_param(&params, "UserName"), Some("testuser"));
        assert_eq!(get_optional_param(&params, "Missing"), None);
    }

    #[test]
    fn test_should_get_optional_i32() {
        let params = vec![
            ("MaxItems".to_owned(), "100".to_owned()),
            ("Invalid".to_owned(), "notanumber".to_owned()),
        ];
        assert_eq!(get_optional_i32(&params, "MaxItems"), Some(100));
        assert_eq!(get_optional_i32(&params, "Invalid"), None);
        assert_eq!(get_optional_i32(&params, "Missing"), None);
    }

    #[test]
    fn test_should_get_optional_bool() {
        let params = vec![
            ("Flag1".to_owned(), "true".to_owned()),
            ("Flag2".to_owned(), "false".to_owned()),
            ("Flag3".to_owned(), "TRUE".to_owned()),
        ];
        assert_eq!(get_optional_bool(&params, "Flag1"), Some(true));
        assert_eq!(get_optional_bool(&params, "Flag2"), Some(false));
        assert_eq!(get_optional_bool(&params, "Flag3"), Some(true));
        assert_eq!(get_optional_bool(&params, "Missing"), None);
    }

    #[test]
    fn test_should_parse_tag_list() {
        let params = vec![
            ("Tags.member.1.Key".to_owned(), "Environment".to_owned()),
            ("Tags.member.1.Value".to_owned(), "Production".to_owned()),
            ("Tags.member.2.Key".to_owned(), "Project".to_owned()),
            ("Tags.member.2.Value".to_owned(), "MyApp".to_owned()),
        ];
        let tags = parse_tag_list(&params);
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].0, "Environment");
        assert_eq!(tags[0].1, "Production");
        assert_eq!(tags[1].0, "Project");
        assert_eq!(tags[1].1, "MyApp");
    }

    #[test]
    fn test_should_parse_tag_list_with_empty_value() {
        let params = vec![("Tags.member.1.Key".to_owned(), "EmptyTag".to_owned())];
        let tags = parse_tag_list(&params);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].0, "EmptyTag");
        assert_eq!(tags[0].1, "");
    }

    #[test]
    fn test_should_parse_empty_tag_list() {
        let params: Vec<(String, String)> = vec![];
        let tags = parse_tag_list(&params);
        assert_eq!(tags.len(), 0);
    }

    #[test]
    fn test_should_parse_string_list() {
        let params = vec![
            ("Actions.member.1".to_owned(), "CreateUser".to_owned()),
            ("Actions.member.2".to_owned(), "DeleteUser".to_owned()),
            ("Actions.member.3".to_owned(), "GetUser".to_owned()),
        ];
        let list = parse_string_list(&params, "Actions");
        assert_eq!(list, vec!["CreateUser", "DeleteUser", "GetUser"]);
    }

    #[test]
    fn test_should_parse_empty_string_list() {
        let params: Vec<(String, String)> = vec![];
        let list = parse_string_list(&params, "Actions");
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_should_parse_url_encoded_special_chars() {
        let body = b"Action=CreateUser&UserName=test+user&Path=%2Fdivision%2F";
        let params = parse_form_params(body);
        assert_eq!(params.len(), 3);
        assert_eq!(params[1].1, "test user");
        assert_eq!(params[2].1, "/division/");
    }
}
