//! STS `awsQuery` request parameter parsing utilities.
//!
//! STS uses `application/x-www-form-urlencoded` request bodies with
//! dot-notation for nested parameters (e.g., `Tags.member.1.Key`).

use ruststack_sts_model::error::StsError;

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
) -> Result<&'a str, StsError> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| {
            StsError::invalid_parameter_value(format!("Missing required parameter: {key}"))
        })
}

/// Get an optional parameter value.
#[must_use]
pub fn get_optional_param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Parse session tags from awsQuery form parameters.
///
/// Tags are encoded as:
/// - `Tags.member.1.Key=Project`
/// - `Tags.member.1.Value=MyProject`
#[must_use]
pub fn parse_session_tags(params: &[(String, String)]) -> Vec<ruststack_sts_model::types::Tag> {
    let mut tags = Vec::new();
    let mut index = 1;

    loop {
        let key_param = format!("Tags.member.{index}.Key");
        let value_param = format!("Tags.member.{index}.Value");

        let key = params
            .iter()
            .find(|(k, _)| k == &key_param)
            .map(|(_, v)| v.clone());
        let value = params
            .iter()
            .find(|(k, _)| k == &value_param)
            .map(|(_, v)| v.clone());

        match (key, value) {
            (Some(k), Some(v)) => {
                tags.push(ruststack_sts_model::types::Tag { key: k, value: v });
                index += 1;
            }
            (Some(k), None) => {
                tags.push(ruststack_sts_model::types::Tag {
                    key: k,
                    value: String::new(),
                });
                index += 1;
            }
            _ => break,
        }
    }

    tags
}

/// Parse transitive tag keys from awsQuery form parameters.
///
/// Encoded as:
/// - `TransitiveTagKeys.member.1=Project`
/// - `TransitiveTagKeys.member.2=Env`
#[must_use]
pub fn parse_transitive_tag_keys(params: &[(String, String)]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut index = 1;

    loop {
        let param = format!("TransitiveTagKeys.member.{index}");
        match params.iter().find(|(k, _)| k == &param) {
            Some((_, v)) => {
                keys.push(v.clone());
                index += 1;
            }
            None => break,
        }
    }

    keys
}

/// Parse policy ARNs from awsQuery form parameters.
#[must_use]
pub fn parse_policy_arns(params: &[(String, String)]) -> Vec<String> {
    let mut arns = Vec::new();
    let mut index = 1;

    loop {
        let param = format!("PolicyArns.member.{index}.arn");
        match params.iter().find(|(k, _)| k == &param) {
            Some((_, v)) => {
                arns.push(v.clone());
                index += 1;
            }
            None => break,
        }
    }

    arns
}

/// Extract the access key ID from a SigV4 Authorization header.
///
/// Parses the Credential component: `Credential=AKID/date/region/service/aws4_request`
#[must_use]
pub fn extract_access_key_from_auth(auth_header: &str) -> Option<String> {
    let cred_start = auth_header.find("Credential=")?;
    let cred_value = &auth_header[cred_start + 11..];
    let cred_end = cred_value.find('/')?;
    Some(cred_value[..cred_end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_form_params() {
        let body = b"Action=GetCallerIdentity&Version=2011-06-15";
        let params = parse_form_params(body);
        assert_eq!(params.len(), 2);
        assert_eq!(
            params[0],
            ("Action".to_owned(), "GetCallerIdentity".to_owned())
        );
    }

    #[test]
    fn test_should_get_required_param() {
        let params = vec![("RoleArn".to_owned(), "arn:aws:iam::123:role/R".to_owned())];
        assert_eq!(
            get_required_param(&params, "RoleArn").unwrap(),
            "arn:aws:iam::123:role/R"
        );
    }

    #[test]
    fn test_should_error_on_missing_required_param() {
        let params: Vec<(String, String)> = vec![];
        let err = get_required_param(&params, "RoleArn").unwrap_err();
        assert!(err.message.contains("RoleArn"));
    }

    #[test]
    fn test_should_get_optional_param() {
        let params = vec![("DurationSeconds".to_owned(), "3600".to_owned())];
        assert_eq!(get_optional_param(&params, "DurationSeconds"), Some("3600"));
        assert_eq!(get_optional_param(&params, "Missing"), None);
    }

    #[test]
    fn test_should_parse_session_tags() {
        let params = vec![
            ("Tags.member.1.Key".to_owned(), "Project".to_owned()),
            ("Tags.member.1.Value".to_owned(), "MyProject".to_owned()),
            ("Tags.member.2.Key".to_owned(), "Env".to_owned()),
            ("Tags.member.2.Value".to_owned(), "Dev".to_owned()),
        ];
        let tags = parse_session_tags(&params);
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].key, "Project");
        assert_eq!(tags[0].value, "MyProject");
        assert_eq!(tags[1].key, "Env");
        assert_eq!(tags[1].value, "Dev");
    }

    #[test]
    fn test_should_parse_transitive_tag_keys() {
        let params = vec![
            (
                "TransitiveTagKeys.member.1".to_owned(),
                "Project".to_owned(),
            ),
            ("TransitiveTagKeys.member.2".to_owned(), "Env".to_owned()),
        ];
        let keys = parse_transitive_tag_keys(&params);
        assert_eq!(keys, vec!["Project", "Env"]);
    }

    #[test]
    fn test_should_parse_policy_arns() {
        let params = vec![(
            "PolicyArns.member.1.arn".to_owned(),
            "arn:aws:iam::123:policy/P".to_owned(),
        )];
        let arns = parse_policy_arns(&params);
        assert_eq!(arns, vec!["arn:aws:iam::123:policy/P"]);
    }

    #[test]
    fn test_should_extract_access_key_from_auth() {
        let auth = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20260319/us-east-1/sts/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=abc123";
        assert_eq!(
            extract_access_key_from_auth(auth),
            Some("AKIAIOSFODNN7EXAMPLE".to_owned())
        );
    }

    #[test]
    fn test_should_return_none_for_missing_credential() {
        assert_eq!(extract_access_key_from_auth("Bearer token123"), None);
    }

    #[test]
    fn test_should_parse_url_encoded_special_chars() {
        let body = b"Action=AssumeRole&RoleArn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2FTestRole";
        let params = parse_form_params(body);
        assert_eq!(params[1].1, "arn:aws:iam::123456789012:role/TestRole");
    }
}
