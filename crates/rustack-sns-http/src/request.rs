//! SNS `awsQuery` request parameter parsing utilities.
//!
//! SNS uses `application/x-www-form-urlencoded` request bodies with
//! dot-notation for nested parameters (e.g., `Attributes.entry.1.key`).

use std::collections::HashMap;

use rustack_sns_model::{
    error::SnsError,
    types::{MessageAttributeValue, PublishBatchRequestEntry, Tag},
};

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
) -> Result<&'a str, SnsError> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| SnsError::invalid_parameter(format!("Missing required parameter: {key}")))
}

/// Get an optional parameter value.
#[must_use]
pub fn get_optional_param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
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

/// Parse `Prefix.entry.N.key` / `Prefix.entry.N.value` into a `HashMap`.
///
/// This handles the AWS `awsQuery` map serialization format where entries
/// are numbered starting at 1:
///
/// ```text
/// Attributes.entry.1.key=DisplayName
/// Attributes.entry.1.value=MyTopic
/// Attributes.entry.2.key=Policy
/// Attributes.entry.2.value=...
/// ```
pub fn parse_attributes_map(
    params: &[(String, String)],
    prefix: &str,
) -> Result<HashMap<String, String>, SnsError> {
    let mut result = HashMap::new();
    let entry_prefix = format!("{prefix}.entry.");
    let indices = collect_indices(params, &entry_prefix);

    for idx in indices {
        let key_param = format!("{entry_prefix}{idx}.key");
        let value_param = format!("{entry_prefix}{idx}.value");

        let key = get_required_param(params, &key_param).map_err(|_| {
            SnsError::invalid_parameter(format!("Missing key for {prefix}.entry.{idx}"))
        })?;

        let value = get_optional_param(params, &value_param).unwrap_or("");
        result.insert(key.to_owned(), value.to_owned());
    }

    Ok(result)
}

/// Parse `Prefix.member.N.Key` / `Prefix.member.N.Value` into a `Vec<Tag>`.
///
/// AWS `awsQuery` tag list format:
///
/// ```text
/// Tags.member.1.Key=Environment
/// Tags.member.1.Value=Production
/// Tags.member.2.Key=Project
/// Tags.member.2.Value=MyApp
/// ```
pub fn parse_tag_list(params: &[(String, String)], prefix: &str) -> Result<Vec<Tag>, SnsError> {
    let mut tags = Vec::new();
    let member_prefix = format!("{prefix}.member.");
    let indices = collect_indices(params, &member_prefix);

    for idx in indices {
        let key_param = format!("{member_prefix}{idx}.Key");
        let value_param = format!("{member_prefix}{idx}.Value");

        let key = get_required_param(params, &key_param).map_err(|_| {
            SnsError::invalid_parameter(format!("Missing Key for {prefix}.member.{idx}"))
        })?;

        let value = get_optional_param(params, &value_param).unwrap_or("");
        tags.push(Tag {
            key: key.to_owned(),
            value: value.to_owned(),
        });
    }

    Ok(tags)
}

