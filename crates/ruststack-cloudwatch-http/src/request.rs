//! CloudWatch `awsQuery` request parameter parsing utilities.
//!
//! CloudWatch uses `application/x-www-form-urlencoded` request bodies with
//! dot-notation for nested parameters (e.g., `MetricData.member.1.MetricName`).

use ruststack_cloudwatch_model::error::{CloudWatchError, CloudWatchErrorCode};

/// Parse a URL-encoded body into a list of key-value pairs.
#[must_use]
pub fn parse_form_params(body: &[u8]) -> Vec<(String, String)> {
    form_urlencoded::parse(body)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// Get a required parameter value.
pub fn get_required_param<'a>(
    params: &'a [(String, String)],
    key: &str,
) -> Result<&'a str, CloudWatchError> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| {
            CloudWatchError::with_message(
                CloudWatchErrorCode::MissingRequiredParameterException,
                format!("Missing required parameter: {key}"),
            )
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

/// Get an optional boolean parameter.
#[must_use]
pub fn get_optional_bool(params: &[(String, String)], key: &str) -> Option<bool> {
    get_optional_param(params, key).map(|v| v.eq_ignore_ascii_case("true"))
}

/// Get an optional i32 parameter.
pub fn get_optional_i32(
    params: &[(String, String)],
    key: &str,
) -> Result<Option<i32>, CloudWatchError> {
    match get_optional_param(params, key) {
        Some(v) => v.parse::<i32>().map(Some).map_err(|_| {
            CloudWatchError::with_message(
                CloudWatchErrorCode::InvalidParameterValueException,
                format!("Invalid integer value for {key}: {v}"),
            )
        }),
        None => Ok(None),
    }
}

/// Get an optional f64 parameter.
pub fn get_optional_f64(
    params: &[(String, String)],
    key: &str,
) -> Result<Option<f64>, CloudWatchError> {
    match get_optional_param(params, key) {
        Some(v) => v.parse::<f64>().map(Some).map_err(|_| {
            CloudWatchError::with_message(
                CloudWatchErrorCode::InvalidParameterValueException,
                format!("Invalid numeric value for {key}: {v}"),
            )
        }),
        None => Ok(None),
    }
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
    indices.sort_unstable();
    indices
}

/// Parse a list of strings: `Prefix.member.N`.
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

/// Parse `Tags.member.N.Key` / `Tags.member.N.Value` into a `Vec<(String, String)>`.
pub fn parse_tag_list(
    params: &[(String, String)],
    prefix: &str,
) -> Result<Vec<(String, String)>, CloudWatchError> {
    let mut tags = Vec::new();
    let member_prefix = format!("{prefix}.member.");
    let indices = collect_indices(params, &member_prefix);

    for idx in indices {
        let key_param = format!("{member_prefix}{idx}.Key");
        let value_param = format!("{member_prefix}{idx}.Value");

        let key = get_required_param(params, &key_param).map_err(|_| {
            CloudWatchError::with_message(
                CloudWatchErrorCode::InvalidParameterValueException,
                format!("Missing Key for {prefix}.member.{idx}"),
            )
        })?;

        let value = get_optional_param(params, &value_param).unwrap_or("");
        tags.push((key.to_owned(), value.to_owned()));
    }

    Ok(tags)
}

/// Parse a list of structs from `.member.N.Field` params.
///
/// Returns a Vec of sub-param sets, one per member index.
#[must_use]
pub fn parse_struct_list(params: &[(String, String)], prefix: &str) -> Vec<Vec<(String, String)>> {
    let member_prefix = format!("{prefix}.member.");
    let indices = collect_indices(params, &member_prefix);

    let mut result = Vec::with_capacity(indices.len());
    for idx in indices {
        let entry_prefix = format!("{member_prefix}{idx}.");
        let mut sub_params: Vec<(String, String)> = Vec::new();
        for (k, v) in params {
            if let Some(rest) = k.strip_prefix(&entry_prefix) {
                sub_params.push((rest.to_owned(), v.clone()));
            }
        }
        result.push(sub_params);
    }
    result
}

/// Parse dimensions from `Prefix.member.N.Name` / `Prefix.member.N.Value`.
#[must_use]
pub fn parse_dimensions(params: &[(String, String)], prefix: &str) -> Vec<(String, String)> {
    let member_prefix = format!("{prefix}.member.");
    let indices = collect_indices(params, &member_prefix);

    let mut dims = Vec::with_capacity(indices.len());
    for idx in indices {
        let name_param = format!("{member_prefix}{idx}.Name");
        let value_param = format!("{member_prefix}{idx}.Value");

        if let (Some(name), Some(value)) = (
            get_optional_param(params, &name_param),
            get_optional_param(params, &value_param),
        ) {
            dims.push((name.to_owned(), value.to_owned()));
        }
    }
    dims
}

