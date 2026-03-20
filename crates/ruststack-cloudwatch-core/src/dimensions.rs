//! Dimension normalization and matching utilities.

use ruststack_cloudwatch_model::types::Dimension;

/// Normalize a set of dimensions for consistent storage and lookup.
///
/// Dimensions are sorted by name. This ensures that
/// `[{Name: "B", Value: "2"}, {Name: "A", Value: "1"}]` and
/// `[{Name: "A", Value: "1"}, {Name: "B", Value: "2"}]`
/// produce the same `MetricKey`.
#[must_use]
pub fn normalize_dimensions(mut dimensions: Vec<Dimension>) -> Vec<Dimension> {
    dimensions.sort_by(|a, b| a.name.cmp(&b.name));
    dimensions.dedup_by(|a, b| a.name == b.name);
    dimensions
}

/// Check if a set of dimensions matches all filter dimensions.
///
/// Each filter dimension must match by name and (optionally) value.
#[must_use]
pub fn dimensions_match(dims: &[Dimension], filters: &[(String, Option<String>)]) -> bool {
    filters.iter().all(|(name, value)| {
        dims.iter()
            .any(|d| d.name == *name && value.as_ref().is_none_or(|v| d.value == *v))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim(name: &str, value: &str) -> Dimension {
        Dimension {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn test_should_normalize_dimensions_by_name() {
        let dims = vec![dim("B", "2"), dim("A", "1"), dim("C", "3")];
        let normalized = normalize_dimensions(dims);
        assert_eq!(normalized[0].name, "A");
        assert_eq!(normalized[1].name, "B");
        assert_eq!(normalized[2].name, "C");
    }

    #[test]
    fn test_should_dedup_dimensions() {
        let dims = vec![dim("A", "1"), dim("A", "2"), dim("B", "3")];
        let normalized = normalize_dimensions(dims);
        assert_eq!(normalized.len(), 2);
    }

    #[test]
    fn test_should_match_dimensions_with_name_only() {
        let dims = vec![dim("Env", "Prod"), dim("Service", "API")];
        let filters = vec![("Env".to_owned(), None)];
        assert!(dimensions_match(&dims, &filters));
    }

    #[test]
    fn test_should_match_dimensions_with_name_and_value() {
        let dims = vec![dim("Env", "Prod"), dim("Service", "API")];
        let filters = vec![("Env".to_owned(), Some("Prod".to_owned()))];
        assert!(dimensions_match(&dims, &filters));
    }

    #[test]
    fn test_should_not_match_wrong_value() {
        let dims = vec![dim("Env", "Prod")];
        let filters = vec![("Env".to_owned(), Some("Staging".to_owned()))];
        assert!(!dimensions_match(&dims, &filters));
    }
}
