//! ListSecrets filter evaluation.

use ruststack_secretsmanager_model::types::{Filter, FilterNameStringType, Tag};

use crate::storage::SecretRecord;

/// Evaluate whether a secret record matches all provided filters.
///
/// A record must match every filter in the list (AND semantics).
/// Within a single filter, the record must match at least one value (OR semantics).
#[must_use]
pub fn matches_filters(record: &SecretRecord, filters: &[Filter]) -> bool {
    filters.iter().all(|f| matches_single_filter(record, f))
}

/// Evaluate a single filter against a record.
fn matches_single_filter(record: &SecretRecord, filter: &Filter) -> bool {
    let key = filter.key.as_ref().unwrap_or(&FilterNameStringType::All);
    let values = &filter.values;

    if values.is_empty() {
        return true;
    }

    // Each filter value must be matched (AND for "name" and "description" with space-separated
    // values). For simplicity in the filter list, each value in the values list is OR.
    values.iter().any(|v| matches_filter_value(record, key, v))
}

/// Check if a record matches a single filter value for a given key.
fn matches_filter_value(record: &SecretRecord, key: &FilterNameStringType, value: &str) -> bool {
    match key {
        FilterNameStringType::Name => {
            // Prefix match, with `!` prefix for negation.
            if let Some(negated) = value.strip_prefix('!') {
                !record.name.starts_with(negated)
            } else {
                record.name.starts_with(value)
            }
        }
        FilterNameStringType::Description => {
            let desc = record.description.as_deref().unwrap_or("");
            if let Some(negated) = value.strip_prefix('!') {
                !desc.starts_with(negated)
            } else {
                desc.starts_with(value)
            }
        }
        FilterNameStringType::TagKey => record.tags.iter().any(|t| t.key.as_deref() == Some(value)),
        FilterNameStringType::TagValue => record
            .tags
            .iter()
            .any(|t| t.value.as_deref() == Some(value)),
        FilterNameStringType::OwningService => record.owning_service.as_deref() == Some(value),
        FilterNameStringType::PrimaryRegion => record.primary_region.as_deref() == Some(value),
        FilterNameStringType::All => {
            // Match across name, description, and tag values.
            let name_match = record.name.contains(value);
            let desc_match = record.description.as_deref().unwrap_or("").contains(value);
            let tag_match = record
                .tags
                .iter()
                .any(|t| t.value.as_deref().is_some_and(|tv| tv.contains(value)));
            name_match || desc_match || tag_match
        }
    }
}

/// Build a default empty tag for internal use.
#[must_use]
pub fn empty_tag() -> Tag {
    Tag {
        key: None,
        value: None,
    }
}