/// Parse dimension filters from `Prefix.member.N.Name` / `Prefix.member.N.Value`.
///
/// Unlike regular dimensions, filter dimensions may omit the Value field.
#[must_use]
pub fn parse_dimension_filters(
    params: &[(String, String)],
    prefix: &str,
) -> Vec<(String, Option<String>)> {
    let member_prefix = format!("{prefix}.member.");
    let indices = collect_indices(params, &member_prefix);

    let mut dims = Vec::with_capacity(indices.len());
    for idx in indices {
        let name_param = format!("{member_prefix}{idx}.Name");
        let value_param = format!("{member_prefix}{idx}.Value");

        if let Some(name) = get_optional_param(params, &name_param) {
            let value = get_optional_param(params, &value_param).map(str::to_owned);
            dims.push((name.to_owned(), value));
        }
    }
    dims
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_form_params() {
        let body = b"Action=PutMetricData&Namespace=MyApp&Version=2010-08-01";
        let params = parse_form_params(body);
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], ("Action".to_owned(), "PutMetricData".to_owned()));
        assert_eq!(params[1], ("Namespace".to_owned(), "MyApp".to_owned()));
    }

    #[test]
    fn test_should_get_required_param() {
        let params = vec![("Namespace".to_owned(), "MyApp".to_owned())];
        assert_eq!(get_required_param(&params, "Namespace").unwrap(), "MyApp");
    }

    #[test]
    fn test_should_error_on_missing_required_param() {
        let params: Vec<(String, String)> = vec![];
        let err = get_required_param(&params, "Namespace").unwrap_err();
        assert!(err.message.contains("Namespace"));
    }

    #[test]
    fn test_should_parse_string_list() {
        let params = vec![
            ("Statistics.member.1".to_owned(), "Sum".to_owned()),
            ("Statistics.member.2".to_owned(), "Average".to_owned()),
            ("Statistics.member.3".to_owned(), "Maximum".to_owned()),
        ];
        let list = parse_string_list(&params, "Statistics");
        assert_eq!(list, vec!["Sum", "Average", "Maximum"]);
    }

    #[test]
    fn test_should_parse_dimensions() {
        let params = vec![
            (
                "Dimensions.member.1.Name".to_owned(),
                "Environment".to_owned(),
            ),
            (
                "Dimensions.member.1.Value".to_owned(),
                "Production".to_owned(),
            ),
            ("Dimensions.member.2.Name".to_owned(), "Service".to_owned()),
            ("Dimensions.member.2.Value".to_owned(), "API".to_owned()),
        ];
        let dims = parse_dimensions(&params, "Dimensions");
        assert_eq!(dims.len(), 2);
        assert_eq!(dims[0], ("Environment".to_owned(), "Production".to_owned()));
        assert_eq!(dims[1], ("Service".to_owned(), "API".to_owned()));
    }

    #[test]
    fn test_should_parse_tag_list() {
        let params = vec![
            ("Tags.member.1.Key".to_owned(), "Environment".to_owned()),
            ("Tags.member.1.Value".to_owned(), "Production".to_owned()),
            ("Tags.member.2.Key".to_owned(), "Team".to_owned()),
            ("Tags.member.2.Value".to_owned(), "Platform".to_owned()),
        ];
        let tags = parse_tag_list(&params, "Tags").unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0], ("Environment".to_owned(), "Production".to_owned()));
        assert_eq!(tags[1], ("Team".to_owned(), "Platform".to_owned()));
    }

    #[test]
    fn test_should_parse_struct_list() {
        let params = vec![
            (
                "MetricData.member.1.MetricName".to_owned(),
                "CPUUtilization".to_owned(),
            ),
            ("MetricData.member.1.Value".to_owned(), "80.5".to_owned()),
            (
                "MetricData.member.2.MetricName".to_owned(),
                "MemoryUsage".to_owned(),
            ),
            ("MetricData.member.2.Value".to_owned(), "65.0".to_owned()),
        ];
        let structs = parse_struct_list(&params, "MetricData");
        assert_eq!(structs.len(), 2);
        assert_eq!(structs[0].len(), 2);
        assert_eq!(structs[0][0].0, "MetricName");
        assert_eq!(structs[0][0].1, "CPUUtilization");
    }

    #[test]
    fn test_should_parse_url_encoded_special_chars() {
        let body = b"Action=PutMetricData&Namespace=AWS%2FEC2";
        let params = parse_form_params(body);
        assert_eq!(params[1].1, "AWS/EC2");
    }

    #[test]
    fn test_should_parse_optional_i32() {
        let params = vec![("Period".to_owned(), "300".to_owned())];
        assert_eq!(get_optional_i32(&params, "Period").unwrap(), Some(300));
        assert_eq!(get_optional_i32(&params, "Missing").unwrap(), None);
    }

    #[test]
    fn test_should_parse_optional_f64() {
        let params = vec![("Threshold".to_owned(), "80.5".to_owned())];
        assert_eq!(get_optional_f64(&params, "Threshold").unwrap(), Some(80.5));
    }

    #[test]
    fn test_should_parse_dimension_filters() {
        let params = vec![
            ("Dimensions.member.1.Name".to_owned(), "Env".to_owned()),
            ("Dimensions.member.1.Value".to_owned(), "Prod".to_owned()),
            ("Dimensions.member.2.Name".to_owned(), "Service".to_owned()),
        ];
        let filters = parse_dimension_filters(&params, "Dimensions");
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0], ("Env".to_owned(), Some("Prod".to_owned())));
        assert_eq!(filters[1], ("Service".to_owned(), None));
    }
}