/// Parse `MessageAttributes` from form params.
///
/// AWS `awsQuery` message attribute format:
///
/// ```text
/// MessageAttributes.entry.1.Name=AttributeName
/// MessageAttributes.entry.1.Value.DataType=String
/// MessageAttributes.entry.1.Value.StringValue=hello
/// ```
pub fn parse_message_attributes(
    params: &[(String, String)],
    prefix: &str,
) -> Result<HashMap<String, MessageAttributeValue>, SnsError> {
    let mut result = HashMap::new();
    let entry_prefix = format!("{prefix}.entry.");
    let indices = collect_indices(params, &entry_prefix);

    for idx in indices {
        let name_param = format!("{entry_prefix}{idx}.Name");
        let data_type_param = format!("{entry_prefix}{idx}.Value.DataType");
        let string_value_param = format!("{entry_prefix}{idx}.Value.StringValue");
        let binary_value_param = format!("{entry_prefix}{idx}.Value.BinaryValue");

        let name = get_required_param(params, &name_param).map_err(|_| {
            SnsError::invalid_parameter(format!("Missing Name for {prefix}.entry.{idx}"))
        })?;

        let data_type = get_required_param(params, &data_type_param).map_err(|_| {
            SnsError::invalid_parameter(format!("Missing DataType for {prefix}.entry.{idx}"))
        })?;

        let string_value = get_optional_param(params, &string_value_param).map(str::to_owned);
        let binary_value = get_optional_param(params, &binary_value_param).map(str::to_owned);

        result.insert(
            name.to_owned(),
            MessageAttributeValue {
                data_type: data_type.to_owned(),
                string_value,
                binary_value,
            },
        );
    }

    Ok(result)
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

/// Parse `PublishBatch` entries from form params.
///
/// Format:
/// ```text
/// PublishBatchRequestEntries.member.1.Id=entry1
/// PublishBatchRequestEntries.member.1.Message=hello
/// PublishBatchRequestEntries.member.1.Subject=test
/// PublishBatchRequestEntries.member.1.MessageAttributes.entry.1.Name=attr1
/// PublishBatchRequestEntries.member.1.MessageAttributes.entry.1.Value.DataType=String
/// PublishBatchRequestEntries.member.1.MessageAttributes.entry.1.Value.StringValue=val1
/// ```
pub fn parse_publish_batch_entries(
    params: &[(String, String)],
) -> Result<Vec<PublishBatchRequestEntry>, SnsError> {
    let member_prefix = "PublishBatchRequestEntries.member.";
    let mut indices = collect_indices(params, member_prefix);
    indices.sort_unstable();

    let mut entries = Vec::with_capacity(indices.len());

    for idx in indices {
        let entry_prefix = format!("{member_prefix}{idx}");

        let id = get_required_param(params, &format!("{entry_prefix}.Id")).map_err(|_| {
            SnsError::invalid_parameter(format!(
                "Missing Id for PublishBatchRequestEntries.member.{idx}"
            ))
        })?;

        let message =
            get_required_param(params, &format!("{entry_prefix}.Message")).map_err(|_| {
                SnsError::invalid_parameter(format!(
                    "Missing Message for PublishBatchRequestEntries.member.{idx}"
                ))
            })?;

        let subject =
            get_optional_param(params, &format!("{entry_prefix}.Subject")).map(str::to_owned);
        let message_structure =
            get_optional_param(params, &format!("{entry_prefix}.MessageStructure"))
                .map(str::to_owned);
        let message_group_id =
            get_optional_param(params, &format!("{entry_prefix}.MessageGroupId"))
                .map(str::to_owned);
        let message_deduplication_id =
            get_optional_param(params, &format!("{entry_prefix}.MessageDeduplicationId"))
                .map(str::to_owned);

        let attrs_prefix = format!("{entry_prefix}.MessageAttributes");
        let message_attributes = parse_message_attributes(params, &attrs_prefix)?;

        entries.push(PublishBatchRequestEntry {
            id: id.to_owned(),
            message: message.to_owned(),
            subject,
            message_structure,
            message_attributes,
            message_group_id,
            message_deduplication_id,
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_form_params() {
        let body = b"Action=CreateTopic&Name=MyTopic&Version=2010-03-31";
        let params = parse_form_params(body);
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], ("Action".to_owned(), "CreateTopic".to_owned()));
        assert_eq!(params[1], ("Name".to_owned(), "MyTopic".to_owned()));
    }

    #[test]
    fn test_should_get_required_param() {
        let params = vec![("Name".to_owned(), "MyTopic".to_owned())];
        assert_eq!(get_required_param(&params, "Name").unwrap(), "MyTopic");
    }

    #[test]
    fn test_should_error_on_missing_required_param() {
        let params: Vec<(String, String)> = vec![];
        let err = get_required_param(&params, "Name").unwrap_err();
        assert!(err.message.contains("Name"));
    }

    #[test]
    fn test_should_get_optional_param() {
        let params = vec![("Name".to_owned(), "MyTopic".to_owned())];
        assert_eq!(get_optional_param(&params, "Name"), Some("MyTopic"));
        assert_eq!(get_optional_param(&params, "Missing"), None);
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
    fn test_should_parse_attributes_map() {
        let params = vec![
            (
                "Attributes.entry.1.key".to_owned(),
                "DisplayName".to_owned(),
            ),
            ("Attributes.entry.1.value".to_owned(), "MyTopic".to_owned()),
            ("Attributes.entry.2.key".to_owned(), "Policy".to_owned()),
            (
                "Attributes.entry.2.value".to_owned(),
                "{\"Version\":\"2012-10-17\"}".to_owned(),
            ),
        ];
        let map = parse_attributes_map(&params, "Attributes").unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("DisplayName").unwrap(), "MyTopic");
        assert_eq!(map.get("Policy").unwrap(), "{\"Version\":\"2012-10-17\"}");
    }

    #[test]
    fn test_should_parse_empty_attributes_map() {
        let params: Vec<(String, String)> = vec![];
        let map = parse_attributes_map(&params, "Attributes").unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn test_should_parse_tag_list() {
        let params = vec![
            ("Tags.member.1.Key".to_owned(), "Environment".to_owned()),
            ("Tags.member.1.Value".to_owned(), "Production".to_owned()),
            ("Tags.member.2.Key".to_owned(), "Project".to_owned()),
            ("Tags.member.2.Value".to_owned(), "MyApp".to_owned()),
        ];
        let tags = parse_tag_list(&params, "Tags").unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].key, "Environment");
        assert_eq!(tags[0].value, "Production");
        assert_eq!(tags[1].key, "Project");
        assert_eq!(tags[1].value, "MyApp");
    }

    #[test]
    fn test_should_parse_tag_list_with_empty_value() {
        let params = vec![("Tags.member.1.Key".to_owned(), "EmptyTag".to_owned())];
        let tags = parse_tag_list(&params, "Tags").unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].key, "EmptyTag");
        assert_eq!(tags[0].value, "");
    }

    #[test]
    fn test_should_parse_message_attributes() {
        let params = vec![
            (
                "MessageAttributes.entry.1.Name".to_owned(),
                "attr1".to_owned(),
            ),
            (
                "MessageAttributes.entry.1.Value.DataType".to_owned(),
                "String".to_owned(),
            ),
            (
                "MessageAttributes.entry.1.Value.StringValue".to_owned(),
                "hello".to_owned(),
            ),
            (
                "MessageAttributes.entry.2.Name".to_owned(),
                "attr2".to_owned(),
            ),
            (
                "MessageAttributes.entry.2.Value.DataType".to_owned(),
                "Number".to_owned(),
            ),
            (
                "MessageAttributes.entry.2.Value.StringValue".to_owned(),
                "42".to_owned(),
            ),
        ];
        let parsed = parse_message_attributes(&params, "MessageAttributes").unwrap();
        assert_eq!(parsed.len(), 2);

        let first = parsed.get("attr1").unwrap();
        assert_eq!(first.data_type, "String");
        assert_eq!(first.string_value.as_deref(), Some("hello"));
        assert!(first.binary_value.is_none());

        let second = parsed.get("attr2").unwrap();
        assert_eq!(second.data_type, "Number");
        assert_eq!(second.string_value.as_deref(), Some("42"));
    }

    #[test]
    fn test_should_parse_message_attributes_with_binary() {
        let params = vec![
            ("MA.entry.1.Name".to_owned(), "binattr".to_owned()),
            ("MA.entry.1.Value.DataType".to_owned(), "Binary".to_owned()),
            (
                "MA.entry.1.Value.BinaryValue".to_owned(),
                "dGVzdA==".to_owned(),
            ),
        ];
        let attrs = parse_message_attributes(&params, "MA").unwrap();
        assert_eq!(attrs.len(), 1);
        let attr = attrs.get("binattr").unwrap();
        assert_eq!(attr.data_type, "Binary");
        assert!(attr.string_value.is_none());
        assert_eq!(attr.binary_value.as_deref(), Some("dGVzdA=="));
    }

    #[test]
    fn test_should_parse_string_list() {
        let params = vec![
            ("Actions.member.1".to_owned(), "Publish".to_owned()),
            ("Actions.member.2".to_owned(), "Subscribe".to_owned()),
            ("Actions.member.3".to_owned(), "Unsubscribe".to_owned()),
        ];
        let list = parse_string_list(&params, "Actions");
        assert_eq!(list, vec!["Publish", "Subscribe", "Unsubscribe"]);
    }

    #[test]
    fn test_should_parse_empty_string_list() {
        let params: Vec<(String, String)> = vec![];
        let list = parse_string_list(&params, "Actions");
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_should_parse_publish_batch_entries() {
        let params = vec![
            (
                "PublishBatchRequestEntries.member.1.Id".to_owned(),
                "entry1".to_owned(),
            ),
            (
                "PublishBatchRequestEntries.member.1.Message".to_owned(),
                "hello".to_owned(),
            ),
            (
                "PublishBatchRequestEntries.member.1.Subject".to_owned(),
                "test subject".to_owned(),
            ),
            (
                "PublishBatchRequestEntries.member.2.Id".to_owned(),
                "entry2".to_owned(),
            ),
            (
                "PublishBatchRequestEntries.member.2.Message".to_owned(),
                "world".to_owned(),
            ),
        ];
        let entries = parse_publish_batch_entries(&params).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "entry1");
        assert_eq!(entries[0].message, "hello");
        assert_eq!(entries[0].subject.as_deref(), Some("test subject"));
        assert_eq!(entries[1].id, "entry2");
        assert_eq!(entries[1].message, "world");
        assert!(entries[1].subject.is_none());
    }

    #[test]
    fn test_should_parse_publish_batch_entries_with_attributes() {
        let params = vec![
            (
                "PublishBatchRequestEntries.member.1.Id".to_owned(),
                "e1".to_owned(),
            ),
            (
                "PublishBatchRequestEntries.member.1.Message".to_owned(),
                "msg1".to_owned(),
            ),
            (
                "PublishBatchRequestEntries.member.1.MessageAttributes.entry.1.Name".to_owned(),
                "key1".to_owned(),
            ),
            (
                "PublishBatchRequestEntries.member.1.MessageAttributes.entry.1.Value.DataType"
                    .to_owned(),
                "String".to_owned(),
            ),
            (
                "PublishBatchRequestEntries.member.1.MessageAttributes.entry.1.Value.StringValue"
                    .to_owned(),
                "val1".to_owned(),
            ),
        ];
        let entries = parse_publish_batch_entries(&params).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_attributes.len(), 1);
        let attr = entries[0].message_attributes.get("key1").unwrap();
        assert_eq!(attr.data_type, "String");
        assert_eq!(attr.string_value.as_deref(), Some("val1"));
    }

    #[test]
    fn test_should_parse_url_encoded_special_chars() {
        let body = b"Action=Publish&Message=hello+world&TopicArn=arn%3Aaws%3Asns%3Aus-east-1%3A123456789012%3AMyTopic";
        let params = parse_form_params(body);
        assert_eq!(params.len(), 3);
        assert_eq!(params[1].1, "hello world");
        assert_eq!(params[2].1, "arn:aws:sns:us-east-1:123456789012:MyTopic");
    }
}
